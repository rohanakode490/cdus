use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct ClipboardEvent {
    pub id: i64,
    pub content: String,
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum IpcMessage {
    Ping,
    Pong,
    Log(String),
    ClipboardChanged(String),
    SetClipboard(String),
    GetHistory { limit: u32 },
    HistoryResponse(Vec<ClipboardEvent>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmp_serde;

    #[test]
    fn test_ipc_message_roundtrip_json() {
        let msg = IpcMessage::Log("test message".to_string());
        let serialized = serde_json::to_string(&msg).unwrap();
        let deserialized: IpcMessage = serde_json::from_str(&serialized).unwrap();
        assert_eq!(msg, deserialized);
    }

    #[test]
    fn test_ipc_message_roundtrip_msgpack() {
        let msg = IpcMessage::Ping;
        let mut buf = Vec::new();
        msg.serialize(&mut rmp_serde::Serializer::new(&mut buf)).unwrap();
        
        let mut de = rmp_serde::Deserializer::new(&buf[..]);
        let deserialized: IpcMessage = Deserialize::deserialize(&mut de).unwrap();
        assert_eq!(msg, deserialized);
    }
}
