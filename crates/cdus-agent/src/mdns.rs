use mdns_sd::{ServiceDaemon, ServiceInfo, ServiceEvent};
use std::collections::HashMap;
use tracing::{info, error, debug, warn};
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
        
        #[cfg(target_os = "android")]
        {
            use mdns_sd::IfKind;
            info!("Android detected: disabling IPv6 for mDNS to improve reliability");
            let _ = daemon.disable_interface(IfKind::IPv6);
        }

        // Enable multicast loopback to ensure we can see our own announcements if needed
        // and to keep the socket 'warm' on some platforms.
        let _ = daemon.set_multicast_loop_v4(true);

        Self { 
            daemon,
            discovery_thread: Mutex::new(None),
            is_scanning: Arc::new(Mutex::new(false)),
        }
    }
}

impl Default for MdnsManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MdnsManager {
    pub fn register_device(&self, node_id: &str, label: &str, port: u16) {
        let service_type = "_cdus._tcp.local.";
        // DNS labels are limited to 63 characters. node_id is 64 hex chars.
        // We use a truncated ID for the instance name to avoid panics in mdns-sd.
        let short_id = if node_id.len() > 32 { &node_id[..32] } else { node_id };
        let instance_name = short_id;
        let host_name = format!("{}.local.", short_id);

        let mut properties = HashMap::new();
        properties.insert("label".to_string(), label.to_string());
        
        #[cfg(target_os = "linux")]
        let os = "Linux";
        #[cfg(target_os = "android")]
        let os = "Android";
        #[cfg(target_os = "windows")]
        let os = "Windows";
        #[cfg(target_os = "macos")]
        let os = "macOS";
        #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "windows", target_os = "macos")))]
        let os = "Unknown";
        
        properties.insert("os".to_string(), os.to_string());
        properties.insert("node_id".to_string(), node_id.to_string());

        let service_info = ServiceInfo::new(
            service_type,
            instance_name,
            &host_name,
            (),
            port,
            Some(properties),
        ).expect("Failed to create service info")
        .enable_addr_auto();

        let daemon = self.daemon.clone();
        let info_clone = service_info.clone();
        
        // Initial registration
        if let Err(e) = daemon.register(service_info) {
            error!("Failed to register mDNS service: {}", e);
        } else {
            info!("mDNS service registered for node: {} ({})", node_id, label);
        }

        // Start a background thread to periodically re-announce (every 60s)
        // This ensures we stay in other devices' caches even if they missed the initial burst.
        thread::spawn(move || {
            loop {
                thread::sleep(std::time::Duration::from_secs(60));
                debug!("Re-announcing mDNS service...");
                if let Err(e) = daemon.register(info_clone.clone()) {
                    warn!("mDNS re-announcement failed: {}", e);
                }
            }
        });
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
            
            info!("mDNS discovery thread started for type: {}", service_type);

            while *is_scanning.lock().unwrap() {
                // Force a fresh query by ensuring we aren't already browsing
                let _ = daemon.stop_browse(service_type);
                
                let receiver = match daemon.browse(service_type) {
                    Ok(r) => r,
                    Err(e) => {
                        error!("Failed to start mDNS browsing: {}", e);
                        thread::sleep(std::time::Duration::from_secs(2));
                        continue;
                    }
                };

                // Listen for events for a few seconds before re-querying
                let start_time = std::time::Instant::now();
                while start_time.elapsed() < std::time::Duration::from_secs(5) && *is_scanning.lock().unwrap() {
                    match receiver.recv_timeout(std::time::Duration::from_millis(500)) {
                        Ok(event) => {
                            match event {
                                ServiceEvent::ServiceFound(ty, name) => {
                                    info!("mDNS service found: {} (type: {})", name, ty);
                                }
                                ServiceEvent::ServiceResolved(info) => {
                                    let node_id = info.get_property_val_str("node_id").unwrap_or_default().to_string();
                                    let label = info.get_property_val_str("label").unwrap_or_else(|| info.get_fullname()).to_string();
                                    let os = info.get_property_val_str("os").unwrap_or("Unknown").to_string();
                                    let addresses: Vec<_> = info.get_addresses().iter().map(|addr| addr.to_string()).collect();
                                    let ip = addresses.first().cloned().unwrap_or_default();
                                    let port = info.get_port();
                                    
                                    info!("mDNS resolved service: {} at {:?}:{}", label, addresses, port);
                                    
                                    if !node_id.is_empty() && !ip.is_empty() {
                                        info!("Discovered CDUS device: {} ({}) at {}:{}", label, node_id, ip, port);
                                        let _ = tx.send(IpcMessage::DeviceDiscovered { node_id, label, os, ip, port });
                                    }
                                }
                                ServiceEvent::ServiceRemoved(ty, name) => {
                                    info!("mDNS service removed: {} (type: {})", name, ty);
                                    // Instance name is the truncated node ID (32 chars)
                                    let _ = tx.send(IpcMessage::DeviceLost { node_id: name });
                                }
                                ServiceEvent::SearchStarted(ty) => {
                                    info!("mDNS search started for type: {}", ty);
                                }
                                ServiceEvent::SearchStopped(ty) => {
                                    info!("mDNS search stopped for type: {}", ty);
                                }
                            }
                        }
                        Err(flume::RecvTimeoutError::Timeout) => {
                            // Regular timeout, just continue inner loop
                        }
                        Err(flume::RecvTimeoutError::Disconnected) => {
                            error!("mDNS receiver disconnected");
                            return;
                        }
                    }
                }
            }
            info!("mDNS discovery stopped");
        });

        let mut thread_handle = self.discovery_thread.lock().unwrap();
        *thread_handle = Some(handle);
    }

    pub fn stop_discovery(&self) {
        {
            let mut scanning = self.is_scanning.lock().unwrap();
            *scanning = false;
        }
        
        let mut thread_handle = self.discovery_thread.lock().unwrap();
        if let Some(handle) = thread_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flume;
    use std::time::Duration;

    #[test]
    fn test_mdns_manager_lifecycle() {
        let manager = MdnsManager::new();
        let (tx, _rx) = flume::unbounded();
        
        manager.start_discovery(tx);
        assert!(*manager.is_scanning.lock().unwrap());
        
        thread::sleep(Duration::from_millis(100));
        
        manager.stop_discovery();
        assert!(!*manager.is_scanning.lock().unwrap());
    }
}
