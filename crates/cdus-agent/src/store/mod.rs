use cdus_common::ClipboardEvent;
use rusqlite::{Connection, OptionalExtension, Result};
use std::path::Path;
use parking_lot::Mutex;
use tracing::info;

pub struct Store {
    pub events_conn: Mutex<Connection>,
    pub state_conn: Mutex<Connection>,
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

        info!("Initializing databases at {}...", data_dir.display());

        let events_conn = Connection::open(events_path)?;
        let state_conn = Connection::open(state_path)?;

        // Enable WAL mode
        events_conn.pragma_update(None, "journal_mode", "WAL")?;
        state_conn.pragma_update(None, "journal_mode", "WAL")?;

        // Initialize tables
        events_conn.execute(
            "CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                payload BLOB NOT NULL,
                source TEXT NOT NULL,
                hash TEXT NOT NULL,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP
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

        Ok(Store {
            events_conn: Mutex::new(events_conn),
            state_conn: Mutex::new(state_conn),
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
        Ok(())
    }

    pub fn delete_transfer(&self, transfer_id: &str) -> Result<()> {
        let conn = self.state_conn.lock();
        conn.execute("DELETE FROM file_chunks WHERE transfer_id = ?1", [transfer_id])?;
        conn.execute("DELETE FROM file_transfers WHERE transfer_id = ?1", [transfer_id])?;
        Ok(())
    }

    pub fn update_transfer_status(&self, transfer_id: &str, status: &str) -> Result<()> {
        let conn = self.state_conn.lock();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        conn.execute(
            "UPDATE file_transfers SET status = ?1, updated_at = ?2 WHERE transfer_id = ?3",
            (status, now, transfer_id),
        )?;
        Ok(())
    }

    pub fn update_transfer_status_error(&self, transfer_id: &str, error: &str) -> Result<()> {
        let conn = self.state_conn.lock();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        conn.execute(
            "UPDATE file_transfers SET status = 'failed', error_message = ?1, updated_at = ?2 WHERE transfer_id = ?3",
            (error, now, transfer_id),
        )?;
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
            "DELETE FROM file_transfers WHERE status IN ('complete', 'failed', 'declined')",
            [],
        )?;
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
        let conn = self.events_conn.lock();

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
        let new_hash = hasher.finalize().to_hex().to_string();

        conn.execute(
            "INSERT INTO events (payload, source, hash) VALUES (?1, ?2, ?3)",
            (payload, source, &new_hash),
        )?;

        info!("Appended event from {} with hash: {}", source, new_hash);
        Ok(new_hash)
    }

    pub fn get_recent_events(&self, limit: u32) -> Result<Vec<ClipboardEvent>> {
        let conn = self.events_conn.lock();

        let mut stmt = conn.prepare(
            "SELECT id, payload, source, timestamp FROM events WHERE length(payload) > 0 ORDER BY id DESC LIMIT ?",
        )?;

        let event_iter = stmt.query_map([limit], |row| {
            let payload: Vec<u8> = row.get(1)?;
            Ok(ClipboardEvent {
                id: row.get(0)?,
                content: String::from_utf8(payload)
                    .unwrap_or_else(|_| "[invalid utf8]".to_string()),
                source: row.get(2)?,
                timestamp: row.get(3)?,
            })
        })?;

        let mut events = Vec::new();
        for event in event_iter {
            events.push(event?);
        }
        Ok(events)
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
        Ok(())
    }

    pub fn remove_paired_device(&self, node_id: &str) -> Result<()> {
        let conn = self.state_conn.lock();
        conn.execute("DELETE FROM paired_devices WHERE node_id = ?", [node_id])?;
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
}
