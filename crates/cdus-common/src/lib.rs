use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct ClipboardEvent {
    pub id: i64,
    pub content: String,
    pub source: String, // "Local" or remote device name
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum TransportType {
    Lan,
    P2p,
    Relay,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum IpcMessage {
    Ping,
    Pong,
    Log(String),
    ClipboardChanged {
        content: String,
        timestamp: u64,
    },
    SetClipboard {
        content: String,
        timestamp: u64,
        source: String,
    },
    GetHistory {
        limit: u32,
    },
    HistoryResponse(Vec<ClipboardEvent>),
    GetState {
        key: String,
    },
    SetState {
        key: String,
        value: String,
    },
    StateResponse(Option<String>),
    StartScan,
    StopScan,
    DeviceDiscovered {
        node_id: String,
        label: String,
        os: String,
        ip: String,
        port: u16,
    },
    DeviceLost {
        node_id: String,
    },
    PeerDisconnected {
        node_id: String,
    },
    GetDiscovered,
    DiscoveredResponse(Vec<(String, String, String, String, u16)>),
    PairWith {
        node_id: String,
    },
    PairWithIp {
        ip: String,
        port: u16,
    },
    PairWithRemote {
        uuid: String,
    },
    PairingPin(String),
    ConfirmPairing(bool),
    PairingResult {
        success: bool,
        node_id: String,
        label: String,
    },
    GetPairingStatus,
    PairingStatusResponse {
        pin: Option<String>,
        active: bool,
        is_initiator: bool,
        remote_label: String,
    },
    GetPairedDevices,
    PairedDevicesResponse(Vec<(String, String, Option<TransportType>)>),
    UnpairDevice {
        node_id: String,
    },
    RevokeDevice {
        uuid: String,
    },
    RelayMessage {
        source_uuid: String,
        payload: Vec<u8>,
    },
    ConnectRelay,
    ListenEvents,
    // File Transfer
    SendFile {
        node_id: String,
        path: String,
    },
    AcceptFileTransfer {
        transfer_id: String,
    },
    RejectFileTransfer {
        transfer_id: String,
    },
    CancelFileTransfer {
        transfer_id: String,
    },
    FileTransferProgress {
        transfer_id: String,
        progress: f32,
    },
    FileTransferComplete {
        transfer_id: String,
    },
    FileTransferError {
        transfer_id: String,
        error: String,
    },
    // New File Transfer IPC
    FileProgress(ProgressEvent),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum ProgressEvent {
    Started {
        transfer_id: String,
        total_bytes: u64,
        is_outgoing: bool,
    },
    Progress {
        transfer_id: String,
        bytes_confirmed: u64,
    },
    Complete {
        transfer_id: String,
        dest_path: std::path::PathBuf,
    },
    Failed {
        transfer_id: String,
        reason: String,
    },
    IncomingRequest {
        transfer_id: String,
        file_name: String,
        total_bytes: u64,
        sender_label: String,
    },
}

// --- New File Transfer Protocol (Phase 2) ---

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TransferRequest {
    pub transfer_id:  String,   // UUID
    pub file_name:    String,   // display name only
    pub total_bytes:  u64,
    pub chunk_size:   u32,      // sender's preferred chunk size
    pub file_hash:    String,   // BLAKE3 hex of whole file
    pub sender_label: String,   // "Rahul's Laptop" — shown in accept dialog
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TransferAcceptance {
    pub transfer_id:  String,
    pub accepted:     bool,
    pub resume_from:  u64,      // byte offset — 0 for fresh, N for resume
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ChunkFrame {
    pub transfer_id:  String,
    pub chunk_index:  u32,
    pub byte_offset:  u64,
    #[serde(with = "serde_bytes")]
    pub data:         Vec<u8>,  // encrypted chunk payload
    pub chunk_hash:   String,   // BLAKE3 of plaintext (verify after decrypt)
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ChunkAck {
    pub transfer_id:  String,
    pub chunk_index:  u32,
    pub bytes_confirmed: u64,   // cumulative confirmed offset
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TransferComplete {
    pub transfer_id:  String,
    pub file_hash:    String,   // BLAKE3 of whole file — receiver verifies
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TransferError {
    pub transfer_id:  String,
    pub reason:       String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type")]
pub enum FileMessage {
    Request(TransferRequest),
    Acceptance(TransferAcceptance),
    Chunk(ChunkFrame),
    Ack(ChunkAck),
    Complete(TransferComplete),
    Error(TransferError),
    Cancel { transfer_id: String },
}

impl FileMessage {
    pub fn to_vec(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        rmp_serde::to_vec(self)
    }

    pub fn from_slice(slice: &[u8]) -> Result<Self, rmp_serde::decode::Error> {
        rmp_serde::from_slice(slice)
    }
}

#[derive(Debug, Clone)]
pub struct TransferProgress {
    pub node_id: String,
    pub completed_hashes: std::collections::HashSet<String>,
    pub accepted: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum SyncMessage {
    ClipboardUpdate {
        content: String,
        timestamp: u64,
    },
}

impl SyncMessage {
    pub fn to_vec(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        rmp_serde::to_vec(self)
    }

    pub fn from_slice(slice: &[u8]) -> Result<Self, rmp_serde::decode::Error> {
        rmp_serde::from_slice(slice)
    }
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
        msg.serialize(&mut rmp_serde::Serializer::new(&mut buf))
            .unwrap();

        let mut de = rmp_serde::Deserializer::new(&buf[..]);
        let deserialized: IpcMessage = Deserialize::deserialize(&mut de).unwrap();
        assert_eq!(msg, deserialized);
    }
}
