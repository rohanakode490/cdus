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
    ListenEvents,
    // File Transfer
    SendFile {
        node_id: String,
        path: String,
    },
    IncomingFileRequest {
        node_id: String,
        manifest: FileManifest,
    },
    AcceptFileTransfer {
        file_hash: String,
    },
    RejectFileTransfer {
        file_hash: String,
    },
    FileTransferProgress {
        file_hash: String,
        progress: f32,
    },
    FileTransferComplete {
        file_hash: String,
    },
    FileTransferError {
        file_hash: String,
        error: String,
    },
    ChunkReceived {
        file_hash: String,
        chunk_hash: String,
        data: Vec<u8>,
    },
    ChunkServed {
        file_hash: String,
        chunk_hash: String,
    },
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct FileChunk {
    pub hash: String,
    pub offset: u64,
    pub size: u32,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct FileManifest {
    pub file_hash: String,
    pub file_name: String,
    pub total_size: u64,
    pub chunks: Vec<FileChunk>,
}

#[derive(Debug, Clone)]
pub struct TransferProgress {
    pub node_id: String,
    pub manifest: FileManifest,
    pub completed_hashes: std::collections::HashSet<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum SyncMessage {
    ClipboardUpdate {
        content: String,
        timestamp: u64,
    },
    FileTransferRequest(FileManifest),
    FileTransferAccepted {
        file_hash: String,
    },
    FileTransferRejected {
        file_hash: String,
    },
    ChunkRequest {
        file_hash: String,
        chunk_hash: String,
    },
    ChunkResponse {
        file_hash: String,
        chunk_hash: String,
        data: Vec<u8>,
    },
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
