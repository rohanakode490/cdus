use cdus_common::ClipboardEvent;
use rusqlite::{Connection, OptionalExtension, Result};
use std::path::Path;
use parking_lot::Mutex;
use tracing::info;

pub struct Store {
    pub events_conn: Mutex<Connection>,
    pub state_conn: Mutex<Connection>,
    pub search_conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct TransferRecord {
    pub transfer_id: String,
    pub direction: String,
    pub peer_node_id: String,
    pub file_path: String,
    pub file_name: String,
    pub total_bytes: i64,
    pub bytes_confirmed: i64,
    pub chunk_size: i64,
    pub file_hash: String,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct PairedDeviceRecord {
    pub node_id: String,
    pub label: String,
    pub last_known_ips: Option<Vec<String>>,
    pub last_known_port: Option<u16>,
    pub static_key: Option<Vec<u8>>,
}

impl Store {
    pub fn init(data_dir: &Path) -> Result<Self> {
        let events_path = data_dir.join("events.db");
        let state_path = data_dir.join("state.db");
        let search_path = data_dir.join("search_index.db");

        info!("Initializing databases at {}...", data_dir.display());

        let events_conn = Connection::open(events_path)?;
        let state_conn = Connection::open(state_path)?;
        let search_conn = Connection::open(search_path)?;

        // Enable WAL mode
        events_conn.pragma_update(None, "journal_mode", "WAL")?;
        state_conn.pragma_update(None, "journal_mode", "WAL")?;
        search_conn.pragma_update(None, "journal_mode", "WAL")?;

        // Initialize tables
        events_conn.execute(
            "CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                payload BLOB NOT NULL,
                source TEXT NOT NULL,
                hash TEXT NOT NULL,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                local_only INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;

        // Migration: Add 'source' column if it doesn't exist (for existing databases)
        let has_source: bool = events_conn
            .query_row(
                "SELECT 1 FROM pragma_table_info('events') WHERE name='source'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !has_source {
            info!("Migrating events table: adding 'source' column...");
            events_conn.execute(
                "ALTER TABLE events ADD COLUMN source TEXT NOT NULL DEFAULT 'Unknown'",
                [],
            )?;
        }

        // Migration: Add 'local_only' column if it doesn't exist
        let has_local_only: bool = events_conn
            .query_row(
                "SELECT 1 FROM pragma_table_info('events') WHERE name='local_only'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !has_local_only {
            info!("Migrating events table: adding 'local_only' column...");
            events_conn.execute(
                "ALTER TABLE events ADD COLUMN local_only INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }

        // State table for simple key-value settings
        state_conn.execute(
            "CREATE TABLE IF NOT EXISTS state (
                key TEXT PRIMARY KEY,
                value TEXT
            )",
            [],
        )?;

        // Paired devices table
        state_conn.execute(
            "CREATE TABLE IF NOT EXISTS paired_devices (
                node_id TEXT PRIMARY KEY,
                label TEXT,
                last_known_ips TEXT,
                last_known_port INTEGER,
                static_key BLOB
            )",
            [],
        )?;

        // Migration: Add network info to paired_devices if it doesn't exist
        let has_network_info: bool = state_conn
            .query_row(
                "SELECT 1 FROM pragma_table_info('paired_devices') WHERE name='last_known_ips'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !has_network_info {
            info!("Migrating paired_devices table: adding network info columns...");
            let _ = state_conn.execute(
                "ALTER TABLE paired_devices ADD COLUMN last_known_ips TEXT",
                [],
            );
            let _ = state_conn.execute(
                "ALTER TABLE paired_devices ADD COLUMN last_known_port INTEGER",
                [],
            );
        }

        // Migration: Add static_key to paired_devices if it doesn't exist
        let has_static_key: bool = state_conn
            .query_row(
                "SELECT 1 FROM pragma_table_info('paired_devices') WHERE name='static_key'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !has_static_key {
            info!("Migrating paired_devices table: adding 'static_key' column...");
            let _ = state_conn.execute(
                "ALTER TABLE paired_devices ADD COLUMN static_key BLOB",
                [],
            );
        }

        // Phase 1.1: File Transfers tables in state.db
        state_conn.execute(
            "CREATE TABLE IF NOT EXISTS file_transfers (
                transfer_id     TEXT PRIMARY KEY,         -- UUID v4
                direction       TEXT NOT NULL,            -- 'outgoing' | 'incoming'
                peer_node_id    TEXT NOT NULL,            -- who we're transferring with
                file_path       TEXT NOT NULL,            -- absolute path on THIS device
                file_name       TEXT NOT NULL,            -- original filename (display only)
                total_bytes     INTEGER NOT NULL,
                bytes_confirmed INTEGER NOT NULL DEFAULT 0, -- last ACKed offset
                chunk_size      INTEGER NOT NULL DEFAULT 262144, -- 256KB default
                file_hash       TEXT NOT NULL,            -- BLAKE3 hex of whole file
                status          TEXT NOT NULL DEFAULT 'pending',
                -- 'pending' | 'awaiting_acceptance' | 'in_progress' | 'paused' | 'complete' | 'failed' | 'declined'
                error_message   TEXT,                     -- populated on 'failed'
                created_at      INTEGER NOT NULL,         -- unix timestamp ms
                updated_at      INTEGER NOT NULL
            )",
            [],
        )?;

        state_conn.execute(
            "CREATE TABLE IF NOT EXISTS file_chunks (
                transfer_id     TEXT NOT NULL REFERENCES file_transfers(transfer_id),
                chunk_index     INTEGER NOT NULL,
                chunk_hash      TEXT NOT NULL,            -- BLAKE3 hex of this chunk's plaintext
                byte_offset     INTEGER NOT NULL,         -- start byte of this chunk in file
                byte_length     INTEGER NOT NULL,
                verified        INTEGER NOT NULL DEFAULT 0, -- 0 = not verified, 1 = hash verified
                PRIMARY KEY (transfer_id, chunk_index)
            )",
            [],
        )?;

        state_conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_transfers_status ON file_transfers(status)",
            [],
        )?;
        state_conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_transfers_peer ON file_transfers(peer_node_id)",
            [],
        )?;

