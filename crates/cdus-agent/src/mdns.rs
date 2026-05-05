use mdns_sd::{ServiceDaemon, ServiceInfo, ServiceEvent};
use std::collections::HashMap;
use tracing::{info, error};
use flume::Sender;
use cdus_common::IpcMessage;
use std::sync::{Arc, Mutex};
use std::thread;

pub struct MdnsManager {
    daemon: ServiceDaemon,
    discovery_thread: Mutex<Option<thread::JoinHandle<()>>>,
    is_scanning: Arc<Mutex<bool>>,
}

impl MdnsManager {
    pub fn new() -> Self {
        let daemon = ServiceDaemon::new().expect("Failed to create mDNS daemon");
        Self { 
            daemon,
            discovery_thread: Mutex::new(None),
            is_scanning: Arc::new(Mutex::new(false)),
        }
    }

    pub fn register_device(&self, node_id: &str, label: &str) {
        let service_type = "_cdus._tcp.local.";
        let instance_name = node_id;
        let host_name = format!("{}.local.", node_id);
        let port = 5200; // Placeholder for now

        let mut properties = HashMap::new();
        properties.insert("label".to_string(), label.to_string());
        
        #[cfg(target_os = "linux")]
        let os = "Linux";
        #[cfg(target_os = "windows")]
        let os = "Windows";
        #[cfg(target_os = "macos")]
        let os = "macOS";
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        let os = "Unknown";
        
        properties.insert("os".to_string(), os.to_string());
        properties.insert("node_id".to_string(), node_id.to_string());

        // In mdns-sd 0.13, new() takes (type, name, host, ip, port, properties)
        // Passing () for ip as we want the daemon to handle it.
        let service_info = ServiceInfo::new(
            service_type,
            instance_name,
            &host_name,
            "", // ip address string
            port,
            Some(properties),
        ).expect("Failed to create service info");

        self.daemon.register(service_info).expect("Failed to register mDNS service");
        info!("mDNS service registered for node: {} ({})", node_id, label);
    }

    pub fn start_discovery(&self, tx: Sender<IpcMessage>) {
        let mut scanning = self.is_scanning.lock().unwrap();
        if *scanning {
            info!("Discovery already running");
            return;
        }
        *scanning = true;

        let daemon = self.daemon.clone();
        let is_scanning = Arc::clone(&self.is_scanning);
        
        let handle = thread::spawn(move || {
            let service_type = "_cdus._tcp.local.";
            let receiver = match daemon.browse(service_type) {
                Ok(r) => r,
                Err(e) => {
                    error!("Failed to start mDNS browsing: {}", e);
                    return;
                }
            };

            info!("mDNS discovery started");

            while *is_scanning.lock().unwrap() {
                if let Ok(event) = receiver.recv_timeout(std::time::Duration::from_millis(500)) {
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let props = info.get_properties();
                            let node_id = props.get("node_id").map(|v| v.to_string()).unwrap_or_default();
                            let label = props.get("label").map(|v| v.to_string()).unwrap_or_else(|| info.get_fullname().to_string());
                            let os = props.get("os").map(|v| v.to_string()).unwrap_or_else(|| "Unknown".to_string());
                            
                            if !node_id.is_empty() {
                                info!("Discovered CDUS device: {} ({})", label, node_id);
                                let _ = tx.send(IpcMessage::DeviceDiscovered { node_id, label, os });
                            }
                        }
                        _ => {}
                    }
                }
            }
            info!("mDNS discovery stopped");
        });

        let mut thread_handle = self.discovery_thread.lock().unwrap();
        *thread_handle = Some(handle);
    }

    pub fn stop_discovery(&self) {
        let mut scanning = self.is_scanning.lock().unwrap();
        *scanning = false;
        
        let mut thread_handle = self.discovery_thread.lock().unwrap();
        if let Some(handle) = thread_handle.take() {
            let _ = handle.join();
        }
    }
}
