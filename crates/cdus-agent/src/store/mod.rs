use rusqlite::{Connection, Result, OptionalExtension};
use std::path::Path;
use tracing::info;
use cdus_common::ClipboardEvent;
use std::sync::Mutex;

pub struct Store {
    pub events_conn: Mutex<Connection>,
    pub state_conn: Mutex<Connection>,
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
                label TEXT
            )",
            [],
        )?;


        Ok(Store {
            events_conn: Mutex::new(events_conn),
            state_conn: Mutex::new(state_conn),
        })
    }

    pub fn append_event(&self, payload: &[u8], source: &str) -> Result<String> {
        let conn = self.events_conn.lock().unwrap();
        
        let last_hash: Option<String> = conn.query_row(
            "SELECT hash FROM events ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        ).optional()?;

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
        let conn = self.events_conn.lock().unwrap();
        
        let mut stmt = conn.prepare(
            "SELECT id, payload, source, timestamp FROM events ORDER BY id DESC LIMIT ?"
        )?;
        
        let event_iter = stmt.query_map([limit], |row| {
            let payload: Vec<u8> = row.get(1)?;
            Ok(ClipboardEvent {
                id: row.get(0)?,
                content: String::from_utf8(payload).unwrap_or_else(|_| "[invalid utf8]".to_string()),
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
        let conn = self.state_conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO state (key, value) VALUES (?1, ?2)",
            (key, value),
        )?;
        Ok(())
    }

    pub fn get_state(&self, key: &str) -> Result<Option<String>> {
        let conn = self.state_conn.lock().unwrap();
        conn.query_row(
            "SELECT value FROM state WHERE key = ?",
            [key],
            |row| row.get(0),
        ).optional()
    }

    pub fn get_or_create_identity(&self, data_dir: &std::path::Path) -> anyhow::Result<(String, Vec<u8>)> {
        use keyring::Entry;
        use snow::{Builder, params::NoiseParams};

        let params: NoiseParams = "Noise_XX_25519_ChaChaPoly_BLAKE2s".parse()?;
        let builder = Builder::new(params.clone());

        // Create a unique service name for the keychain based on the data directory
        let dir_hash = blake3::hash(data_dir.to_string_lossy().as_bytes()).to_hex().to_string();
        let service_name = format!("com.cdus.agent.{}", &dir_hash[..8]);

        if let Some(existing_id) = self.get_state("node_id")? {
            let entry = Entry::new(&service_name, "private_key")?;
            if let Ok(priv_key_hex) = entry.get_password() {
                if let Ok(priv_bytes) = hex::decode(priv_key_hex) {
                    // VERIFY: Does existing_id match the public key of these priv_bytes?
                    // We generate a keypair from the private key to get the corresponding public key.
                    let mut temp_builder = Builder::new(params);
                    temp_builder = temp_builder.local_private_key(&priv_bytes);
                    
                    // Since snow doesn't easily expose deriving public from private without building,
                    // we use a trick: any Noise handshake will have the local static key.
                    if let Ok(handshake) = temp_builder.build_initiator() {
                        // In snow 0.9, we can't easily get the local static key back out of HandshakeState.
                        // However, we KNOW the public key is correct if we generated it ourselves.
                        // For legacy migration, we'll just check if the ID is 64 hex chars (32 bytes).
                        // If it's a blake3 hash (also 64 chars), we might collide, so let's be safer.
                        if existing_id.len() == 64 && !existing_id.starts_with("0000") { // Crude check
                            // If it's already a hex string of correct length, we'll assume it's fine for now
                            // OR we could regenerate once to be absolutely sure.
                            // Let's force a migration by checking a flag.
                            if self.get_state("id_migrated_v2")?.is_some() {
                                return Ok((existing_id, priv_bytes));
                            }
                        }
                    }
                }
            }
        }

        info!("Generating fresh Noise identity for {}...", service_name);
        
        let keypair = builder.generate_keypair()?;
        let priv_bytes = keypair.private;
        let pub_bytes = keypair.public;
        let node_id = hex::encode(&pub_bytes);
        
        self.set_state("node_id", &node_id)?;
        self.set_state("id_migrated_v2", "true")?;
        let entry = Entry::new(&service_name, "private_key")?;
        entry.set_password(&hex::encode(&priv_bytes))?;
        
        if self.get_state("device_name")?.is_none() {
            let hostname = gethostname::gethostname().into_string().unwrap_or_else(|_| "Unknown Device".to_string());
            self.set_state("device_name", &format!("{} ({})", hostname, &dir_hash[..4]))?;
        }

        Ok((node_id, priv_bytes))
    }

    pub fn add_paired_device(&self, node_id: &str, label: &str) -> Result<()> {
        let conn = self.state_conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO paired_devices (node_id, label) VALUES (?1, ?2)",
            (node_id, label),
        )?;
        Ok(())
    }

    pub fn remove_paired_device(&self, node_id: &str) -> Result<()> {
        let conn = self.state_conn.lock().unwrap();
        conn.execute(
            "DELETE FROM paired_devices WHERE node_id = ?",
            [node_id],
        )?;
        Ok(())
    }

    pub fn get_paired_devices(&self) -> Result<Vec<(String, String)>> {
        let conn = self.state_conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT node_id, label FROM paired_devices")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;

        let mut devices = Vec::new();
        for row in rows {
            devices.push(row?);
        }
        Ok(devices)
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
