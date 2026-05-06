use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct ClipboardEvent {
    pub id: i64,
    pub content: String,
    pub source: String, // "Local" or remote device name
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
    GetState { key: String },
    SetState { key: String, value: String },
    StateResponse(Option<String>),
    StartScan,
    StopScan,
    DeviceDiscovered { node_id: String, label: String, os: String, ip: String },
    GetDiscovered,
    DiscoveredResponse(Vec<(String, String, String, String)>),
    PairWith { node_id: String },
    PairWithIp { ip: String, port: u16 },
    PairingPin(String),
    ConfirmPairing(bool),
    PairingResult { success: bool, node_id: String, label: String },
    GetPairingStatus,
    PairingStatusResponse { 
        pin: Option<String>, 
        active: bool,
        is_initiator: bool,
        remote_label: String 
    },
    GetPairedDevices,
    PairedDevicesResponse(Vec<(String, String)>),
    UnpairDevice { node_id: String },
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