        state_conn.execute(
            "CREATE TABLE IF NOT EXISTS audit_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp INTEGER NOT NULL
            )",
            [],
        )?;

        search_conn.execute(
            "CREATE TABLE IF NOT EXISTS search_index (
                id TEXT PRIMARY KEY,
                item_type TEXT NOT NULL,
                title TEXT NOT NULL,
                subtitle TEXT NOT NULL,
                content TEXT,
                timestamp INTEGER NOT NULL
            )",
            [],
        )?;

        search_conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_search_type ON search_index(item_type)",
            [],
        )?;

        search_conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_search_timestamp ON search_index(timestamp)",
            [],
        )?;

        // Populate search index if empty
        let is_empty: bool = search_conn
            .query_row(
                "SELECT count(*) FROM search_index",
                [],
                |row| {
                    let count: i64 = row.get(0)?;
                    Ok(count == 0)
                },
            )
            .unwrap_or(true);

        if is_empty {
            info!("Search index is empty, performing initial population...");
            // Re-index devices
            let mut stmt = state_conn.prepare("SELECT node_id, label FROM paired_devices WHERE node_id != 'unknown'")?;
            let dev_rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for dev in dev_rows {
                if let Ok((node_id, label)) = dev {
                    let title = label.to_string();
                    let subtitle = format!("Device ID: {} • Paired", node_id);
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as i64;
                    let content = format!("{} {} device", label, node_id);
                    let _ = search_conn.execute(
                        "INSERT OR REPLACE INTO search_index (id, item_type, title, subtitle, content, timestamp) VALUES (?1, 'device', ?2, ?3, ?4, ?5)",
                        (node_id, &title, &subtitle, &content, now),
                    );
                }
            }

            // Re-index clipboard events
            let mut stmt = events_conn.prepare("SELECT hash, payload, source FROM events")?;
            let event_rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, String>(2)?))
            })?;
            for ev in event_rows {
                if let Ok((hash, payload, source)) = ev {
                    let text = String::from_utf8_lossy(&payload).to_string();
                    let (title, is_url, url_to_parse) = Self::parse_clipboard_payload(&text);
                    if title.is_empty() {
                        continue;
                    }
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as i64;
                    let subtitle = if let Some(ref u) = url_to_parse {
                        let mut truncated_url = u.chars().take(60).collect::<String>();
                        if u.chars().count() > 60 {
                            truncated_url.push_str("...");
                        }
                        format!("{} • synced from {}", truncated_url, source)
                    } else {
                        format!("synced from {} • clipboard history", source)
                    };
                    let mut content = format!("{} {} {}", title, source, if is_url { "url" } else { "text" });
                    if let Some(ref u) = url_to_parse {
                        if let Ok(url) = url::Url::parse(u) {
                            if let Some(host) = url.host_str() {
                                content = format!("{} {} {} {} {}", title, u, host, source, "url");
                            }
                        }
                    }
                    let _ = search_conn.execute(
                        "INSERT OR REPLACE INTO search_index (id, item_type, title, subtitle, content, timestamp) VALUES (?1, 'clipboard', ?2, ?3, ?4, ?5)",
                        (&hash, &title, &subtitle, &content, now),
                    );
                }
            }

            // Re-index file transfers
            let mut stmt = state_conn.prepare("SELECT transfer_id, direction, peer_node_id, file_name, total_bytes, status FROM file_transfers")?;
            let trans_rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)? as u64,
                    row.get::<_, String>(5)?,
                ))
            })?;
            for trans in trans_rows {
                if let Ok((transfer_id, direction, peer_node_id, file_name, total_bytes, status)) = trans {
                    let peer_label: String = state_conn
                        .query_row(
                            "SELECT label FROM paired_devices WHERE node_id = ?",
                            [&peer_node_id],
                            |row| row.get(0),
                        )
                        .unwrap_or_else(|_| {
                            peer_node_id[..std::cmp::min(8, peer_node_id.len())].to_string()
                        });
                    let size_str = {
                        const KB: u64 = 1024;
                        const MB: u64 = 1024 * 1024;
                        const GB: u64 = 1024 * 1024 * 1024;
                        if total_bytes >= GB {
                            format!("{:.2} GB", total_bytes as f64 / GB as f64)
                        } else if total_bytes >= MB {
                            format!("{:.2} MB", total_bytes as f64 / MB as f64)
                        } else if total_bytes >= KB {
                            format!("{:.2} KB", total_bytes as f64 / KB as f64)
                        } else {
                            format!("{} B", total_bytes)
                        }
                    };
                    let title = file_name.to_string();
                    let subtitle = if direction == "outgoing" {
                        format!("Size: {} • sent to {} ({})", size_str, peer_label, status)
                    } else {
                        format!("Size: {} • received from {} ({})", size_str, peer_label, status)
                    };
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as i64;
                    let content = format!("{} {} {} file", file_name, peer_label, direction);
                    let _ = search_conn.execute(
                        "INSERT OR REPLACE INTO search_index (id, item_type, title, subtitle, content, timestamp) VALUES (?1, 'file', ?2, ?3, ?4, ?5)",
                        (&transfer_id, &title, &subtitle, &content, now),
                    );
                }
            }
        }

        Ok(Store {
            events_conn: Mutex::new(events_conn),
            state_conn: Mutex::new(state_conn),
            search_conn: Mutex::new(search_conn),
        })
    }

    // --- File Transfer Methods ---

    pub fn create_transfer(
        &self,
        transfer_id: &str,
        direction: &str,
        peer_node_id: &str,
        file_path: &str,
        file_name: &str,
        total_bytes: u64,
        chunk_size: u32,
        file_hash: &str,
    ) -> Result<()> {
        let conn = self.state_conn.lock();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        conn.execute(
            "INSERT OR REPLACE INTO file_transfers (
                transfer_id, direction, peer_node_id, file_path, file_name,
                total_bytes, chunk_size, file_hash, status, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9, ?9)",
            (
                transfer_id,
                direction,
                peer_node_id,
                file_path,
                file_name,
                total_bytes as i64,
                chunk_size as i64,
                file_hash,
                now,
            ),
        )?;

        // Append audit log for transfer initiation
        let peer_label: String = conn
            .query_row(
                "SELECT label FROM paired_devices WHERE node_id = ?",
                [peer_node_id],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| {
                peer_node_id[..std::cmp::min(8, peer_node_id.len())].to_string()
            });

        let size_str = {
            const KB: u64 = 1024;
            const MB: u64 = 1024 * 1024;
            const GB: u64 = 1024 * 1024 * 1024;
            if total_bytes >= GB {
                format!("{:.2} GB", total_bytes as f64 / GB as f64)
            } else if total_bytes >= MB {
                format!("{:.2} MB", total_bytes as f64 / MB as f64)
            } else if total_bytes >= KB {
                format!("{:.2} KB", total_bytes as f64 / KB as f64)
            } else {
                format!("{} B", total_bytes)
            }
        };

        let log_content = if direction == "outgoing" {
            format!(
                "Outgoing file transfer initiated: {} ({}) to {}",
                file_name, size_str, peer_label
            )
        } else {
            format!(
                "Incoming file transfer request: {} ({}) from {}",
                file_name, size_str, peer_label
            )
        };

        conn.execute(
            "INSERT INTO audit_logs (event_type, content, timestamp) VALUES (?1, ?2, ?3)",
            ("sync", &log_content, now),
        )?;

        // Index the file transfer
        let _ = self.index_file_transfer(transfer_id, direction, &peer_label, file_name, total_bytes, "pending");

        Ok(())
    }

    pub fn delete_transfer(&self, transfer_id: &str) -> Result<()> {
        let conn = self.state_conn.lock();
        let chunks_deleted = conn.execute("DELETE FROM file_chunks WHERE transfer_id = ?1", [transfer_id])?;
        let transfers_deleted = conn.execute("DELETE FROM file_transfers WHERE transfer_id = ?1", [transfer_id])?;
        info!("delete_transfer: ID='{}' -> chunks_deleted={}, transfers_deleted={}", transfer_id, chunks_deleted, transfers_deleted);

        // Remove from search index
        let search_conn = self.search_conn.lock();
        let _ = search_conn.execute("DELETE FROM search_index WHERE id = ?", [transfer_id]);

        Ok(())
    }

    pub fn update_transfer_status(&self, transfer_id: &str, status: &str) -> Result<()> {
        let conn = self.state_conn.lock();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        // Query transfer details before update to log status change
        let transfer_info = conn
            .query_row(
                "SELECT direction, peer_node_id, file_name, total_bytes, status FROM file_transfers WHERE transfer_id = ?",
                [transfer_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)? as u64,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;

        conn.execute(
            "UPDATE file_transfers SET status = ?1, updated_at = ?2 WHERE transfer_id = ?3",
            (status, now, transfer_id),
        )?;

        if let Some((direction, peer_node_id, file_name, total_bytes, old_status)) = transfer_info {
            let peer_label: String = conn
                .query_row(
                    "SELECT label FROM paired_devices WHERE node_id = ?",
                    [&peer_node_id],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| {
                    peer_node_id[..std::cmp::min(8, peer_node_id.len())].to_string()
                });

            if old_status != status {
                let size_str = {
                    const KB: u64 = 1024;
                    const MB: u64 = 1024 * 1024;
                    const GB: u64 = 1024 * 1024 * 1024;
                    if total_bytes >= GB {
                        format!("{:.2} GB", total_bytes as f64 / GB as f64)
                    } else if total_bytes >= MB {
                        format!("{:.2} MB", total_bytes as f64 / MB as f64)
                    } else if total_bytes >= KB {
                        format!("{:.2} KB", total_bytes as f64 / KB as f64)
                    } else {
                        format!("{} B", total_bytes)
                    }
                };

                let log_content = match status {
                    "in_progress" => Some(if direction == "outgoing" {
                        format!(
                            "Outgoing file transfer started: {} ({}) to {}",
                            file_name, size_str, peer_label
                        )
                    } else {
                        format!(
                            "Incoming file transfer started: {} ({}) from {}",
                            file_name, size_str, peer_label
                        )
                    }),
                    "complete" => Some(if direction == "outgoing" {
                        format!(
                            "Outgoing file transfer completed: {} ({}) to {}",
                            file_name, size_str, peer_label
                        )
                    } else {
                        format!(
                            "Incoming file transfer completed: {} ({}) from {}",
                            file_name, size_str, peer_label
                        )
                    }),
                    "declined" => Some(if direction == "outgoing" {
                        format!(
                            "Outgoing file transfer declined: {} ({}) by {}",
                            file_name, size_str, peer_label
                        )
                    } else {
                        format!(
                            "Incoming file transfer request declined: {} ({}) from {}",
                            file_name, size_str, peer_label
                        )
                    }),
                    "failed" => Some(if direction == "outgoing" {
                        format!(
                            "Outgoing file transfer failed: {} ({}) to {}",
                            file_name, size_str, peer_label
                        )
                    } else {
                        format!(
                            "Incoming file transfer failed: {} ({}) from {}",
                            file_name, size_str, peer_label
                        )
                    }),
                    "paused" => Some(format!(
                        "File transfer paused: {} ({}) with {}",
                        file_name, size_str, peer_label
                    )),
                    "awaiting_acceptance" => Some(format!(
                        "File transfer awaiting acceptance: {} ({}) with {}",
                        file_name, size_str, peer_label
                    )),
                    _ => None,
                };

                if let Some(content) = log_content {
                    conn.execute(
                        "INSERT INTO audit_logs (event_type, content, timestamp) VALUES (?1, ?2, ?3)",
                        ("sync", &content, now),
                    )?;
                }
            }
            let _ = self.index_file_transfer(transfer_id, &direction, &peer_label, &file_name, total_bytes, status);
        }

        Ok(())
    }

    pub fn update_transfer_status_error(&self, transfer_id: &str, error: &str) -> Result<()> {
        let conn = self.state_conn.lock();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        // Query transfer details before update to log status change
        let transfer_info = conn
            .query_row(
                "SELECT direction, peer_node_id, file_name, total_bytes, status FROM file_transfers WHERE transfer_id = ?",
                [transfer_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)? as u64,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;

        conn.execute(
            "UPDATE file_transfers SET status = 'failed', error_message = ?1, updated_at = ?2 WHERE transfer_id = ?3",
            (error, now, transfer_id),
        )?;

        if let Some((direction, peer_node_id, file_name, total_bytes, old_status)) = transfer_info {
            let peer_label: String = conn
                .query_row(
                    "SELECT label FROM paired_devices WHERE node_id = ?",
                    [&peer_node_id],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| {
                    peer_node_id[..std::cmp::min(8, peer_node_id.len())].to_string()
                });

            if old_status != "failed" {
                let size_str = {
                    const KB: u64 = 1024;
                    const MB: u64 = 1024 * 1024;
                    const GB: u64 = 1024 * 1024 * 1024;
                    if total_bytes >= GB {
                        format!("{:.2} GB", total_bytes as f64 / GB as f64)
                    } else if total_bytes >= MB {
                        format!("{:.2} MB", total_bytes as f64 / MB as f64)
                    } else if total_bytes >= KB {
                        format!("{:.2} KB", total_bytes as f64 / KB as f64)
                    } else {
                        format!("{} B", total_bytes)
                    }
                };

                let log_content = if direction == "outgoing" {
                    format!(
                        "Outgoing file transfer failed: {} ({}) to {} (Error: {})",
                        file_name, size_str, peer_label, error
                    )
                } else {
                    format!(
                        "Incoming file transfer failed: {} ({}) from {} (Error: {})",
                        file_name, size_str, peer_label, error
                    )
                };

                conn.execute(
                    "INSERT INTO audit_logs (event_type, content, timestamp) VALUES (?1, ?2, ?3)",
                    ("sync", &log_content, now),
                )?;
            }
            let _ = self.index_file_transfer(transfer_id, &direction, &peer_label, &file_name, total_bytes, "failed");
        }

        Ok(())
    }

    pub fn update_bytes_confirmed(&self, transfer_id: &str, bytes: u64) -> Result<()> {
        let conn = self.state_conn.lock();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        conn.execute(
            "UPDATE file_transfers SET bytes_confirmed = ?1, updated_at = ?2 WHERE transfer_id = ?3",
            (bytes as i64, now, transfer_id),
        )?;
        Ok(())
    }

    pub fn insert_chunk(
        &self,
        transfer_id: &str,
        index: u32,
        hash: &str,
        offset: u64,
        length: u32,
    ) -> Result<()> {
        let conn = self.state_conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO file_chunks (transfer_id, chunk_index, chunk_hash, byte_offset, byte_length)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (transfer_id, index, hash, offset as i64, length as i64),
        )?;
        Ok(())
    }

    pub fn insert_chunks_batch(
        &self,
        transfer_id: &str,
        chunks: &[(u32, String, u64, u32)],
    ) -> Result<()> {
        let mut conn = self.state_conn.lock();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO file_chunks (transfer_id, chunk_index, chunk_hash, byte_offset, byte_length)
                 VALUES (?1, ?2, ?3, ?4, ?5)"
            )?;
            for (index, hash, offset, length) in chunks {
                stmt.execute((transfer_id, index, hash, *offset as i64, *length as i64))?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn mark_chunk_verified(&self, transfer_id: &str, index: u32) -> Result<()> {
        let conn = self.state_conn.lock();
        conn.execute(
            "UPDATE file_chunks SET verified = 1 WHERE transfer_id = ?1 AND chunk_index = ?2",
            (transfer_id, index),
        )?;
        Ok(())
    }

    pub fn get_incomplete_chunks(&self, transfer_id: &str) -> Result<Vec<u32>> {
        let conn = self.state_conn.lock();
        let mut stmt = conn.prepare(
            "SELECT chunk_index FROM file_chunks WHERE transfer_id = ?1 AND verified = 0 ORDER BY chunk_index ASC",
        )?;
        let rows = stmt.query_map([transfer_id], |row| row.get(0))?;
        let mut result = Vec::new();
        for r in rows {
            result.push(r?);
        }
        Ok(result)
    }

    pub fn get_transfer(&self, transfer_id: &str) -> Result<Option<TransferRecord>> {
        let conn = self.state_conn.lock();
        conn.query_row(
            "SELECT transfer_id, direction, peer_node_id, file_path, file_name,
                    total_bytes, bytes_confirmed, chunk_size, file_hash, status,
                    error_message, created_at, updated_at
             FROM file_transfers WHERE transfer_id = ?",
            [transfer_id],
            |row| {
                Ok(TransferRecord {
                    transfer_id: row.get(0)?,
                    direction: row.get(1)?,
                    peer_node_id: row.get(2)?,
                    file_path: row.get(3)?,
                    file_name: row.get(4)?,
                    total_bytes: row.get(5)?,
                    bytes_confirmed: row.get(6)?,
                    chunk_size: row.get(7)?,
                    file_hash: row.get(8)?,
                    status: row.get(9)?,
                    error_message: row.get(10)?,
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                })
            },
        )
        .optional()
    }

    pub fn get_transfers_older_than(&self, cutoff_ms: i64, statuses: &[&str]) -> Result<Vec<TransferRecord>> {
        let conn = self.state_conn.lock();

        let status_placeholders: String = statuses.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let query = format!(
            "SELECT transfer_id, direction, peer_node_id, file_path, file_name,
                    total_bytes, bytes_confirmed, chunk_size, file_hash, status,
                    error_message, created_at, updated_at
             FROM file_transfers WHERE created_at < ?1 AND status IN ({})",
            status_placeholders
        );

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        params.push(Box::new(cutoff_ms));
        for status in statuses {
            params.push(Box::new(status.to_string()));
        }

        let mut stmt = conn.prepare(&query)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
            Ok(TransferRecord {
                transfer_id: row.get(0)?,
                direction: row.get(1)?,
                peer_node_id: row.get(2)?,
                file_path: row.get(3)?,
                file_name: row.get(4)?,
                total_bytes: row.get(5)?,
                bytes_confirmed: row.get(6)?,
                chunk_size: row.get(7)?,
                file_hash: row.get(8)?,
                status: row.get(9)?,
                error_message: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn get_transfer_history(&self, limit: u32) -> Result<Vec<TransferRecord>> {
        let conn = self.state_conn.lock();
        let mut stmt = conn.prepare(
            "SELECT transfer_id, direction, peer_node_id, file_path, file_name,
                    total_bytes, bytes_confirmed, chunk_size, file_hash, status,
                    error_message, created_at, updated_at
             FROM file_transfers ORDER BY created_at DESC LIMIT ?",
        )?;

        let rows = stmt.query_map([limit], |row| {
            Ok(TransferRecord {
                transfer_id: row.get(0)?,
                direction: row.get(1)?,
                peer_node_id: row.get(2)?,
                file_path: row.get(3)?,
                file_name: row.get(4)?,
                total_bytes: row.get(5)?,
                bytes_confirmed: row.get(6)?,
                chunk_size: row.get(7)?,
                file_hash: row.get(8)?,
                status: row.get(9)?,
                error_message: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn clear_finished_transfers(&self) -> Result<()> {
        let conn = self.state_conn.lock();
        conn.execute(
            "DELETE FROM file_chunks WHERE transfer_id IN (SELECT transfer_id FROM file_transfers WHERE status IN ('complete', 'failed', 'declined'))",
            [],
        )?;
        let deleted = conn.execute(
            "DELETE FROM file_transfers WHERE status IN ('complete', 'failed', 'declined')",
            [],
        )?;
        info!("clear_finished_transfers: deleted {} finished transfers", deleted);
        Ok(())
    }

    pub fn update_paired_device_network_info(&self, node_id: &str, ips: &[String], port: u16) -> Result<()> {
        let conn = self.state_conn.lock();
        let ips_json = serde_json::to_string(ips).unwrap_or_default();
        conn.execute(
            "UPDATE paired_devices SET last_known_ips = ?1, last_known_port = ?2 WHERE node_id = ?3",
            (ips_json, port as i64, node_id),
        )?;
        Ok(())
    }

    pub fn append_event(&self, payload: &[u8], source: &str) -> Result<String> {
        let new_hash = {
            let conn = self.events_conn.lock();

            let last_row: Option<(String, Vec<u8>)> = conn
                .query_row(
                    "SELECT hash, payload FROM events ORDER BY id DESC LIMIT 1",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
                )
                .optional()?;

            if let Some((prev_hash, prev_payload)) = last_row {
                let prev_str = String::from_utf8_lossy(&prev_payload);
                let current_str = String::from_utf8_lossy(payload);
                if prev_str.trim() == current_str.trim() {
                    info!("Payload is identical (trimmed) to the most recent event, skipping append.");
                    return Ok(prev_hash);
                }
            }

            // Global deduplication: Find and delete any existing rows with matching trimmed payload.
            let current_str = String::from_utf8_lossy(payload);
            let current_trimmed = current_str.trim();

            let mut stmt = conn.prepare("SELECT id, payload FROM events")?;
            let event_rows = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?;

            let mut ids_to_delete = Vec::new();
            for r in event_rows {
                if let Ok((id, db_payload)) = r {
                    let db_str = String::from_utf8_lossy(&db_payload);
                    if db_str.trim() == current_trimmed {
                        ids_to_delete.push(id);
                    }
                }
            }

            for id in ids_to_delete {
                conn.execute("DELETE FROM events WHERE id = ?", [id])?;
            }

            let last_hash: Option<String> = conn
                .query_row(
                    "SELECT hash FROM events ORDER BY id DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()?;

            let mut hasher = blake3::Hasher::new();
            if let Some(prev_hash_str) = last_hash {
                hasher.update(prev_hash_str.as_bytes());
            } else {
                hasher.update(b"CDUS_GENESIS");
            }
            hasher.update(payload);
            hasher.update(source.as_bytes());
            let hash = hasher.finalize().to_hex().to_string();

            conn.execute(
                "INSERT INTO events (payload, source, hash) VALUES (?1, ?2, ?3)",
                (payload, source, &hash),
            )?;

            info!("Appended event from {} with hash: {}", source, hash);
            hash
        };

        // Index the clipboard item
        let _ = self.index_clipboard_item(&new_hash, payload, source);

        // Prune events based on history limit and auto-expiry configurations
        let _ = self.prune_events();

        Ok(new_hash)
    }

    pub fn delete_event(&self, id: i64) -> Result<()> {
        let conn = self.events_conn.lock();
        let hash: Option<String> = conn
            .query_row(
                "SELECT hash FROM events WHERE id = ?",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        
        if let Some(hash_str) = hash {
            let search_conn = self.search_conn.lock();
            let _ = search_conn.execute("DELETE FROM search_index WHERE id = ?", [&hash_str]);
        }

        conn.execute("DELETE FROM events WHERE id = ?", [id])?;
        Ok(())
    }

    pub fn clear_events(&self) -> Result<()> {
        let conn = self.events_conn.lock();
        conn.execute("DELETE FROM events", [])?;
        
        let search_conn = self.search_conn.lock();
        let _ = search_conn.execute("DELETE FROM search_index WHERE item_type = 'clipboard'", []);
        Ok(())
    }

    pub fn prune_events(&self) -> Result<()> {
        let limit = self.get_state("clipboard_limit")
            .unwrap_or(None)
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(50);
        let days = self.get_state("clipboard_expiry_days")
            .unwrap_or(None)
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(7);

        let conn = self.events_conn.lock();

        // 1. Delete events older than expiry days
        conn.execute(
            &format!("DELETE FROM events WHERE timestamp < datetime('now', '-{} days')", days),
            [],
        )?;

        // 2. Keep only the latest 'limit' events
        conn.execute(
            "DELETE FROM events WHERE id NOT IN (
                SELECT id FROM events ORDER BY id DESC LIMIT ?
            )",
            [limit],
        )?;

        // Sync search index
        let remaining_hashes: Vec<String> = {
            let mut stmt = conn.prepare("SELECT hash FROM events")?;
            let hash_iter = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut hashes = Vec::new();
            for h in hash_iter {
                if let Ok(h_str) = h {
                    hashes.push(h_str);
                }
            }
            hashes
        };
        
        let search_conn = self.search_conn.lock();
        if remaining_hashes.is_empty() {
            let _ = search_conn.execute("DELETE FROM search_index WHERE item_type = 'clipboard'", []);
        } else {
            let vars = vec!["?"; remaining_hashes.len()].join(",");
            let sql = format!("DELETE FROM search_index WHERE item_type = 'clipboard' AND id NOT IN ({})", vars);
            let params = rusqlite::params_from_iter(remaining_hashes.iter());
            let _ = search_conn.execute(&sql, params);
        }

        Ok(())
    }

    pub fn get_recent_events(&self, limit: u32) -> Result<Vec<ClipboardEvent>> {
        let conn = self.events_conn.lock();

        let mut stmt = conn.prepare(
            "SELECT id, payload, source, timestamp, local_only FROM events WHERE length(payload) > 0 ORDER BY id DESC LIMIT ?",
        )?;

        let event_iter = stmt.query_map([limit], |row| {
            let payload: Vec<u8> = row.get(1)?;
            let content = String::from_utf8(payload)
                .unwrap_or_else(|_| "[invalid utf8]".to_string());
            let is_sensitive = cdus_common::is_sensitive_content(&content);
            let local_only: bool = row.get(4)?;
            Ok(ClipboardEvent {
                id: row.get(0)?,
                content,
                source: row.get(2)?,
                timestamp: row.get(3)?,
                is_sensitive,
                local_only,
            })
        })?;

        let mut events = Vec::new();
        for event in event_iter {
            events.push(event?);
        }
        Ok(events)
    }

    pub fn set_local_only(&self, id: i64, local_only: bool) -> Result<()> {
        let conn = self.events_conn.lock();
        conn.execute(
            "UPDATE events SET local_only = ?1 WHERE id = ?2",
            (local_only as i32, id),
        )?;
        Ok(())
    }

    pub fn is_content_local_only(&self, content: &str) -> Result<bool> {
        let conn = self.events_conn.lock();
        let mut stmt = conn.prepare("SELECT local_only, payload FROM events")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, bool>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;

        let trimmed_target = content.trim();
        for r in rows {
            if let Ok((local_only, db_payload)) = r {
                let db_str = String::from_utf8_lossy(&db_payload);
                if db_str.trim() == trimmed_target && local_only {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    pub fn is_current_local_only(&self) -> Result<bool> {
        let conn = self.events_conn.lock();
        let local_only: Option<bool> = conn
            .query_row(
                "SELECT local_only FROM events ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(local_only.unwrap_or(false))
    }

    pub fn set_state(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.state_conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO state (key, value) VALUES (?1, ?2)",
            (key, value),
        )?;
        Ok(())
    }

    pub fn get_state(&self, key: &str) -> Result<Option<String>> {
        let conn = self.state_conn.lock();
        conn.query_row("SELECT value FROM state WHERE key = ?", [key], |row| {
            row.get(0)
        })
        .optional()
    }

    pub fn get_or_create_identity(
        &self,
        data_dir: &std::path::Path,
    ) -> anyhow::Result<(String, Vec<u8>)> {
        use keyring::Entry;
        use snow::{params::NoiseParams, Builder};

        let params: NoiseParams = "Noise_XX_25519_ChaChaPoly_BLAKE2s".parse()?;

        // Create a unique service name for the keychain based on the data directory
        let dir_hash = blake3::hash(data_dir.to_string_lossy().as_bytes())
            .to_hex()
            .to_string();
        let service_name = format!("com.cdus.agent.{}", &dir_hash[..8]);

        let mut loaded_priv_bytes = None;

        // Try keyring first
        if let Ok(entry) = Entry::new(&service_name, "private_key") {
            if let Ok(priv_key_hex) = entry.get_password() {
                if let Ok(priv_bytes) = hex::decode(priv_key_hex) {
                    loaded_priv_bytes = Some(priv_bytes);
                }
            }
        }

        // Try database if keyring failed or was not available
        if loaded_priv_bytes.is_none() {
            if let Ok(Some(priv_key_hex)) = self.get_state("private_key") {
                if let Ok(priv_bytes) = hex::decode(priv_key_hex) {
                    info!("Loaded identity private key from state database fallback.");
                    loaded_priv_bytes = Some(priv_bytes);
                }
            }
        }

        if let Some(priv_bytes) = loaded_priv_bytes {
            if let Some(existing_id) = self.get_state("node_id")? {
                if self.get_state("id_migrated_v3")?.is_some() {
                    // Check if it's actually a valid PeerId string (Base58)
                    if existing_id.parse::<libp2p::PeerId>().is_ok() {
                        return Ok((existing_id, priv_bytes));
                    }
                }

                // Attempt to migrate existing hex ID to PeerId
                let mut temp_builder = Builder::new(params.clone());
                temp_builder = temp_builder.local_private_key(&priv_bytes);
                if let Ok(_handshake) = temp_builder.build_initiator() {
                    if let Ok(bytes) = priv_bytes.clone().try_into() {
                        let signing_key = ed25519_dalek::SigningKey::from_bytes(&bytes);
                        let pub_bytes = signing_key.verifying_key().to_bytes().to_vec();
                        let pub_hex = hex::encode(&pub_bytes);
                        if let Ok(node_id) = crate::utils::hex_to_peer_id(&pub_hex) {
                            info!("Migrating hex node_id to PeerId: {}", node_id);
                            self.set_state("node_id", &node_id)?;
                            self.set_state("id_migrated_v3", "true")?;
                            return Ok((node_id, priv_bytes));
                        }
                    }
                }
            }
        }

        info!("Generating fresh Noise identity for {}...", service_name);

        let mut rng = rand::rngs::OsRng;
        let signing_key = ed25519_dalek::SigningKey::generate(&mut rng);
        let priv_bytes = signing_key.to_bytes().to_vec();
        let pub_bytes = signing_key.verifying_key().to_bytes().to_vec();
        let pub_hex = hex::encode(&pub_bytes);
        let node_id = crate::utils::hex_to_peer_id(&pub_hex)?;

        self.set_state("node_id", &node_id)?;
        self.set_state("id_migrated_v3", "true")?; // Bump version to v3
        
        // Save to local database
        self.set_state("private_key", &hex::encode(&priv_bytes))?;

        // Save to keyring (best effort, do not crash if unsupported/fails)
        if let Ok(entry) = Entry::new(&service_name, "private_key") {
            if let Err(e) = entry.set_password(&hex::encode(&priv_bytes)) {
                info!("Keyring is not available or failed to save private key (falling back to database): {}", e);
            }
        }

        if self.get_state("device_name")?.is_none() {
            let hostname = gethostname::gethostname()
                .into_string()
                .unwrap_or_else(|_| "Unknown Device".to_string());
            self.set_state("device_name", &format!("{} ({})", hostname, &dir_hash[..4]))?;
        }

        Ok((node_id, priv_bytes))
    }

    pub fn add_paired_device(&self, node_id: &str, label: &str, static_key: Option<&[u8]>) -> Result<()> {
        let conn = self.state_conn.lock();
        conn.execute(
            "INSERT INTO paired_devices (node_id, label, static_key) VALUES (?1, ?2, ?3)
             ON CONFLICT(node_id) DO UPDATE SET label = excluded.label, static_key = COALESCE(excluded.static_key, paired_devices.static_key)",
            (node_id, label, static_key),
        )?;
        let _ = self.index_paired_device(node_id, label);
        Ok(())
    }

    pub fn remove_paired_device(&self, node_id: &str) -> Result<()> {
        let conn = self.state_conn.lock();
        conn.execute("DELETE FROM paired_devices WHERE node_id = ?", [node_id])?;
        let search_conn = self.search_conn.lock();
        let _ = search_conn.execute("DELETE FROM search_index WHERE id = ?", [node_id]);
        Ok(())
    }

    pub fn get_paired_devices(&self) -> Result<Vec<PairedDeviceRecord>> {
        let conn = self.state_conn.lock();
        let mut stmt = conn.prepare(
            "SELECT node_id, label, last_known_ips, last_known_port, static_key FROM paired_devices WHERE node_id != 'unknown'",
        )?;
        let rows = stmt.query_map([], |row| {
            let ips_str: Option<String> = row.get(2)?;
            let last_known_ips = ips_str.map(|s| {
                s.split(',')
                    .filter(|ss| !ss.is_empty())
                    .map(|ss| ss.to_string())
                    .collect()
            });

            Ok(PairedDeviceRecord {
                node_id: row.get(0)?,
                label: row.get(1)?,
                last_known_ips,
                last_known_port: row.get::<_, Option<i64>>(3)?.map(|p| p as u16),
                static_key: row.get(4)?,
            })
        })?;

        let mut devices = Vec::new();
        for row in rows {
            devices.push(row?);
        }
        Ok(devices)
    }

    pub fn get_paired_device(&self, node_id: &str) -> Result<Option<PairedDeviceRecord>> {
        let conn = self.state_conn.lock();
        conn.query_row(
            "SELECT node_id, label, last_known_ips, last_known_port, static_key FROM paired_devices WHERE node_id = ?",
            [node_id],
            |row| {
                let ips_str: Option<String> = row.get(2)?;
                let last_known_ips = ips_str.map(|s| {
                    s.split(',')
                        .filter(|ss| !ss.is_empty())
                        .map(|ss| ss.to_string())
                        .collect()
                });

                Ok(PairedDeviceRecord {
                    node_id: row.get(0)?,
                    label: row.get(1)?,
                    last_known_ips,
                    last_known_port: row.get::<_, Option<i64>>(3)?.map(|p| p as u16),
                    static_key: row.get(4)?,
                })
            },
        )
        .optional()
    }

    pub fn get_node_id_by_static_key(&self, static_key: &[u8]) -> Result<Option<String>> {
        let conn = self.state_conn.lock();
        conn.query_row(
            "SELECT node_id FROM paired_devices WHERE static_key = ?",
            [static_key],
            |row| row.get(0),
        )
        .optional()
    }

    pub fn is_device_paired(&self, node_id: &str) -> Result<bool> {
        let conn = self.state_conn.lock();
        let count: i64 = conn.query_row(
            "SELECT count(*) FROM paired_devices WHERE node_id = ?",
            [node_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn append_audit_log(&self, event_type: &str, content: &str) -> Result<()> {
        let conn = self.state_conn.lock();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        conn.execute(
            "INSERT INTO audit_logs (event_type, content, timestamp) VALUES (?1, ?2, ?3)",
            (event_type, content, now),
        )?;
        Ok(())
    }

    pub fn get_audit_logs(&self, limit: u32) -> Result<Vec<cdus_common::AuditLogRecord>> {
        let conn = self.state_conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, event_type, content, timestamp FROM audit_logs ORDER BY timestamp DESC LIMIT ?"
        )?;
        let rows = stmt.query_map([limit], |row| {
            Ok(cdus_common::AuditLogRecord {
                id: row.get(0)?,
                event_type: row.get(1)?,
                content: row.get(2)?,
                timestamp: row.get(3)?,
            })
        })?;
        let mut results = Vec::new();
        for r in rows {
            results.push(r?);
        }
        Ok(results)
    }

    pub fn clear_audit_logs(&self) -> Result<()> {
        let conn = self.state_conn.lock();
        conn.execute("DELETE FROM audit_logs", [])?;
        Ok(())
    }

fn parse_clipboard_payload(payload_str: &str) -> (String, bool, Option<String>) {
    let trimmed = payload_str.trim();
    if trimmed.is_empty() {
        return ("".to_string(), false, None);
    }

    // Try parsing payload as resolved URL metadata JSON.
    let mut resolved_url: Option<(String, String)> = None; // (title, url)
    if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if json_val["type"] == "url" {
            let u = json_val["url"].as_str().unwrap_or("").to_string();
            let t = json_val["title"].as_str().unwrap_or("").to_string();
            let display_title = if t.trim().is_empty() {
                u.clone()
            } else {
                t
            };
            resolved_url = Some((display_title, u));
        }
    }

    if let Some((t, u)) = resolved_url {
        (t, true, Some(u))
    } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        (trimmed.to_string(), true, Some(trimmed.to_string()))
    } else {
        let mut preview = trimmed.chars().take(50).collect::<String>();
        if trimmed.chars().count() > 50 {
            preview.push_str("...");
        }
        (preview, false, None)
    }
}

    pub fn index_clipboard_item(&self, hash: &str, payload: &[u8], source: &str) -> Result<()> {
        let text = String::from_utf8_lossy(payload).to_string();
        let (title, is_url, url_to_parse) = Self::parse_clipboard_payload(&text);
        if title.is_empty() {
            return Ok(());
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let subtitle = if let Some(ref u) = url_to_parse {
            let mut truncated_url = u.chars().take(60).collect::<String>();
            if u.chars().count() > 60 {
                truncated_url.push_str("...");
            }
            format!("{} • synced from {}", truncated_url, source)
        } else {
            format!("synced from {} • clipboard history", source)
        };

        let mut content = format!("{} {} {}", title, source, if is_url { "url" } else { "text" });
        if let Some(ref u) = url_to_parse {
            if let Ok(url) = url::Url::parse(u) {
                if let Some(host) = url.host_str() {
                    content = format!("{} {} {} {} {}", title, u, host, source, "url");
                }
            }
        }

        let search_conn = self.search_conn.lock();
        search_conn.execute(
            "INSERT OR REPLACE INTO search_index (id, item_type, title, subtitle, content, timestamp) VALUES (?1, 'clipboard', ?2, ?3, ?4, ?5)",
            (hash, &title, &subtitle, &content, now),
        )?;

        Ok(())
    }

    pub fn index_file_transfer(
        &self,
        transfer_id: &str,
        direction: &str,
        peer_label: &str,
        file_name: &str,
        total_bytes: u64,
        status: &str,
    ) -> Result<()> {

        let size_str = {
            const KB: u64 = 1024;
            const MB: u64 = 1024 * 1024;
            const GB: u64 = 1024 * 1024 * 1024;
            if total_bytes >= GB {
                format!("{:.2} GB", total_bytes as f64 / GB as f64)
            } else if total_bytes >= MB {
                format!("{:.2} MB", total_bytes as f64 / MB as f64)
            } else if total_bytes >= KB {
                format!("{:.2} KB", total_bytes as f64 / KB as f64)
            } else {
                format!("{} B", total_bytes)
            }
        };

        let title = file_name.to_string();
        let subtitle = if direction == "outgoing" {
            format!("Size: {} • sent to {} ({})", size_str, peer_label, status)
        } else {
            format!("Size: {} • received from {} ({})", size_str, peer_label, status)
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let content = format!("{} {} {} file", file_name, peer_label, direction);

        let search_conn = self.search_conn.lock();
        search_conn.execute(
            "INSERT OR REPLACE INTO search_index (id, item_type, title, subtitle, content, timestamp) VALUES (?1, 'file', ?2, ?3, ?4, ?5)",
            (transfer_id, &title, &subtitle, &content, now),
        )?;

        Ok(())
    }

    pub fn index_paired_device(&self, node_id: &str, label: &str) -> Result<()> {
        let title = label.to_string();
        let subtitle = format!("Device ID: {} • Paired", node_id);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let content = format!("{} {} device", label, node_id);

        let search_conn = self.search_conn.lock();
        search_conn.execute(
            "INSERT OR REPLACE INTO search_index (id, item_type, title, subtitle, content, timestamp) VALUES (?1, 'device', ?2, ?3, ?4, ?5)",
            (node_id, &title, &subtitle, &content, now),
        )?;

        Ok(())
    }

    pub fn search(&self, query: &str) -> Result<Vec<cdus_common::SearchResult>> {
        let trimmed_query = query.trim().to_lowercase();
        let search_conn = self.search_conn.lock();

        let mut stmt = search_conn.prepare(
            "SELECT id, item_type, title, subtitle, content, timestamp FROM search_index"
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?;

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let local_device_name = self.get_state("device_name")
            .unwrap_or(None)
            .unwrap_or_else(|| "".to_string())
            .to_lowercase();

        let mut scored_results = Vec::new();

        if trimmed_query.is_empty() {
            for row in rows {
                if let Ok((id, item_type, title, subtitle, _content, timestamp)) = row {
                    scored_results.push((
                        cdus_common::SearchResult {
                            id,
                            item_type,
                            title,
                            subtitle,
                            timestamp: timestamp as u64,
                        },
                        timestamp as f64,
                    ));
                }
            }
            scored_results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let results: Vec<cdus_common::SearchResult> = scored_results
                .into_iter()
                .take(50)
                .map(|(r, _)| r)
                .collect();
            return Ok(results);
        }

        let query_terms: Vec<&str> = trimmed_query.split_whitespace().collect();

        for row in rows {
            if let Ok((id, item_type, title, subtitle, content, timestamp)) = row {
                let content_str = content.unwrap_or_default();
                
                let mut relevance_score = 0.0;
                let title_lower = title.to_lowercase();
                let subtitle_lower = subtitle.to_lowercase();
                let content_lower = content_str.to_lowercase();

                let mut matched = false;
                for term in &query_terms {
                    let mut term_matched = false;
                    if title_lower.contains(term) {
                        term_matched = true;
                        if title_lower.starts_with(term) {
                            relevance_score += 25.0;
                        } else {
                            relevance_score += 15.0;
                        }
                        if title_lower == *term {
                            relevance_score += 50.0;
                        }
                    }
                    if content_lower.contains(term) {
                        term_matched = true;
                        relevance_score += 10.0;
                    }
                    if subtitle_lower.contains(term) {
                        term_matched = true;
                        relevance_score += 5.0;
                    }

                    if term_matched {
                        matched = true;
                    }
                }

                if !matched {
                    continue;
                }

                if query_terms.len() > 1 {
                    if title_lower.contains(&trimmed_query) {
                        relevance_score += 40.0;
                    }
                    if content_lower.contains(&trimmed_query) {
                        relevance_score += 20.0;
                    }
                }

                let age_seconds = ((now_ms - timestamp) as f64 / 1000.0).max(0.0);
                let recency_score = 30.0 / (1.0 + (age_seconds / 7200.0));

                let mut proximity_score = 0.0;
                let is_local = subtitle_lower.contains("local") 
                    || content_lower.contains("local")
                    || (!local_device_name.is_empty() && (
                        subtitle_lower.contains(&local_device_name) 
                        || content_lower.contains(&local_device_name)
                    ));

                if is_local {
                    proximity_score += 15.0;
                } else {
                    proximity_score += 5.0;
                }

                let total_score = relevance_score + recency_score + proximity_score;

                scored_results.push((
                    cdus_common::SearchResult {
                        id,
                        item_type,
                        title,
                        subtitle,
                        timestamp: timestamp as u64,
                    },
                    total_score,
                ));
            }
        }

        scored_results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let results: Vec<cdus_common::SearchResult> = scored_results
            .into_iter()
            .take(50)
            .map(|(r, _)| r)
            .collect();

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_event_chaining() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();

        let h1 = store.append_event(b"event1", "Local").unwrap();
        let h2 = store.append_event(b"event2", "Remote").unwrap();
        let h3 = store.append_event(b"event3", "Local").unwrap();

        assert_ne!(h1, h2);
        assert_ne!(h2, h3);

        let mut hasher = blake3::Hasher::new();
        hasher.update(b"CDUS_GENESIS");
        hasher.update(b"event1");
        hasher.update(b"Local");
        let expected_h1 = hasher.finalize().to_hex().to_string();
        assert_eq!(h1, expected_h1);
    }

    #[test]
    fn test_get_recent_events() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();

        store.append_event(b"event1", "Local").unwrap();
        store.append_event(b"event2", "iPhone").unwrap();

        let events = store.get_recent_events(10).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].content, "event2");
        assert_eq!(events[0].source, "iPhone");
        assert_eq!(events[1].content, "event1");
        assert_eq!(events[1].source, "Local");
    }

    #[test]
    fn test_append_event_deduplication() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();

        let h1 = store.append_event(b"same_content", "Local").unwrap();
        let h2 = store.append_event(b"same_content\n", "Local").unwrap();
        let h3 = store.append_event(b"different_content", "Local").unwrap();
        let h4 = store.append_event(b"same_content ", "Local").unwrap();

        assert_eq!(h1, h2);
        assert_ne!(h2, h3);
        assert_ne!(h3, h4);

        let events = store.get_recent_events(10).unwrap();
        // same_content (consecutive duplicate with newline) should be skipped once.
        // different_content should be appended.
        // same_content (non-consecutive with trailing space) should trigger global deduplication:
        // the old instance of "same_content" is deleted, and "same_content " is moved/appended to the top.
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].content, "same_content ");
        assert_eq!(events[1].content, "different_content");
    }

    #[test]
    fn test_delete_transfer() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();

        let transfer_id = "test-transfer-id";
        store.create_transfer(
            transfer_id,
            "outgoing",
            "peer-1",
            "path/to/file",
            "file.txt",
            100,
            10,
            "hash"
        ).unwrap();

        let history = store.get_transfer_history(10).unwrap();
        assert_eq!(history.len(), 1);

        store.delete_transfer(transfer_id).unwrap();

        let history = store.get_transfer_history(10).unwrap();
        assert_eq!(history.len(), 0);
    }

    #[test]
    fn test_audit_logs() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();

        // 1. Initially empty
        let logs = store.get_audit_logs(10).unwrap();
        assert_eq!(logs.len(), 0);

        // 2. Append logs
        store.append_audit_log("sync", "Outgoing sync content").unwrap();
        store.append_audit_log("pairing", "Device paired").unwrap();

        // 3. Get logs (newest first)
        let logs = store.get_audit_logs(10).unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].event_type, "pairing");
        assert_eq!(logs[0].content, "Device paired");
        assert_eq!(logs[1].event_type, "sync");
        assert_eq!(logs[1].content, "Outgoing sync content");
        assert!(logs[0].timestamp > 0);

        // 4. Limit parameter
        let logs_limit = store.get_audit_logs(1).unwrap();
        assert_eq!(logs_limit.len(), 1);
        assert_eq!(logs_limit[0].content, "Device paired");

        // 5. Clear logs
        store.clear_audit_logs().unwrap();
        let logs_cleared = store.get_audit_logs(10).unwrap();
        assert_eq!(logs_cleared.len(), 0);
    }

    #[test]
    fn test_search_index() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();

        // 1. Test indexing paired device
        store.add_paired_device("node-1", "My Phone", None).unwrap();
        let results = store.search("Phone").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item_type, "device");
        assert_eq!(results[0].title, "My Phone");

        // 2. Test indexing clipboard item
        let hash = store.append_event(b"Hello World from Antigravity!", "Local").unwrap();
        let results = store.search("Antigravity").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item_type, "clipboard");
        assert_eq!(results[0].title, "Hello World from Antigravity!");
        assert_eq!(results[0].id, hash);

        // 3. Test indexing file transfer
        store.create_transfer(
            "transfer-123",
            "incoming",
            "node-1",
            "path/to/somefile.txt",
            "somefile.txt",
            5000000,
            256,
            "somehash"
        ).unwrap();
        let results = store.search("somefile").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item_type, "file");
        assert_eq!(results[0].title, "somefile.txt");
        assert!(results[0].subtitle.contains("4.77 MB"));

        // 4. Test rank/score: fuzzy search relevance
        let results = store.search("somefile").unwrap();
        assert_eq!(results[0].title, "somefile.txt");

        // Test delete paired device
        store.remove_paired_device("node-1").unwrap();
        let results = store.search("Phone").unwrap();
        assert!(!results.iter().any(|r| r.item_type == "device"));

        // Test delete transfer
        store.delete_transfer("transfer-123").unwrap();
        let results = store.search("somefile").unwrap();
        assert_eq!(results.len(), 0);

        // Test indexing resolved URL JSON metadata
        let url_payload = serde_json::json!({
            "type": "url",
            "url": "https://google.com/search?q=antigravity",
            "title": "Google Search - Antigravity",
            "favicon": "data:image/png;base64,1234"
        }).to_string();
        let url_hash = store.append_event(url_payload.as_bytes(), "Remote").unwrap();
        let results = store.search("Search").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item_type, "clipboard");
        assert_eq!(results[0].title, "Google Search - Antigravity");
        assert_eq!(results[0].subtitle, "https://google.com/search?q=antigravity • synced from Remote");
        assert_eq!(results[0].id, url_hash);

        // Test searching by part of URL string
        let results_by_url = store.search("google.com").unwrap();
        assert_eq!(results_by_url.len(), 1);
        assert_eq!(results_by_url[0].id, url_hash);
    }

    #[test]
    fn test_telemetry_opt_in() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();

        // Default should be None / false
        let opt_in = store.get_state("telemetry_opt_in")
            .unwrap()
            .map(|val| val == "true")
            .unwrap_or(false);
        assert!(!opt_in);

        // Enable telemetry
        store.set_state("telemetry_opt_in", "true").unwrap();
        let opt_in = store.get_state("telemetry_opt_in")
            .unwrap()
            .map(|val| val == "true")
            .unwrap_or(false);
        assert!(opt_in);

        // Disable telemetry
        store.set_state("telemetry_opt_in", "false").unwrap();
        let opt_in = store.get_state("telemetry_opt_in")
            .unwrap()
            .map(|val| val == "true")
            .unwrap_or(false);
        assert!(!opt_in);
    }
}

