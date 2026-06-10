use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct ClipboardEvent {
    pub id: i64,
    pub content: String,
    pub source: String, // "Local" or remote device name
    pub timestamp: String,
    pub is_sensitive: bool,
}

pub fn is_sensitive_content(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        return false;
    }
    
    // Heuristic: If it looks like a password (mix of uppercase, lowercase, numbers/symbols, length 8-64)
    if trimmed.len() < 8 || trimmed.len() > 64 {
        return false;
    }
    
    let has_upper = trimmed.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = trimmed.chars().any(|c| c.is_ascii_lowercase());
    let has_digit = trimmed.chars().any(|c| c.is_ascii_digit());
    let has_symbol = trimmed.chars().any(|c| c.is_ascii_punctuation() || "@#$%-+=_*^&".contains(c));
    
    has_upper && has_lower && (has_digit || has_symbol)
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
    DeleteHistoryItem {
        id: i64,
    },
    ClearHistory,
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
        ips: Vec<String>,
        port: u16,
    },
    DeviceLost {
        node_id: String,
    },
    PeerDisconnected {
        node_id: String,
    },
    PeerConnected {
        node_id: String,
    },
    GetDiscovered,
    DiscoveredResponse(Vec<(String, String, String, Vec<String>, u16)>),
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
    AlreadyPaired {
        node_id: String,
        label: String,
    },
    StalePairing {
        node_id: String,
        label: String,
    },
    GetPairingStatus,
    PairingStatusResponse {
        pin: Option<String>,
        active: bool,
        is_initiator: bool,
        remote_label: String,
        silent: bool,
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
    // QR / OOB Pairing
    GetQrPairingPayload,
    QrPairingPayloadResponse {
        payload: String,
    },
    PairWithQr {
        payload: String,
    },
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
    SimulateCrash {
        transfer_id: String,
    },
    SetCrashTrigger {
        transfer_id: String,
        offset: u64,
    },
    ResumeFileTransfer {
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
    StartBenchmark {
        node_id: String,
    },
    GetFileTransferHistory {
        limit: u32,
    },
    FileTransferHistoryResponse(Vec<FileTransferRecord>),
    ClearFinishedTransfers,
    // New File Transfer IPC
    FileProgress(ProgressEvent),
    // Testing
    TestLibp2pRequest {
        peer_id: String,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct FileTransferRecord {
    pub transfer_id: String,
    pub direction: String,
    pub peer_node_id: String,
    pub file_path: String,
    pub file_name: String,
    pub total_bytes: u64,
    pub bytes_confirmed: u64,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum ProgressEvent {
    Started {
        transfer_id: String,
        file_name: String,
        total_bytes: u64,
        is_outgoing: bool,
    },
    Progress {
        transfer_id: String,
        bytes_confirmed: u64,
        total_bytes: u64,
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
        node_id: String,
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
    pub transfer_id:    String,
    pub accepted:       bool,
    pub resume_from:    u64,           // byte offset — 0 for fresh, N for resume
    pub missing_chunks: Option<Vec<u32>>, // Specific chunk indices if sparse resume is needed
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
pub struct PeerExchangeRecord {
    pub node_id: String,
    pub addresses: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum SyncMessage {
    ClipboardUpdate {
        content: String,
        timestamp: u64,
    },
    PeerExchange {
        peers: Vec<PeerExchangeRecord>,
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
