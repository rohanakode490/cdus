use cdus_common::IpcMessage;
use flume::Sender;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::sync::Arc; use parking_lot::Mutex;
use std::thread;
use tracing::{debug, error, info};
#[cfg(target_os = "android")]
use tracing::warn;

pub struct MdnsManager {
    daemon: ServiceDaemon,
    discovery_thread: Mutex<Option<thread::JoinHandle<()>>>,
    is_scanning: Arc<Mutex<bool>>,
    registered_services: Mutex<HashMap<String, ServiceInfo>>,
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
            registered_services: Mutex::new(HashMap::new()),
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
        let short_id = if node_id.len() > 32 {
            &node_id[..32]
        } else {
            node_id
        };
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
        #[cfg(not(any(
            target_os = "linux",
            target_os = "android",
            target_os = "windows",
            target_os = "macos"
        )))]
        let os = "Unknown";

        properties.insert("os".to_string(), os.to_string());
        properties.insert("node_id".to_string(), node_id.to_string());

        #[cfg(target_os = "android")]
        let service_info = if let Some(ip) = get_local_ip() {
            info!("Android: Using explicit local IP for mDNS: {}", ip);
            ServiceInfo::new(
                service_type,
                instance_name,
                &host_name,
                ip.to_string().as_str(),
                port,
                Some(properties),
            )
            .expect("Failed to create service info")
        } else {
            warn!("Android: Could not find local IP, falling back to auto-address");
            ServiceInfo::new(
                service_type,
                instance_name,
                &host_name,
                (),
                port,
                Some(properties),
            )
            .expect("Failed to create service info")
            .enable_addr_auto()
        };

        #[cfg(not(target_os = "android"))]
        let service_info = ServiceInfo::new(
            service_type,
            instance_name,
            &host_name,
            (),
            port,
            Some(properties),
        )
        .expect("Failed to create service info")
        .enable_addr_auto();

        let daemon = self.daemon.clone();

        let mut registered = self.registered_services.lock();
        if let Some(existing) = registered.get(instance_name) {
            if existing.get_port() == port && existing.get_properties() == service_info.get_properties() {
                debug!("Service already registered with same info: {}", instance_name);
                return;
            }
            info!("Updating mDNS service registration for: {}", instance_name);
            let _ = daemon.unregister(existing.get_fullname());
        }

        // Initial registration
        if let Err(e) = daemon.register(service_info.clone()) {
            error!("Failed to register mDNS service: {}", e);
        } else {
            info!("mDNS service registered for node: {} ({})", node_id, label);
            registered.insert(instance_name.to_string(), service_info);
        }
    }

    pub fn start_discovery(&self, tx: Sender<IpcMessage>) {
        let mut scanning = self.is_scanning.lock();
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

            let receiver = match daemon.browse(service_type) {
                Ok(r) => r,
                Err(e) => {
                    error!("Failed to start mDNS browsing: {}", e);
                    let mut scanning = is_scanning.lock();
                    *scanning = false;
                    return;
                }
            };

            while *is_scanning.lock() {
                match receiver.recv_timeout(std::time::Duration::from_millis(500)) {
                    Ok(event) => {
                        match event {
                            ServiceEvent::ServiceFound(ty, name) => {
                                info!("mDNS service found: {} (type: {})", name, ty);
                            }
                            ServiceEvent::ServiceResolved(info) => {
                                let node_id = info
                                    .get_property_val_str("node_id")
                                    .unwrap_or_default()
                                    .to_string();
                                let label = info
                                    .get_property_val_str("label")
                                    .unwrap_or_else(|| info.get_fullname())
                                    .to_string();
                                let os = info
                                    .get_property_val_str("os")
                                    .unwrap_or("Unknown")
                                    .to_string();

                                let mut addresses: Vec<_> = info
                                    .get_addresses()
                                    .iter()
                                    .cloned()
                                    .collect();

                                // Rank addresses to prioritize LAN over virtual interfaces
                                addresses.sort_by(|a, b| {
                                    let score = |addr: &std::net::IpAddr| {
                                        let ip_str = addr.to_string();
                                        if ip_str.starts_with("192.168.") || ip_str.starts_with("10.") {
                                            100 // Common LAN
                                        } else if ip_str.starts_with("172.") && !ip_str.starts_with("172.17.") {
                                            90 // Possible LAN (excluding default Docker)
                                        } else if addr.is_loopback() {
                                            0 // Loopback
                                        } else if addr.is_ipv6() {
                                            50 // IPv6
                                        } else if ip_str.starts_with("172.17.") {
                                            10 // Default Docker bridge
                                        } else {
                                            60 // Others
                                        }
                                    };
                                    score(b).cmp(&score(a))
                                });

                                let ips: Vec<String> = addresses.iter().map(|a| a.to_string()).collect();
                                let port = info.get_port();

                                info!(
                                    "mDNS resolved service: {} at {:?}:{}",
                                    label, ips, port
                                );

                                if !node_id.is_empty() && !ips.is_empty() {
                                    info!(
                                        "Discovered CDUS device: {} ({}) at {:?}:{}",
                                        label, node_id, ips, port
                                    );
                                    let _ = tx.send(IpcMessage::DeviceDiscovered {
                                        node_id,
                                        label,
                                        os,
                                        ips,
                                        port,
                                    });
                                }
                            }

                            ServiceEvent::ServiceRemoved(ty, name) => {
                                info!("mDNS service removed: {} (type: {})", name, ty);
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
                        // Regular timeout, just continue
                    }
                    Err(flume::RecvTimeoutError::Disconnected) => {
                        error!("mDNS receiver disconnected");
                        break;
                    }
                }
            }
            let _ = daemon.stop_browse(service_type);
            info!("mDNS discovery stopped");
        });

        let mut thread_handle = self.discovery_thread.lock();
        *thread_handle = Some(handle);
    }

    pub fn stop_discovery(&self) {
        {
            let mut scanning = self.is_scanning.lock();
            *scanning = false;
        }

        let mut thread_handle = self.discovery_thread.lock();
        if let Some(handle) = thread_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(target_os = "android")]
fn get_local_ip() -> Option<std::net::IpAddr> {
    use std::net::UdpSocket;
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip())
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
        assert!(*manager.is_scanning.lock());

        thread::sleep(Duration::from_millis(100));

        manager.stop_discovery();
        assert!(!*manager.is_scanning.lock());
    }
}
