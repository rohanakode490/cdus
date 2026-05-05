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

        state_conn.execute(
            "CREATE TABLE IF NOT EXISTS state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
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
