uniffi::setup_scaffolding!();

use tracing::info;
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

#[uniffi::export]
pub fn init_core(data_dir: String) -> String {
    let path = std::path::Path::new(&data_dir);
    if let Err(e) = std::fs::create_dir_all(path) {
        return format!("error:Failed to create directory: {}", e);
    }
    
    match cdus_agent::store::Store::init(path) {
        Ok(store) => {
            let store = Arc::new(store);
            match store.get_or_create_identity(path) {
                Ok((node_id, private_key)) => {
                    *LOCAL_NODE_ID.lock().unwrap() = node_id.clone();
                    let label = store.get_state("device_name").unwrap_or(None).unwrap_or_else(|| "Android Device".to_string());
                    
                    let (tx, rx) = flume::unbounded();
                    
                    // Initialize Relay Manager with a default URL for now
                    let relay_url = "http://localhost:8080".to_string(); // In a real app, this would be configurable
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
                    
                    // Background thread to drain messages (could be expanded to handle them)
                    std::thread::spawn(move || {
                        while let Ok(msg) = rx.recv() {
                            info!("FFI Core: Received IPC message: {:?}", msg);
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
pub fn get_pairing_status() -> Option<PairingStatus> {
    let ap = ACTIVE_PAIRING.lock().unwrap();
    ap.as_ref().map(|s| PairingStatus {
        active: true,
        pin: s.pin.clone(),
        remote_label: s.remote_label.clone(),
        is_initiator: s.is_initiator,
    })
}

#[uniffi::export]
pub fn initiate_pairing(node_id: String) {
    if let Some(pm) = PAIRING_MANAGER.lock().unwrap().as_ref() {
        let discovered = DISCOVERED.lock().unwrap();
        if let Some(device) = discovered.iter().find(|d| d.node_id == node_id) {
            if let Ok(ip_addr) = device.ip.parse() {
                let addr = std::net::SocketAddr::new(ip_addr, device.port);
                let pm_clone = Arc::clone(pm);
                std::thread::spawn(move || {
                    info!("FFI: Initiating pairing with {} at {}", node_id, addr);
                    pm_clone.initiate_pairing(addr);
                });
            }
        }
    }
}

#[uniffi::export]
pub fn confirm_pairing(accepted: bool) {
    let ap = ACTIVE_PAIRING.lock().unwrap();
    if let Some(state) = ap.as_ref() {
        let mut confirmed = state.confirmed.lock().unwrap();
        *confirmed = Some(accepted);
        info!("FFI: Pairing {} by user", if accepted { "confirmed" } else { "declined" });
    }
}

#[uniffi::export]
pub fn cancel_pairing() {
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
    
    spawn_mdns_receiver(rx, discovered_clone, local_id);

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
            if let cdus_common::IpcMessage::DeviceDiscovered { node_id, label, os, ip, port } = msg {
                if !local_id.is_empty() && node_id == local_id {
                    continue;
                }
                
                let mut list = discovered.lock().unwrap();
                if !list.iter().any(|d| d.node_id == node_id) {
                    info!("FFI: Discovered device: {} ({}) at {}:{}", label, node_id, ip, port);
                    list.push(DiscoveredDevice { node_id, label, os, ip, port });
                }
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
