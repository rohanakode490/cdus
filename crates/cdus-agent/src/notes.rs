use anyhow::Result;
use automerge::sync::SyncDoc;
use automerge::transaction::Transactable;
use automerge::{AutoCommit, ReadDoc, ScalarValue, Value, ROOT};
use cdus_common::NoteRecord;

pub struct AutomergeNote {
    pub doc: AutoCommit,
}

impl AutomergeNote {
    pub fn new(
        doc_id: &str,
        title: &str,
        content: &str,
        created_at: u64,
        updated_at: u64,
    ) -> Result<Self> {
        let mut doc = AutoCommit::new();
        doc.put(ROOT, "doc_id", doc_id)?;
        doc.put(ROOT, "title", title)?;
        doc.put(ROOT, "content", content)?;
        doc.put(ROOT, "created_at", created_at as i64)?;
        doc.put(ROOT, "updated_at", updated_at as i64)?;
        Ok(Self { doc })
    }

    pub fn load(data: &[u8]) -> Result<Self> {
        let doc = AutoCommit::load(data)?;
        Ok(Self { doc })
    }

    pub fn save(&mut self) -> Vec<u8> {
        self.doc.save()
    }

    pub fn to_record(&self) -> Result<NoteRecord> {
        let doc_id = match self.doc.get(ROOT, "doc_id")? {
            Some((Value::Scalar(s), _)) => match s.as_ref() {
                ScalarValue::Str(str_val) => str_val.to_string(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        let title = match self.doc.get(ROOT, "title")? {
            Some((Value::Scalar(s), _)) => match s.as_ref() {
                ScalarValue::Str(str_val) => str_val.to_string(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        let content = match self.doc.get(ROOT, "content")? {
            Some((Value::Scalar(s), _)) => match s.as_ref() {
                ScalarValue::Str(str_val) => str_val.to_string(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        let created_at = match self.doc.get(ROOT, "created_at")? {
            Some((Value::Scalar(s), _)) => match s.as_ref() {
                ScalarValue::Int(i) => *i as u64,
                ScalarValue::Uint(u) => *u,
                _ => 0,
            },
            _ => 0,
        };
        let updated_at = match self.doc.get(ROOT, "updated_at")? {
            Some((Value::Scalar(s), _)) => match s.as_ref() {
                ScalarValue::Int(i) => *i as u64,
                ScalarValue::Uint(u) => *u,
                _ => 0,
            },
            _ => 0,
        };

        Ok(NoteRecord {
            doc_id,
            title,
            content,
            created_at,
            updated_at,
        })
    }

    pub fn update(&mut self, title: &str, content: &str, updated_at: u64) -> Result<()> {
        self.doc.put(ROOT, "title", title)?;
        self.doc.put(ROOT, "content", content)?;
        self.doc.put(ROOT, "updated_at", updated_at as i64)?;
        Ok(())
    }

    pub fn generate_sync_message(
        &mut self,
        sync_state: &mut automerge::sync::State,
    ) -> Option<Vec<u8>> {
        self.doc
            .sync()
            .generate_sync_message(sync_state)
            .map(|m| m.encode())
    }

    pub fn receive_sync_message(
        &mut self,
        sync_state: &mut automerge::sync::State,
        msg_bytes: &[u8],
    ) -> Result<()> {
        let msg = automerge::sync::Message::decode(msg_bytes)?;
        self.doc.sync().receive_sync_message(sync_state, msg)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_automerge_note_create_and_sync() {
        let mut note1 = AutomergeNote::new("doc-1", "My Note", "Hello World", 1000, 1000).unwrap();
        let bytes = note1.save();
        let note2 = AutomergeNote::load(&bytes).unwrap();
        let record = note2.to_record().unwrap();
        assert_eq!(record.doc_id, "doc-1");
        assert_eq!(record.title, "My Note");
        assert_eq!(record.content, "Hello World");
    }

    #[test]
    fn test_automerge_two_way_sync() {
        let mut peer_a =
            AutomergeNote::new("doc-sync", "Title A", "Initial Text", 1000, 1000).unwrap();
        let bytes_a = peer_a.save();
        let mut peer_b = AutomergeNote::load(&bytes_a).unwrap();

        // Concurrent modifications
        peer_a
            .update("Title Updated A", "Initial Text + A changes", 1010)
            .unwrap();
        peer_b
            .update("Title Updated B", "Initial Text + B changes", 1020)
            .unwrap();

        let mut sync_state_a = automerge::sync::State::new();
        let mut sync_state_b = automerge::sync::State::new();

        // Round 1: A sends to B
        let msg_a = peer_a.generate_sync_message(&mut sync_state_a).unwrap();
        peer_b
            .receive_sync_message(&mut sync_state_b, &msg_a)
            .unwrap();

        // Round 2: B sends back to A
        let msg_b = peer_b.generate_sync_message(&mut sync_state_b).unwrap();
        peer_a
            .receive_sync_message(&mut sync_state_a, &msg_b)
            .unwrap();

        // Round 3: A acknowledges to B
        if let Some(msg_a2) = peer_a.generate_sync_message(&mut sync_state_a) {
            peer_b
                .receive_sync_message(&mut sync_state_b, &msg_a2)
                .unwrap();
        }

        // Both documents have converged and are valid Automerge records
        let rec_a = peer_a.to_record().unwrap();
        let rec_b = peer_b.to_record().unwrap();

        assert_eq!(rec_a.doc_id, rec_b.doc_id);
        assert_eq!(rec_a.title, rec_b.title);
        assert_eq!(rec_a.content, rec_b.content);
    }
}
