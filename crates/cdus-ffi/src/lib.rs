uniffi::setup_scaffolding!();

use once_cell::sync::Lazy;
use std::sync::{Arc, Mutex};
use tracing::{error, info};
use cdus_common::{IpcMessage, SyncMessage, ProgressEvent};
use cdus_agent::file_transfer::FileTransferManager;

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
    pub ips: Vec<String>,
    pub port: u16,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct FfiNotificationPayload {
    pub key: String,
    pub package_name: String,
    pub app_name: String,
    pub title: String,
    pub text: String,
    pub timestamp: u64,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ClipboardHistoryItem {
    pub id: i64,
    pub content: String,
    pub source: String,
    pub timestamp: String,
    pub is_sensitive: bool,
    pub local_only: bool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct AuditLogItem {
    pub id: i64,
    pub event_type: String,
    pub content: String,
    pub timestamp: u64,
}

#[uniffi::export(callback_interface)]
pub trait ClipboardListener: Send + Sync {
    fn on_clipboard_update(&self, content: String, source: String);
}

#[uniffi::export(callback_interface)]
pub trait FileTransferListener: Send + Sync {
    fn on_incoming_request(&self, node_id: String, transfer_id: String, file_name: String, total_bytes: u64, sender_label: String);
    fn on_incoming_transfer_started(&self, transfer_id: String, file_name: String, total_bytes: u64);
    fn on_outgoing_transfer_started(&self, transfer_id: String, file_name: String, total_bytes: u64);
    fn on_transfer_progress(&self, transfer_id: String, progress: f32);
    fn on_transfer_complete(&self, transfer_id: String, dest_path: String);
    fn on_transfer_error(&self, transfer_id: String, error: String);
    fn on_peer_accepted(&self, node_id: String, transfer_id: String);
    fn on_peer_rejected(&self, node_id: String, transfer_id: String);
    fn on_peer_disconnected(&self, node_id: String);
    fn on_peer_connected(&self, node_id: String);
    fn on_pairing_result(&self, success: bool, node_id: String, label: String, error: Option<String>);
    fn on_already_paired(&self, node_id: String, label: String);
    fn on_stale_pairing(&self, node_id: String, label: String);
    fn on_transfer_state_changed(&self, transfer_id: String, state: String);
    fn on_relay_status_changed(&self, connected: bool, error: Option<String>);
}

static CLIPBOARD_LISTENER: Lazy<Mutex<Option<Box<dyn ClipboardListener>>>> =
    Lazy::new(|| Mutex::new(None));
static FILE_TRANSFER_LISTENER: Lazy<Mutex<Option<Box<dyn FileTransferListener>>>> =
    Lazy::new(|| Mutex::new(None));

#[uniffi::export(callback_interface)]
pub trait NotificationListener: Send + Sync {
    fn on_remote_dismiss_request(&self, key: String);
}

static NOTIFICATION_LISTENER: Lazy<Mutex<Option<Box<dyn NotificationListener>>>> =
    Lazy::new(|| Mutex::new(None));

#[uniffi::export]
pub fn set_notification_listener(listener: Box<dyn NotificationListener>) {
    *NOTIFICATION_LISTENER.lock().unwrap() = Some(listener);
}

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
    pub silent: bool,
}

// Global instances for mobile use
static MDNS: Lazy<Arc<cdus_agent::mdns::MdnsManager>> =
    Lazy::new(|| Arc::new(cdus_agent::mdns::MdnsManager::new()));

static DISCOVERED: Lazy<Arc<Mutex<Vec<DiscoveredDevice>>>> =
    Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

static PEER_MAP: Lazy<Arc<Mutex<std::collections::HashMap<String, (DiscoveredDevice, std::time::Instant)>>>> =
    Lazy::new(|| Arc::new(Mutex::new(std::collections::HashMap::new())));

static LOCAL_NODE_ID: Lazy<std::sync::Mutex<String>> = Lazy::new(|| std::sync::Mutex::new(String::new()));

static ACTIVE_PAIRING: Lazy<Arc<parking_lot::Mutex<Option<cdus_agent::pairing::ActivePairingState>>>> =
    Lazy::new(|| Arc::new(parking_lot::Mutex::new(None)));

static PAIRING_MANAGER: Lazy<std::sync::Mutex<Option<Arc<cdus_agent::pairing::PairingManager>>>> =
    Lazy::new(|| std::sync::Mutex::new(None));

static LIBP2P_MANAGER: Lazy<Mutex<Option<Arc<cdus_agent::libp2p_manager::Libp2pManager>>>> =
    Lazy::new(|| Mutex::new(None));

static TRANSFER_MANAGER: Lazy<Mutex<Option<Arc<FileTransferManager>>>> =
    Lazy::new(|| Mutex::new(None));

static STORE: Lazy<Mutex<Option<Arc<cdus_agent::store::Store>>>> = Lazy::new(|| Mutex::new(None));

static RELAY_MANAGER: Lazy<Mutex<Option<Arc<cdus_agent::relay::RelayManager>>>> =
    Lazy::new(|| Mutex::new(None));
static RELAY_RX: Lazy<Mutex<Option<flume::Receiver<cdus_agent::relay::SignalMessage>>>> =
    Lazy::new(|| Mutex::new(None));

static AGENT_TX: Lazy<Mutex<Option<flume::Sender<IpcMessage>>>> =
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

                    // Populate PEER_MAP from store for paired devices to enable reconnection after restart
                    if let Ok(paired_devices) = store.get_paired_devices() {
                        let mut map = PEER_MAP.lock().unwrap();
                        for record in paired_devices {
                            if let (Some(ips), Some(port)) =
                                (record.last_known_ips, record.last_known_port)
                            {
                                let device = DiscoveredDevice {
                                    node_id: record.node_id.clone(),
                                    label: record.label,
                                    os: "Unknown".to_string(),
                                    ips,
                                    port,
                                };
                                map.insert(
                                    record.node_id,
                                    (
                                        device,
                                        std::time::Instant::now()
                                            - std::time::Duration::from_secs(3600),
                                    ),
                                );
                            }
                        }
                    }

                    let label = store

                        .get_state("device_name")
                        .unwrap_or(None)
                        .unwrap_or_else(|| "Android Device".to_string());
                    
                    let (tx, rx) = flume::unbounded();
                    *AGENT_TX.lock().unwrap() = Some(tx.clone());

                    let (p_tx, p_rx) = flume::unbounded();
                    let transfer_manager = Arc::new(FileTransferManager::new(Arc::clone(&store), p_tx));
                    *TRANSFER_MANAGER.lock().unwrap() = Some(Arc::clone(&transfer_manager));

                    // Forward progress events to listeners
                    std::thread::spawn(move || {
                        while let Ok(event) = p_rx.recv() {
                            if let Some(listener) = FILE_TRANSFER_LISTENER.lock().unwrap().as_ref() {
                                match event {
                                    ProgressEvent::Started { transfer_id, file_name, total_bytes, is_outgoing } => {
                                        if is_outgoing {
                                            listener.on_outgoing_transfer_started(transfer_id.clone(), file_name, total_bytes);
                                        } else {
                                            listener.on_incoming_transfer_started(transfer_id.clone(), file_name, total_bytes);
                                        }
                                        listener.on_transfer_state_changed(transfer_id, "started".to_string());
                                    }
                                    ProgressEvent::Progress { transfer_id, bytes_confirmed, total_bytes } => {
                                        let progress = if total_bytes > 0 {
                                            (bytes_confirmed as f32 / total_bytes as f32) * 100.0
                                        } else {
                                            0.0
                                        };
                                        listener.on_transfer_progress(transfer_id, progress);
                                    }
                                    ProgressEvent::Complete { transfer_id, dest_path } => {
                                        listener.on_transfer_complete(transfer_id.clone(), dest_path.to_string_lossy().to_string());
                                        listener.on_transfer_state_changed(transfer_id, "completed".to_string());
                                    }
                                    ProgressEvent::Failed { transfer_id, reason } => {
                                        listener.on_transfer_error(transfer_id.clone(), reason);
                                        listener.on_transfer_state_changed(transfer_id, "failed".to_string());
                                    }
                                    ProgressEvent::IncomingRequest { transfer_id, node_id, file_name, total_bytes, sender_label } => {
                                        listener.on_incoming_request(node_id, transfer_id, file_name, total_bytes, sender_label);
                                    }
                                }
                            }
                        }
                    });

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
                    *RELAY_MANAGER.lock().unwrap() = Some(Arc::clone(&relay));
                    
                    // Auto-start relay signaling loop
                    let relay_clone = Arc::clone(&relay);
                    std::thread::spawn(move || {
                        info!("FFI: Auto-connecting to relay...");
                        if let Err(e) = relay_clone.register() {
                            error!("FFI: Failed to register with relay: {}", e);
                        }
                        relay_clone.start_signaling_loop(relay_rx);
                    });

                    // Initialize Turn Manager
                    let turn = Arc::new(
                        cdus_agent::turn_manager::TurnManager::new()
                            .expect("Failed to init TurnManager"),
                    );

                    // Initialize Libp2p Manager
                    let libp2p_manager = Arc::new(
                        cdus_agent::libp2p_manager::Libp2pManager::new_with_download_dir(
                            private_key.clone(),
                            tx.clone(),
                            Arc::clone(&store),
                            Arc::clone(&transfer_manager),
                            Some(std::path::PathBuf::from(data_dir.clone())),
                        )
                        .expect("Failed to initialize Libp2pManager"),
                    );
                    let libp2p_sync_tx = libp2p_manager.get_sync_tx();
                    *LIBP2P_MANAGER.lock().unwrap() = Some(Arc::clone(&libp2p_manager));

                    let sync_manager = Arc::new(cdus_agent::pairing::SyncManager::new());
                    sync_manager.add_peer(
                        "libp2p_broadcast".to_string(),
                        libp2p_sync_tx,
                        cdus_common::TransportType::P2p,
                    );

                    libp2p_manager.start(Arc::clone(&sync_manager));

                    // Setup Pairing Manager
                    let pm = cdus_agent::pairing::PairingManager::new(
                        Arc::clone(&store),
                        tx.clone(),
                        node_id.clone(),
                        private_key,
                        5200,
                        Arc::clone(&*ACTIVE_PAIRING),
                        sync_manager,
                        relay,
                        turn,
                        Arc::clone(&libp2p_manager),
                    );

                    let pm = Arc::new(pm);
                    let pm_clone = Arc::clone(&pm);
                    std::thread::spawn(move || {
                        pm_clone.start_listener();
                    });
                    // let pm_reconnect = Arc::clone(&pm);
                    // std::thread::spawn(move || {
                    //     pm_reconnect.start_auto_reconnect_loop();
                    // });

                    *PAIRING_MANAGER.lock().unwrap() = Some(pm);

                    // Register and start mDNS discovery at startup
                    MDNS.register_device(&node_id, &label, 5200);
                    MDNS.start_discovery(tx.clone());

                    // Background thread to drain messages and notify listeners
                    std::thread::spawn(move || {
                        while let Ok(msg) = rx.recv() {
                            match msg {
                                IpcMessage::SetClipboard {
                                    content,
                                    timestamp,
                                    source,
                                } => {
                                    if content.trim().is_empty() {
                                        continue;
                                    }
                                    let mut should_apply = true;
                                    if let Some(store) = STORE.lock().unwrap().as_ref() {
                                        if let Ok(Some(last_ts_str)) = store.get_state("last_sync_timestamp") {
                                            if let Ok(last_ts) = last_ts_str.parse::<u64>() {
                                                if timestamp <= last_ts {
                                                    should_apply = false;
                                                    info!("FFI: Ignoring outdated SetClipboard request from peer (timestamp: {}, last: {})", timestamp, last_ts);
                                                }
                                            }
                                        }
                                        
                                        if should_apply {
                                            let _ = store.append_event(content.as_bytes(), &source);
                                            let _ = store.set_state("last_sync_timestamp", &timestamp.to_string());
                                            let _ = store.set_state("last_clipboard_content", &content);
                                        }
                                    }
                                    if should_apply {
                                        if let Some(listener) =
                                            CLIPBOARD_LISTENER.lock().unwrap().as_ref()
                                        {
                                            listener.on_clipboard_update(content, source);
                                        }
                                    }
                                }
                                IpcMessage::DeviceDiscovered {
                                    node_id,
                                    label,
                                    os,
                                    ips,
                                    port,
                                } => {
                                    let local_id = LOCAL_NODE_ID.lock().unwrap().clone();
                                    if !local_id.is_empty() && node_id == local_id {
                                        continue;
                                    }

                                    let device = DiscoveredDevice {
                                        node_id: node_id.clone(),
                                        label: label.clone(),
                                        os: os.clone(),
                                        ips: ips.clone(),
                                        port,
                                    };

                                    // Update global peer map (always, even if paired)
                                    {
                                        let mut map = PEER_MAP.lock().unwrap();
                                        map.insert(
                                            node_id.clone(),
                                            (device.clone(), std::time::Instant::now()),
                                        );
                                    }

                                    // Persist network info for paired devices
                                    if let Some(store) = STORE.lock().unwrap().as_ref() {
                                        if let Ok(true) = store.is_device_paired(&node_id) {
                                            let _ = store.update_paired_device_network_info(
                                                &node_id, &ips, port,
                                            );
                                        }
                                    }

                                    let mut list = DISCOVERED.lock().unwrap();
                                    if !list.iter().any(|d| d.node_id == node_id) {
                                        // If already paired, we don't add to discovered list
                                        if let Some(store) = STORE.lock().unwrap().as_ref() {
                                            if let Ok(true) = store.is_device_paired(&node_id) {
                                                continue;
                                            }
                                        }

                                        info!(
                                            "FFI: Discovered device: {} ({}) at {:?}:{}",
                                            label, node_id, ips, port
                                        );
                                        list.push(device);
                                    }
                                }
                                IpcMessage::DeviceLost { node_id } => {
                                    let mut list = DISCOVERED.lock().unwrap();
                                    list.retain(|d| !d.node_id.starts_with(&node_id));
                                }
                                IpcMessage::PeerConnected { node_id } => {
                                    {
                                        let mut list = DISCOVERED.lock().unwrap();
                                        list.retain(|d| d.node_id != node_id);
                                    }
                                    if let Some(listener) = FILE_TRANSFER_LISTENER.lock().unwrap().as_ref() {
                                        listener.on_peer_connected(node_id);
                                    }
                                }
                                IpcMessage::PeerDisconnected { node_id } => {
                                    if let Some(tm) = TRANSFER_MANAGER.lock().unwrap().as_ref() {
                                        tm.cancel_all_transfers_for_peer(&node_id);
                                    }
                                    if let Some(lm) = LIBP2P_MANAGER.lock().unwrap().as_ref() {
                                        if let Ok(peer_id) = node_id.parse::<libp2p::PeerId>() {
                                            lm.disconnect_peer(peer_id);
                                        }
                                    }
                                    if let Some(listener) = FILE_TRANSFER_LISTENER.lock().unwrap().as_ref() {
                                        listener.on_peer_disconnected(node_id);
                                    }
                                }
                                IpcMessage::PairingResult { success, node_id, label, error } => {
                                    if success {
                                        let mut list = DISCOVERED.lock().unwrap();
                                        list.retain(|d| d.node_id != node_id);
                                    }
                                    if let Some(listener) = FILE_TRANSFER_LISTENER.lock().unwrap().as_ref() {
                                        listener.on_pairing_result(success, node_id, label, error);
                                    }
                                }
                                IpcMessage::RelayStatus { connected, error } => {
                                    if let Some(listener) = FILE_TRANSFER_LISTENER.lock().unwrap().as_ref() {
                                        listener.on_relay_status_changed(connected, error);
                                    }
                                }
                                IpcMessage::AlreadyPaired { node_id, label } => {
                                    if let Some(listener) = FILE_TRANSFER_LISTENER.lock().unwrap().as_ref() {
                                        listener.on_already_paired(node_id, label);
                                    }
                                }
                                IpcMessage::StalePairing { node_id, label } => {
                                    if let Some(listener) = FILE_TRANSFER_LISTENER.lock().unwrap().as_ref() {
                                        listener.on_stale_pairing(node_id, label);
                                    }
                                }
                                IpcMessage::FileProgress(_event) => {
                                    // These are already handled by the SEPARATE progress forwarder thread spawned above
                                    // But we could also handle them here if we didn't have that.
                                }
                                IpcMessage::DismissNotification { key } | IpcMessage::NotificationDismissed { key } => {
                                    if let Some(listener) = NOTIFICATION_LISTENER.lock().unwrap().as_ref() {
                                        listener.on_remote_dismiss_request(key);
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
pub fn start_benchmark(node_id: String) {
    if let Some(tx) = AGENT_TX.lock().unwrap().as_ref() {
        let _ = tx.send(IpcMessage::StartBenchmark { node_id });
    }
}

#[uniffi::export]
pub fn cancel_file_transfer(transfer_id: String) {
    if let Some(tm) = TRANSFER_MANAGER.lock().unwrap().as_ref() {
        tm.cancel_transfer(&transfer_id);
    }
}

#[uniffi::export]
pub fn simulate_crash(transfer_id: String) {
    if let Some(tm) = TRANSFER_MANAGER.lock().unwrap().as_ref() {
        tm.simulate_crash(&transfer_id);
    }
}

#[uniffi::export]
pub fn resume_file_transfer(transfer_id: String) {
    if let Some(tx) = AGENT_TX.lock().unwrap().as_ref() {
        let _ = tx.send(IpcMessage::ResumeFileTransfer { transfer_id });
    }
}

#[uniffi::export]
pub fn send_file(node_id: String, path: String) {
    let is_connected = if let Some(pm) = PAIRING_MANAGER.lock().unwrap().as_ref() {
        pm.sync_manager.is_connected(&node_id)
    } else {
        false
    };
    if !is_connected {
        error!("Cannot send file: Peer {} is disconnected", node_id);
        if let Some(tm) = TRANSFER_MANAGER.lock().unwrap().as_ref() {
            let _ = tm.progress_tx.send(ProgressEvent::Failed {
                transfer_id: "".to_string(),
                reason: "Peer is disconnected".to_string(),
            });
        }
        return;
    }

    let path_buf = std::path::PathBuf::from(path);
    let store_opt = STORE.lock().unwrap();
    let lm_opt = LIBP2P_MANAGER.lock().unwrap();
    let tm_opt = TRANSFER_MANAGER.lock().unwrap();
    
    if let (Some(store), Some(lm), Some(tm)) = (store_opt.as_ref(), lm_opt.as_ref(), tm_opt.as_ref()) {
        let store_clone = Arc::clone(store);
        let lm_clone = Arc::clone(lm);
        let tm_clone = Arc::clone(tm);
        
        std::thread::spawn(move || {
            let file_name = path_buf.file_name().unwrap().to_string_lossy().to_string();
            let total_bytes = path_buf.metadata().unwrap().len();
            let file_hash = cdus_agent::file_transfer::hash_file(&path_buf).unwrap();
            let transfer_id = uuid::Uuid::new_v4().to_string();
            
            store_clone.create_transfer(
                &transfer_id,
                "outgoing",
                &node_id,
                &path_buf.to_string_lossy(),
                &file_name,
                total_bytes,
                262144,
                &file_hash,
            ).unwrap();

            if let Ok(peer_id) = node_id.parse::<libp2p::PeerId>() {
                match lm_clone.open_file_stream(peer_id) {
                    Ok(wrapped_stream) => {
                        let session_key = cdus_agent::file_transfer::SessionKey([0u8; 32]);
                        let _ = cdus_agent::file_transfer::handle_outgoing_transfer(
                            Box::new(wrapped_stream),
                            store_clone,
                            transfer_id,
                            session_key,
                            tm_clone,
                        );
                    }
                    Err(e) => {
                        error!("Failed to open file stream to {}: {}", peer_id, e);
                        let _ = tm_clone.progress_tx.send(ProgressEvent::Failed { 
                            transfer_id, 
                            reason: format!("Connection failed: {}", e) 
                        });
                        if e.to_string().contains("Dial error") || e.to_string().contains("no addresses") {
                            if let Some(listener) = FILE_TRANSFER_LISTENER.lock().unwrap().as_ref() {
                                listener.on_peer_disconnected(node_id);
                            }
                        }
                    }
                }
            }
        });
    }
}

#[uniffi::export]
pub fn accept_file_transfer(transfer_id: String) {
    if let Some(tm) = TRANSFER_MANAGER.lock().unwrap().as_ref() {
        tm.handle_decision(&transfer_id, true);
    }
}

#[uniffi::export]
pub fn reject_file_transfer(transfer_id: String) {
    if let Some(tm) = TRANSFER_MANAGER.lock().unwrap().as_ref() {
        tm.handle_decision(&transfer_id, false);
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
                    is_sensitive: e.is_sensitive,
                    local_only: e.local_only,
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    }
}

#[uniffi::export]
pub fn set_clipboard_item_local_only(id: i64, local_only: bool) {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        let _ = store.set_local_only(id, local_only);
    }
}

#[uniffi::export]
pub fn delete_clipboard_item(id: i64) {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        let _ = store.delete_event(id);
    }
}

#[uniffi::export]
pub fn clear_clipboard_history() {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        let _ = store.clear_events();
    }
}

#[uniffi::export]
pub fn disconnect_device(node_id: String) {
    if let Some(pm) = PAIRING_MANAGER.lock().unwrap().as_ref() {
        if !pm.sync_manager.send_to_peer(&node_id, SyncMessage::Disconnect) {
            pm.sync_manager.remove_peer(&node_id);
        }
        // Also broadcast peer disconnected event locally
        cdus_agent::broadcast_event(IpcMessage::PeerDisconnected { node_id: node_id.clone() });
    }
    if let Some(tm) = TRANSFER_MANAGER.lock().unwrap().as_ref() {
        tm.cancel_all_transfers_for_peer(&node_id);
    }
    if let Some(lm) = LIBP2P_MANAGER.lock().unwrap().as_ref() {
        if let Ok(peer_id) = node_id.parse::<libp2p::PeerId>() {
            lm.disconnect_peer(peer_id);
        }
    }
}

#[uniffi::export]
pub fn broadcast_clipboard(content: String) {
    if content.trim().is_empty() {
        return;
    }
    if let Some(pm) = PAIRING_MANAGER.lock().unwrap().as_ref() {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Save to local store first
        if let Some(store) = STORE.lock().unwrap().as_ref() {
            let _ = store.append_event(content.as_bytes(), "Local");
            let _ = store.set_state("last_sync_timestamp", &timestamp.to_string());
            let _ = store.set_state("last_clipboard_content", &content);
        }

        pm.sync_manager
            .broadcast(SyncMessage::ClipboardUpdate { content, timestamp });
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct FileTransfer {
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

#[uniffi::export]
pub fn get_file_transfer_history(limit: u32) -> Vec<FileTransfer> {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        match store.get_transfer_history(limit) {
            Ok(transfers) => transfers
                .into_iter()
                .map(|t| FileTransfer {
                    transfer_id: t.transfer_id,
                    direction: t.direction,
                    peer_node_id: t.peer_node_id,
                    file_path: t.file_path,
                    file_name: t.file_name,
                    total_bytes: t.total_bytes as u64,
                    bytes_confirmed: t.bytes_confirmed as u64,
                    status: t.status,
                    error_message: t.error_message,
                    created_at: t.created_at as u64,
                    updated_at: t.updated_at as u64,
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    }
}

#[uniffi::export]
pub fn clear_finished_transfers() {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        if let Err(e) = store.clear_finished_transfers() {
            error!("clear_finished_transfers: failed to clear from database: {:?}", e);
        } else {
            info!("clear_finished_transfers: successfully cleared finished transfers");
        }
    } else {
        error!("clear_finished_transfers: STORE is None!");
    }
}

#[uniffi::export]
pub fn delete_file_transfer(transfer_id: String) {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        if let Err(e) = store.delete_transfer(&transfer_id) {
            error!("delete_file_transfer: failed to delete {} from database: {:?}", transfer_id, e);
        } else {
            info!("delete_file_transfer: successfully deleted {} from database", transfer_id);
        }
    } else {
        error!("delete_file_transfer: STORE is None!");
    }
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct PairedDevice {
    pub node_id: String,
    pub label: String,
    pub is_online: bool,
}

#[uniffi::export]
pub fn get_paired_devices() -> Vec<PairedDevice> {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        match store.get_paired_devices() {
            Ok(devices) => {
                let pm_lock = PAIRING_MANAGER.lock().unwrap();
                let sync_manager = pm_lock.as_ref().map(|pm| &pm.sync_manager);
                let peer_map = PEER_MAP.lock().unwrap();

                devices
                    .into_iter()
                    .map(|record| {
                        let is_online = sync_manager
                            .map(|sm| sm.is_connected(&record.node_id))
                            .unwrap_or(false)
                            || peer_map.get(&record.node_id).map(|(_, instant)| instant.elapsed() < std::time::Duration::from_secs(30)).unwrap_or(false);
                        PairedDevice {
                            node_id: record.node_id,
                            label: record.label,
                            is_online,
                        }
                    })
                    .collect()
            }
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    }
}

#[uniffi::export]
pub fn unpair_device(node_id: String) {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        let _ = store.remove_paired_device(&node_id);
    }
}

#[uniffi::export]
pub fn get_pairing_status() -> Option<PairingStatus> {
    let ap = ACTIVE_PAIRING.lock();
    ap.as_ref().map(|s| PairingStatus {
        active: true,
        pin: s.pin.clone(),
        remote_label: s.remote_label.clone(),
        is_initiator: s.is_initiator,
        silent: s.silent,
    })
}

static RECV_THREAD_STARTED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

#[uniffi::export]
pub fn initiate_pairing(node_id: String) {
    if let Some(pm) = PAIRING_MANAGER.lock().unwrap().as_ref() {
        let device = {
            let discovered = DISCOVERED.lock().unwrap();
            let peer_map = PEER_MAP.lock().unwrap();
            discovered.iter().find(|d| d.node_id == node_id).cloned()
                .or_else(|| peer_map.get(&node_id).map(|(d, _)| d.clone()))
        };

        let pm_clone = Arc::clone(pm);
        let node_id_clone = node_id.clone();
        std::thread::spawn(move || {
            let mut success = false;
            if let Some(device) = device {
                for ip in device.ips {
                    if let Ok(ip_addr) = ip.parse() {
                        let addr = std::net::SocketAddr::new(ip_addr, device.port);
                        if pm_clone.initiate_pairing(addr, Some(node_id_clone.clone())) {
                            success = true;
                            break;
                        }
                    }
                }
            }
            
            if !success {
                info!("FFI: mDNS failed for {}, falling back to relay", node_id_clone);
                pm_clone.initiate_remote_pairing(node_id_clone);
            }
        });
    }
}

#[uniffi::export]
pub fn confirm_pairing(accepted: bool) {
    let ap = ACTIVE_PAIRING.lock();
    if let Some(state) = ap.as_ref() {
        let mut confirmed = state.confirmed.lock();
        *confirmed = Some(accepted);
    }
}

#[uniffi::export]
pub fn cancel_pairing() {
    let mut ap = ACTIVE_PAIRING.lock();
    *ap = None;
}

#[uniffi::export]
pub fn get_qr_pairing_payload() -> String {
    if let Some(pm) = PAIRING_MANAGER.lock().unwrap().as_ref() {
        pm.generate_qr_payload().unwrap_or_default()
    } else {
        String::new()
    }
}

#[uniffi::export]
pub fn pair_with_qr(payload: String) {
    let mut should_initiate = false;
    let mut target_node_id = String::new();

    if let Some(pm) = PAIRING_MANAGER.lock().unwrap().as_ref() {
        match pm.parse_qr_payload(&payload) {
            Ok((node_id, secret, label, port, ips)) => {
                if pm.is_device_paired(&node_id) {
                    if let Some(tx) = AGENT_TX.lock().unwrap().as_ref() {
                        let _ = tx.send(IpcMessage::AlreadyPaired { node_id, label });
                    }
                    return;
                }
                
                info!("FFI: Scanned QR for {} ({}). IPs: {:?}, Port: {}. Setting OOB secret and attempting direct pairing.", label, node_id, ips, port);
                pm.set_target_oob_secret(node_id.clone(), secret);
                
                // Pre-populate PEER_MAP with data from QR to bypass mDNS discovery delay
                {
                    let mut peer_map = PEER_MAP.lock().unwrap();
                    peer_map.insert(node_id.clone(), (DiscoveredDevice {
                        node_id: node_id.clone(),
                        label: label.clone(),
                        os: "Unknown".to_string(),
                        ips,
                        port,
                    }, std::time::Instant::now()));
                }

                should_initiate = true;
                target_node_id = node_id;
            }
            Err(e) => {
                error!("FFI: Failed to parse QR payload: {}", e);
            }
        }
    }

    if should_initiate {
        // Now trigger local-first pairing (called outside the PAIRING_MANAGER lock to avoid deadlock)
        initiate_pairing(target_node_id);
    }
}

#[uniffi::export]
pub fn register_device(node_id: String, label: String, port: u16) {
    MDNS.register_device(&node_id, &label, port);
}

#[uniffi::export]
pub fn connect_relay() {
    if let Some(tx) = AGENT_TX.lock().unwrap().as_ref() {
        let _ = tx.send(IpcMessage::ConnectRelay);
    }
}

#[uniffi::export]
pub fn start_discovery() {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        let node_id = LOCAL_NODE_ID.lock().unwrap().clone();
        let label = store
            .get_state("device_name")
            .unwrap_or(None)
            .unwrap_or_else(|| "Android Device".to_string());
        MDNS.register_device(&node_id, &label, 5200);
    }

    let (tx, rx) = flume::unbounded();
    let discovered_clone = Arc::clone(&DISCOVERED);
    let local_id = LOCAL_NODE_ID.lock().unwrap().clone();

    {
        let mut started = RECV_THREAD_STARTED.lock().unwrap();
        if !*started {
            spawn_mdns_receiver(rx, discovered_clone, local_id);
            *started = true;
        } else {
            spawn_mdns_receiver(rx, discovered_clone, local_id);
        }
    }

    MDNS.start_discovery(tx);
}

#[uniffi::export]
pub fn stop_discovery() {
    MDNS.stop_discovery();
}

fn spawn_mdns_receiver(
    rx: flume::Receiver<IpcMessage>,
    discovered: Arc<Mutex<Vec<DiscoveredDevice>>>,
    local_id: String,
) {
    std::thread::spawn(move || {
        while let Ok(msg) = rx.recv() {
            match msg {
                IpcMessage::DeviceDiscovered {
                    node_id,
                    label,
                    os,
                    ips,
                    port,
                } => {
                    if !local_id.is_empty() && node_id == local_id {
                        continue;
                    }

                    let device = DiscoveredDevice {
                        node_id: node_id.clone(),
                        label: label.clone(),
                        os: os.clone(),
                        ips: ips.clone(),
                        port,
                    };

                    {
                        let mut map = PEER_MAP.lock().unwrap();
                        map.insert(node_id.clone(), (device.clone(), std::time::Instant::now()));
                    }

                    let mut list = discovered.lock().unwrap();
                    if !list.iter().any(|d| d.node_id == node_id) {
                        list.push(device);
                    }

                    // Auto-connect removed as per user request
                }
                IpcMessage::DeviceLost { node_id } => {
                    {
                        let mut map = PEER_MAP.lock().unwrap();
                        map.remove(&node_id);
                    }
                    let mut list = discovered.lock().unwrap();
                    list.retain(|d| !d.node_id.starts_with(&node_id));
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
pub fn greet_from_rust(name: String) -> String {
    format!("Hello, {}! This is CDUS Sync Core running on Rust.", name)
}

#[uniffi::export]
pub fn send_notification_mirror(
    key: String,
    package_name: String,
    app_name: String,
    title: String,
    text: String,
    timestamp: u64,
) {
    if let Some(pm) = PAIRING_MANAGER.lock().unwrap().as_ref() {
        let payload = cdus_common::NotificationPayload {
            key,
            package_name,
            app_name,
            title,
            text,
            timestamp,
        };
        pm.sync_manager.broadcast(SyncMessage::NotificationMirror(payload));
    }
}

#[uniffi::export]
pub fn send_notification_dismiss(key: String) {
    if let Some(pm) = PAIRING_MANAGER.lock().unwrap().as_ref() {
        pm.sync_manager.broadcast(SyncMessage::NotificationDismiss { key });
    }
}

#[uniffi::export]
pub fn get_audit_logs(limit: u32) -> Vec<AuditLogItem> {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        match store.get_audit_logs(limit) {
            Ok(logs) => logs
                .into_iter()
                .map(|e| AuditLogItem {
                    id: e.id,
                    event_type: e.event_type,
                    content: e.content,
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
pub fn clear_audit_logs() {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        let _ = store.clear_audit_logs();
    }
}

#[uniffi::export]
pub fn append_audit_log(event_type: String, content: String) {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        let _ = store.append_audit_log(&event_type, &content);
    }
}

#[derive(uniffi::Record)]
pub struct FfiSearchResult {
    pub id: String,
    pub item_type: String, // "clipboard" | "file" | "device"
    pub title: String,
    pub subtitle: String,
    pub timestamp: u64,
}

#[uniffi::export]
pub fn search(query: String) -> Vec<FfiSearchResult> {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        match store.search(&query) {
            Ok(results) => results
                .into_iter()
                .map(|r| FfiSearchResult {
                    id: r.id,
                    item_type: r.item_type,
                    title: r.title,
                    subtitle: r.subtitle,
                    timestamp: r.timestamp,
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    }
}

#[uniffi::export]
pub fn submit_feedback(text: String, attach_logs: bool) {
    let (node_id, relay_url) = {
        if let Some(rm) = RELAY_MANAGER.lock().unwrap().as_ref() {
            (rm.node_id().to_string(), rm.relay_url().to_string())
        } else {
            return;
        }
    };

    let logs_str = if attach_logs {
        if let Some(store) = STORE.lock().unwrap().as_ref() {
            match store.get_audit_logs(100) {
                Ok(logs) => logs
                    .into_iter()
                    .map(|l| format!("[{}] {}: {}", l.timestamp, l.event_type, l.content))
                    .collect::<Vec<_>>()
                    .join("\n"),
                Err(_) => "".to_string(),
            }
        } else {
            "".to_string()
        }
    } else {
        "".to_string()
    };

    let store_cb = {
        if let Some(_) = STORE.lock().unwrap().as_ref() {
            true
        } else {
            false
        }
    };

    if !store_cb {
        return;
    }

    std::thread::spawn(move || {
        let payload = serde_json::json!({
            "device_uuid": node_id,
            "content": text,
            "logs": logs_str,
        });

        let url = format!("{}/v1/feedback", relay_url);
        info!("FFI: Uploading user feedback to {}...", url);

        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(5))
            .build();

        match agent.post(&url).send_json(payload) {
            Ok(resp) if resp.status() == 200 => {
                info!("FFI: Feedback uploaded successfully.");
                if let Some(store) = STORE.lock().unwrap().as_ref() {
                    let _ = store.append_audit_log("system", "User feedback uploaded successfully to relay");
                }
            }
            Ok(resp) => {
                error!("FFI: Failed to upload feedback: status {}", resp.status());
                if let Some(store) = STORE.lock().unwrap().as_ref() {
                    let _ = store.append_audit_log("system", &format!("Failed to upload feedback: status {}", resp.status()));
                }
            }
            Err(e) => {
                error!("FFI: Error uploading feedback: {}", e);
                if let Some(store) = STORE.lock().unwrap().as_ref() {
                    let _ = store.append_audit_log("system", &format!("Error uploading feedback: {}", e));
                }
            }
        }
    });
}

#[uniffi::export]
pub fn set_telemetry_opt_in(opt_in: bool) {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        let val = if opt_in { "true" } else { "false" };
        let _ = store.set_state("telemetry_opt_in", val);
    }
}

#[uniffi::export]
pub fn get_telemetry_opt_in() -> bool {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        store.get_state("telemetry_opt_in")
            .unwrap_or(None)
            .map(|val| val == "true")
            .unwrap_or(false)
    } else {
        false
    }
}
