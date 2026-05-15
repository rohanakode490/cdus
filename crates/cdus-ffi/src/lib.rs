uniffi::setup_scaffolding!();

use blake3;
use once_cell::sync::Lazy;
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

#[uniffi::export]
pub fn init_logging() {
    #[cfg(target_os = "android")]
    {
        use tracing_subscriber::prelude::*;
        use tracing_subscriber::Registry;

        let android_layer = tracing_android::layer("CDUS-Rust").ok();
        Registry::default().with(android_layer).init();
        info!("Rust logging initialized for Android");
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct DiscoveredDevice {
    pub node_id: String,
    pub label: String,
    pub os: String,
    pub ip: String,
    pub port: u16,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ClipboardHistoryItem {
    pub id: i64,
    pub content: String,
    pub source: String,
    pub timestamp: String,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct FileChunk {
    pub hash: String,
    pub offset: u64,
    pub size: u32,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct FileManifest {
    pub file_hash: String,
    pub file_name: String,
    pub total_size: u64,
    pub chunks: Vec<FileChunk>,
}

#[uniffi::export(callback_interface)]
pub trait ClipboardListener: Send + Sync {
    fn on_clipboard_update(&self, content: String, source: String);
}

#[uniffi::export(callback_interface)]
pub trait FileTransferListener: Send + Sync {
    fn on_incoming_request(&self, node_id: String, manifest: FileManifest);
    fn on_transfer_progress(&self, file_hash: String, progress: f32);
    fn on_transfer_complete(&self, file_hash: String);
    fn on_transfer_error(&self, file_hash: String, error: String);
}

static CLIPBOARD_LISTENER: Lazy<Mutex<Option<Box<dyn ClipboardListener>>>> =
    Lazy::new(|| Mutex::new(None));
static FILE_TRANSFER_LISTENER: Lazy<Mutex<Option<Box<dyn FileTransferListener>>>> =
    Lazy::new(|| Mutex::new(None));

#[uniffi::export]
pub fn set_clipboard_listener(listener: Box<dyn ClipboardListener>) {
    *CLIPBOARD_LISTENER.lock().unwrap() = Some(listener);
}

#[uniffi::export]
pub fn set_file_transfer_listener(listener: Box<dyn FileTransferListener>) {
    *FILE_TRANSFER_LISTENER.lock().unwrap() = Some(listener);
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct PairingStatus {
    pub active: bool,
    pub pin: String,
    pub remote_label: String,
    pub is_initiator: bool,
}

// Global instances for mobile use
static MDNS: Lazy<Arc<cdus_agent::mdns::MdnsManager>> =
    Lazy::new(|| Arc::new(cdus_agent::mdns::MdnsManager::new()));

static DISCOVERED: Lazy<Arc<Mutex<Vec<DiscoveredDevice>>>> =
    Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

static LOCAL_NODE_ID: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

static ACTIVE_PAIRING: Lazy<Arc<Mutex<Option<cdus_agent::pairing::ActivePairingState>>>> =
    Lazy::new(|| Arc::new(Mutex::new(None)));

static PAIRING_MANAGER: Lazy<Mutex<Option<Arc<cdus_agent::pairing::PairingManager>>>> =
    Lazy::new(|| Mutex::new(None));

static LIBP2P_MANAGER: Lazy<Mutex<Option<Arc<cdus_agent::libp2p_manager::Libp2pManager>>>> =
    Lazy::new(|| Mutex::new(None));

static ACTIVE_TRANSFERS: Lazy<
    Arc<Mutex<std::collections::HashMap<String, (std::path::PathBuf, cdus_common::FileManifest)>>>,
> = Lazy::new(|| Arc::new(Mutex::new(std::collections::HashMap::new())));

static RECEIVED_MANIFESTS: Lazy<
    Arc<Mutex<std::collections::HashMap<String, cdus_common::TransferProgress>>>,
> = Lazy::new(|| Arc::new(Mutex::new(std::collections::HashMap::new())));

static STORE: Lazy<Mutex<Option<Arc<cdus_agent::store::Store>>>> = Lazy::new(|| Mutex::new(None));

static AGENT_TX: Lazy<Mutex<Option<flume::Sender<cdus_common::IpcMessage>>>> =
    Lazy::new(|| Mutex::new(None));

#[uniffi::export]
pub fn init_core(data_dir: String, device_name: String) -> String {
    let path = std::path::Path::new(&data_dir);
    if let Err(e) = std::fs::create_dir_all(path) {
        return format!("error:Failed to create directory: {}", e);
    }

    match cdus_agent::store::Store::init(path) {
        Ok(store) => {
            let store = Arc::new(store);
            *STORE.lock().unwrap() = Some(Arc::clone(&store));

            // Set device name if provided
            if !device_name.is_empty() {
                let _ = store.set_state("device_name", &device_name);
            }

            match store.get_or_create_identity(path) {
                Ok((node_id, private_key)) => {
                    *LOCAL_NODE_ID.lock().unwrap() = node_id.clone();
                    let label = store
                        .get_state("device_name")
                        .unwrap_or(None)
                        .unwrap_or_else(|| "Android Device".to_string());

                    let (tx, rx) = flume::unbounded();
                    *AGENT_TX.lock().unwrap() = Some(tx.clone());

                    // Initialize Relay Manager with a default URL for now
                    let relay_url = if cfg!(target_os = "android") {
                        "http://10.0.2.2:8080".to_string()
                    } else {
                        "http://localhost:8080".to_string()
                    };
                    let (relay, relay_rx) = cdus_agent::relay::RelayManager::new(
                        node_id.clone(),
                        relay_url,
                        tx.clone(),
                    );
                    let relay = Arc::new(relay);
                    relay.start_signaling_loop(relay_rx);

                    // Initialize Turn Manager
                    let turn = Arc::new(
                        cdus_agent::turn_manager::TurnManager::new()
                            .expect("Failed to init TurnManager"),
                    );

                    // Initialize Libp2p Manager
                    let libp2p_manager = Arc::new(
                        cdus_agent::libp2p_manager::Libp2pManager::new(
                            private_key.clone(),
                            tx.clone(),
                            Arc::clone(&store),
                            Arc::clone(&ACTIVE_TRANSFERS),
                            Arc::clone(&RECEIVED_MANIFESTS),
                        )
                        .expect("Failed to initialize Libp2pManager"),
                    );
                    libp2p_manager.start();
                    let libp2p_sync_tx = libp2p_manager.get_sync_tx();
                    *LIBP2P_MANAGER.lock().unwrap() = Some(Arc::clone(&libp2p_manager));

                    let sync_manager = Arc::new(cdus_agent::pairing::SyncManager::new());
                    sync_manager.add_peer(
                        "libp2p_broadcast".to_string(),
                        libp2p_sync_tx,
                        cdus_common::TransportType::P2p,
                    );

                    // Setup Pairing Manager
                    let pm = cdus_agent::pairing::PairingManager::new(
                        Arc::clone(&store),
                        tx.clone(),
                        node_id.clone(),
                        private_key,
                        5200, // Default port
                        Arc::clone(&ACTIVE_PAIRING),
                        sync_manager,
                        relay,
                        turn,
                    );
                    let pm = Arc::new(pm);
                    let pm_clone = Arc::clone(&pm);
                    std::thread::spawn(move || {
                        pm_clone.start_listener();
                    });

                    *PAIRING_MANAGER.lock().unwrap() = Some(pm);

                    let libp2p_request_tx = libp2p_manager.get_request_tx();

                    // Background thread to drain messages and notify listeners
                    std::thread::spawn(move || {
                        while let Ok(msg) = rx.recv() {
                            match msg {
                                cdus_common::IpcMessage::SendFile { node_id, path } => {
                                    if let Ok(peer_id) = node_id.parse::<libp2p::PeerId>() {
                                        if let Ok(manifest) =
                                            cdus_agent::file_transfer::generate_manifest(
                                                std::path::Path::new(&path),
                                            )
                                        {
                                            let mut at = ACTIVE_TRANSFERS.lock().unwrap();
                                            at.insert(
                                                manifest.file_hash.clone(),
                                                (std::path::PathBuf::from(path), manifest.clone()),
                                            );
                                            let _ = libp2p_request_tx.send((
                                                peer_id,
                                                cdus_common::SyncMessage::FileTransferRequest(
                                                    manifest,
                                                ),
                                            ));
                                        }
                                    }
                                }
                                cdus_common::IpcMessage::AcceptFileTransfer { file_hash } => {
                                    let progress = {
                                        let rm = RECEIVED_MANIFESTS.lock().unwrap();
                                        rm.get(&file_hash)
                                            .map(|p| (p.node_id.clone(), p.manifest.clone()))
                                    };

                                    if let Some((node_id, manifest)) = progress {
                                        if let Some(pm) = PAIRING_MANAGER.lock().unwrap().as_ref() {
                                            pm.sync_manager.broadcast(
                                                cdus_common::SyncMessage::FileTransferAccepted {
                                                    file_hash: file_hash.clone(),
                                                },
                                            );
                                        }

                                        if let Ok(peer_id) = node_id.parse::<libp2p::PeerId>() {
                                            let req_tx_clone = libp2p_request_tx.clone();
                                            std::thread::spawn(move || {
                                                for chunk in manifest.chunks {
                                                    let _ = req_tx_clone.send((
                                                        peer_id,
                                                        cdus_common::SyncMessage::ChunkRequest {
                                                            file_hash: file_hash.clone(),
                                                            chunk_hash: chunk.hash.clone(),
                                                        },
                                                    ));
                                                }
                                            });
                                        }
                                    }
                                }
                                cdus_common::IpcMessage::RejectFileTransfer { file_hash } => {
                                    if let Some(pm) = PAIRING_MANAGER.lock().unwrap().as_ref() {
                                        pm.sync_manager.broadcast(
                                            cdus_common::SyncMessage::FileTransferRejected {
                                                file_hash,
                                            },
                                        );
                                    }
                                }
                                cdus_common::IpcMessage::ChunkReceived {
                                    file_hash,
                                    chunk_hash,
                                    data,
                                } => {
                                    let mut rm = RECEIVED_MANIFESTS.lock().unwrap();
                                    if let Some(progress) = rm.get_mut(&file_hash) {
                                        if let Some(chunk) = progress
                                            .manifest
                                            .chunks
                                            .iter()
                                            .find(|c| c.hash == chunk_hash)
                                        {
                                            let actual_hash = blake3::hash(&data).to_string();
                                            if actual_hash == chunk_hash {
                                                // Save chunk to disk
                                                // For mobile, we'll save in the data dir
                                                let mut path = std::path::PathBuf::from(&data_dir);
                                                path.push(format!("{}.part", file_hash));

                                                if let Ok(mut file) = std::fs::OpenOptions::new()
                                                    .create(true)
                                                    .write(true)
                                                    .open(&path)
                                                {
                                                    use std::io::{Seek, Write};
                                                    let _ = file.seek(std::io::SeekFrom::Start(
                                                        chunk.offset,
                                                    ));
                                                    let _ = file.write_all(&data);

                                                    progress.completed_hashes.insert(chunk_hash);

                                                    let total = progress.manifest.chunks.len();
                                                    let completed = progress.completed_hashes.len();
                                                    let percent =
                                                        (completed as f32 / total as f32) * 100.0;

                                                    if let Some(tx) =
                                                        AGENT_TX.lock().unwrap().as_ref()
                                                    {
                                                        let _ = tx.send(cdus_common::IpcMessage::FileTransferProgress { file_hash: file_hash.clone(), progress: percent });
                                                        if completed == total {
                                                            // Move to final destination
                                                            let mut final_path =
                                                                std::path::PathBuf::from(&data_dir);
                                                            final_path
                                                                .push(&progress.manifest.file_name);
                                                            let _ =
                                                                std::fs::rename(path, final_path);
                                                            let _ = tx.send(cdus_common::IpcMessage::FileTransferComplete { file_hash: file_hash.clone() });
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                cdus_common::IpcMessage::SetClipboard {
                                    content, source, ..
                                } => {
                                    if let Some(listener) =
                                        CLIPBOARD_LISTENER.lock().unwrap().as_ref()
                                    {
                                        listener.on_clipboard_update(content, source);
                                    }
                                }
                                cdus_common::IpcMessage::IncomingFileRequest {
                                    node_id,
                                    manifest,
                                } => {
                                    if let Some(listener) =
                                        FILE_TRANSFER_LISTENER.lock().unwrap().as_ref()
                                    {
                                        let ffi_manifest = FileManifest {
                                            file_hash: manifest.file_hash,
                                            file_name: manifest.file_name,
                                            total_size: manifest.total_size,
                                            chunks: manifest
                                                .chunks
                                                .into_iter()
                                                .map(|c| FileChunk {
                                                    hash: c.hash,
                                                    offset: c.offset,
                                                    size: c.size,
                                                })
                                                .collect(),
                                        };
                                        listener.on_incoming_request(node_id, ffi_manifest);
                                    }
                                }
                                cdus_common::IpcMessage::FileTransferProgress {
                                    file_hash,
                                    progress,
                                } => {
                                    if let Some(listener) =
                                        FILE_TRANSFER_LISTENER.lock().unwrap().as_ref()
                                    {
                                        listener.on_transfer_progress(file_hash, progress);
                                    }
                                }
                                cdus_common::IpcMessage::FileTransferComplete { file_hash } => {
                                    if let Some(listener) =
                                        FILE_TRANSFER_LISTENER.lock().unwrap().as_ref()
                                    {
                                        listener.on_transfer_complete(file_hash);
                                    }
                                }
                                cdus_common::IpcMessage::FileTransferError { file_hash, error } => {
                                    if let Some(listener) =
                                        FILE_TRANSFER_LISTENER.lock().unwrap().as_ref()
                                    {
                                        listener.on_transfer_error(file_hash, error);
                                    }
                                }
                                _ => {
                                    info!("FFI Core: Received IPC message: {:?}", msg);
                                }
                            }
                        }
                    });

                    format!("{}:{}", node_id, label)
                }
                Err(e) => format!("error:Failed to get identity: {}", e),
            }
        }
        Err(e) => format!("error:Failed to init store: {}", e),
    }
}

#[uniffi::export]
pub fn send_file(node_id: String, path: String) {
    if let Some(tx) = AGENT_TX.lock().unwrap().as_ref() {
        let _ = tx.send(cdus_common::IpcMessage::SendFile { node_id, path });
    }
}

#[uniffi::export]
pub fn accept_file_transfer(file_hash: String) {
    if let Some(tx) = AGENT_TX.lock().unwrap().as_ref() {
        let _ = tx.send(cdus_common::IpcMessage::AcceptFileTransfer { file_hash });
    }
}

#[uniffi::export]
pub fn reject_file_transfer(file_hash: String) {
    if let Some(tx) = AGENT_TX.lock().unwrap().as_ref() {
        let _ = tx.send(cdus_common::IpcMessage::RejectFileTransfer { file_hash });
    }
}

#[uniffi::export]
pub fn get_clipboard_history(limit: u32) -> Vec<ClipboardHistoryItem> {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        match store.get_recent_events(limit) {
            Ok(events) => events
                .into_iter()
                .map(|e| ClipboardHistoryItem {
                    id: e.id,
                    content: e.content,
                    source: e.source,
                    timestamp: e.timestamp,
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    }
}

#[uniffi::export]
pub fn broadcast_clipboard(content: String) {
    info!("FFI: broadcast_clipboard called (len={})", content.len());
    if let Some(pm) = PAIRING_MANAGER.lock().unwrap().as_ref() {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Save to local store first
        if let Some(store) = STORE.lock().unwrap().as_ref() {
            if let Err(e) = store.append_event(content.as_bytes(), "Local") {
                error!("FFI: Failed to append local event to store: {}", e);
            } else {
                info!("FFI: Appended local clipboard event to store");
            }
        }

        pm.sync_manager
            .broadcast(cdus_common::SyncMessage::ClipboardUpdate { content, timestamp });
        info!("FFI: Broadcasted clipboard update to connected peers");
    } else {
        warn!("FFI: broadcast_clipboard called but PAIRING_MANAGER is None");
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct PairedDevice {
    pub node_id: String,
    pub label: String,
}

#[uniffi::export]
pub fn get_paired_devices() -> Vec<PairedDevice> {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        match store.get_paired_devices() {
            Ok(devices) => devices
                .into_iter()
                .map(|(node_id, label)| PairedDevice { node_id, label })
                .collect(),
            Err(e) => {
                error!("FFI: Failed to get paired devices: {}", e);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    }
}

#[uniffi::export]
pub fn unpair_device(node_id: String) {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        if let Err(e) = store.remove_paired_device(&node_id) {
            error!("FFI: Failed to unpair device {}: {}", node_id, e);
        }
    }
}

#[uniffi::export]
pub fn get_pairing_status() -> Option<PairingStatus> {
    let ap = ACTIVE_PAIRING.lock().unwrap();
    ap.as_ref().map(|s| PairingStatus {
        active: true,
        pin: s.pin.clone(),
        remote_label: s.remote_label.clone(),
        is_initiator: s.is_initiator,
    })
}

static RECV_THREAD_STARTED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

#[uniffi::export]
pub fn initiate_pairing(node_id: String) {
    info!("FFI: initiate_pairing called for node_id: {}", node_id);
    if let Some(pm) = PAIRING_MANAGER.lock().unwrap().as_ref() {
        let discovered = DISCOVERED.lock().unwrap();
        info!(
            "FFI: Current discovered devices count: {}",
            discovered.len()
        );
        if let Some(device) = discovered.iter().find(|d| d.node_id == node_id) {
            info!(
                "FFI: Found device in list, starting pairing thread for {}",
                device.ip
            );
            if let Ok(ip_addr) = device.ip.parse() {
                let addr = std::net::SocketAddr::new(ip_addr, device.port);
                let pm_clone = Arc::clone(pm);
                std::thread::spawn(move || {
                    info!("FFI: Initiating pairing with {} at {}", node_id, addr);
                    pm_clone.initiate_pairing(addr);
                });
            } else {
                error!("FFI: Failed to parse IP: {}", device.ip);
            }
        } else {
            warn!(
                "FFI: Device {} not found in discovered list. Current list: {:?}",
                node_id, discovered
            );
        }
    } else {
        error!("FFI: PAIRING_MANAGER is None!");
    }
}

#[uniffi::export]
pub fn confirm_pairing(accepted: bool) {
    info!("FFI: confirm_pairing called with: {}", accepted);
    let ap = ACTIVE_PAIRING.lock().unwrap();
    if let Some(state) = ap.as_ref() {
        let mut confirmed = state.confirmed.lock().unwrap();
        *confirmed = Some(accepted);
        info!(
            "FFI: Pairing {} by user",
            if accepted { "confirmed" } else { "declined" }
        );
    } else {
        warn!("FFI: confirm_pairing called but no active pairing state found");
    }
}

#[uniffi::export]
pub fn cancel_pairing() {
    info!("FFI: cancel_pairing called");
    let mut ap = ACTIVE_PAIRING.lock().unwrap();
    *ap = None;
    info!("FFI: Pairing cancelled/cleared");
}

#[uniffi::export]
pub fn register_device(node_id: String, label: String, port: u16) {
    info!(
        "FFI: Registering device: {} ({}) on port {}",
        node_id, label, port
    );
    MDNS.register_device(&node_id, &label, port);
}

#[uniffi::export]
pub fn start_discovery() {
    info!("FFI: Starting mDNS discovery");
    let (tx, rx) = flume::unbounded();

    let discovered_clone = Arc::clone(&DISCOVERED);
    let local_id = LOCAL_NODE_ID.lock().unwrap().clone();

    {
        let mut started = RECV_THREAD_STARTED.lock().unwrap();
        if !*started {
            spawn_mdns_receiver(rx, discovered_clone, local_id);
            *started = true;
            info!("FFI: mDNS receiver thread spawned");
        } else {
            // If already started, we need to handle the new rx or just use the old one.
            // Actually, MdnsManager::start_discovery takes a tx.
            // If we already have a thread, it's listening to an OLD rx.
            // Let's refactor spawn_mdns_receiver to be more robust or just always spawn for now but log it.
            spawn_mdns_receiver(rx, discovered_clone, local_id);
            info!("FFI: mDNS receiver thread spawned (additional)");
        }
    }

    MDNS.start_discovery(tx);
}

#[uniffi::export]
pub fn stop_discovery() {
    info!("FFI: Stopping mDNS discovery");
    MDNS.stop_discovery();
}

// Internal bridge for mDNS events to FFI records
fn spawn_mdns_receiver(
    rx: flume::Receiver<cdus_common::IpcMessage>,
    discovered: Arc<Mutex<Vec<DiscoveredDevice>>>,
    local_id: String,
) {
    std::thread::spawn(move || {
        while let Ok(msg) = rx.recv() {
            match msg {
                cdus_common::IpcMessage::DeviceDiscovered {
                    node_id,
                    label,
                    os,
                    ip,
                    port,
                } => {
                    if !local_id.is_empty() && node_id == local_id {
                        continue;
                    }

                    let mut list = discovered.lock().unwrap();
                    if !list.iter().any(|d| d.node_id == node_id) {
                        info!(
                            "FFI: Discovered device: {} ({}) at {}:{}",
                            label, node_id, ip, port
                        );
                        list.push(DiscoveredDevice {
                            node_id,
                            label,
                            os,
                            ip,
                            port,
                        });
                    }
                }
                cdus_common::IpcMessage::DeviceLost { node_id } => {
                    let mut list = discovered.lock().unwrap();
                    list.retain(|d| !d.node_id.starts_with(&node_id));
                    info!("FFI: Removed device: {} (prefix)", node_id);
                }
                _ => {}
            }
        }
    });
}

#[uniffi::export]
pub fn get_discovered_devices() -> Vec<DiscoveredDevice> {
    DISCOVERED.lock().unwrap().clone()
}

#[uniffi::export]
pub fn clear_discovered_devices() {
    DISCOVERED.lock().unwrap().clear();
}

#[uniffi::export]
#[tracing::instrument]
pub fn greet_from_rust(name: String) -> String {
    info!("Greeting requested for: {}", name);
    format!(
        "Hello, {}! This is CDUS core running on Android via Rust.",
        name
    )
}
