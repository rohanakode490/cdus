uniffi::setup_scaffolding!();

use tracing::{info, error, warn};
use std::sync::{Arc, Mutex};
use once_cell::sync::Lazy;

#[uniffi::export]
pub fn init_logging() {
    #[cfg(target_os = "android")]
    {
        use tracing_subscriber::prelude::*;
        use tracing_subscriber::Registry;

        let android_layer = tracing_android::layer("CDUS-Rust").ok();
        Registry::default()
            .with(android_layer)
            .init();
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

#[uniffi::export(callback_interface)]
pub trait ClipboardListener: Send + Sync {
    fn on_clipboard_update(&self, content: String, source: String);
}

static CLIPBOARD_LISTENER: Lazy<Mutex<Option<Box<dyn ClipboardListener>>>> = Lazy::new(|| Mutex::new(None));

#[uniffi::export]
pub fn set_clipboard_listener(listener: Box<dyn ClipboardListener>) {
    *CLIPBOARD_LISTENER.lock().unwrap() = Some(listener);
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct PairingStatus {
    pub active: bool,
    pub pin: String,
    pub remote_label: String,
    pub is_initiator: bool,
}

// Global instances for mobile use
static MDNS: Lazy<Arc<cdus_agent::mdns::MdnsManager>> = Lazy::new(|| {
    Arc::new(cdus_agent::mdns::MdnsManager::new())
});

static DISCOVERED: Lazy<Arc<Mutex<Vec<DiscoveredDevice>>>> = Lazy::new(|| {
    Arc::new(Mutex::new(Vec::new()))
});

static LOCAL_NODE_ID: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

static ACTIVE_PAIRING: Lazy<Arc<Mutex<Option<cdus_agent::pairing::ActivePairingState>>>> = Lazy::new(|| {
    Arc::new(Mutex::new(None))
});

static PAIRING_MANAGER: Lazy<Mutex<Option<Arc<cdus_agent::pairing::PairingManager>>>> = Lazy::new(|| {
    Mutex::new(None)
});

static STORE: Lazy<Mutex<Option<Arc<cdus_agent::store::Store>>>> = Lazy::new(|| {
    Mutex::new(None)
});

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
                    let label = store.get_state("device_name").unwrap_or(None).unwrap_or_else(|| "Android Device".to_string());
                    
                    let (tx, rx) = flume::unbounded();
                    
                    // Initialize Relay Manager with a default URL for now
                    let relay_url = if cfg!(target_os = "android") {
                        "http://10.0.2.2:8080".to_string()
                    } else {
                        "http://localhost:8080".to_string()
                    };
                    let (relay, _relay_rx) = cdus_agent::relay::RelayManager::new(node_id.clone(), relay_url, tx.clone());
                    let relay = Arc::new(relay);
                    
                    // Initialize Turn Manager
                    let turn = Arc::new(cdus_agent::turn_manager::TurnManager::new().expect("Failed to init TurnManager"));
                    
                    // Setup Pairing Manager
                    let pm = cdus_agent::pairing::PairingManager::new(
                        Arc::clone(&store),
                        tx.clone(),
                        node_id.clone(),
                        private_key,
                        5200, // Default port
                        Arc::clone(&ACTIVE_PAIRING),
                        Arc::new(cdus_agent::pairing::SyncManager::new()),
                        relay,
                        turn,
                    );
                    let pm = Arc::new(pm);
                    let pm_clone = Arc::clone(&pm);
                    std::thread::spawn(move || {
                        pm_clone.start_listener();
                    });
                    
                    *PAIRING_MANAGER.lock().unwrap() = Some(pm);
                    
                    // Background thread to drain messages and notify listener
                    std::thread::spawn(move || {
                        while let Ok(msg) = rx.recv() {
                            match msg {
                                cdus_common::IpcMessage::SetClipboard { content, source, .. } => {
                                    if let Some(listener) = CLIPBOARD_LISTENER.lock().unwrap().as_ref() {
                                        listener.on_clipboard_update(content, source);
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
pub fn get_clipboard_history(limit: u32) -> Vec<ClipboardHistoryItem> {
    if let Some(store) = STORE.lock().unwrap().as_ref() {
        match store.get_recent_events(limit) {
            Ok(events) => events.into_iter()
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

        pm.sync_manager.broadcast(cdus_common::SyncMessage::ClipboardUpdate {
            content,
            timestamp,
        });
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
            Ok(devices) => devices.into_iter()
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
        info!("FFI: Current discovered devices count: {}", discovered.len());
        if let Some(device) = discovered.iter().find(|d| d.node_id == node_id) {
            info!("FFI: Found device in list, starting pairing thread for {}", device.ip);
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
            warn!("FFI: Device {} not found in discovered list. Current list: {:?}", node_id, discovered);
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
        info!("FFI: Pairing {} by user", if accepted { "confirmed" } else { "declined" });
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
    info!("FFI: Registering device: {} ({}) on port {}", node_id, label, port);
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
fn spawn_mdns_receiver(rx: flume::Receiver<cdus_common::IpcMessage>, discovered: Arc<Mutex<Vec<DiscoveredDevice>>>, local_id: String) {
    std::thread::spawn(move || {
        while let Ok(msg) = rx.recv() {
            match msg {
                cdus_common::IpcMessage::DeviceDiscovered { node_id, label, os, ip, port } => {
                    if !local_id.is_empty() && node_id == local_id {
                        continue;
                    }
                    
                    let mut list = discovered.lock().unwrap();
                    if !list.iter().any(|d| d.node_id == node_id) {
                        info!("FFI: Discovered device: {} ({}) at {}:{}", label, node_id, ip, port);
                        list.push(DiscoveredDevice { node_id, label, os, ip, port });
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
    format!("Hello, {}! This is CDUS core running on Android via Rust.", name)
}
