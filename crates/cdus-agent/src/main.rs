use flume::{Receiver, Sender};
use interprocess::local_socket::LocalSocketListener;
use std::io::{Read, Write};
use std::thread;
use tracing::{info, error};
use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use std::sync::{Arc, Mutex};
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH, Duration};

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long, default_value = "5200")]
    port: u16,

    #[arg(short, long, default_value = "/tmp/cdus-agent.sock")]
    socket: String,

    #[arg(long)]
    data_dir: Option<String>,

    #[arg(long, default_value = "http://localhost:8080")]
    relay_url: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Install the agent as a systemd user service
    Install,
    /// Uninstall the agent systemd user service
    Uninstall,
}

mod store;
mod mdns;
mod pairing;
mod relay;
mod integration_tests;
use store::Store;
use mdns::MdnsManager;
use pairing::{PairingManager, SyncManager};
use relay::RelayManager;
use cdus_common::{IpcMessage, SyncMessage};

#[derive(Clone)]
pub struct ActivePairingState {
    pub pin: String,
    pub is_initiator: bool,
    pub remote_id: String,
    pub remote_label: String,
    pub confirmed: Arc<Mutex<Option<bool>>>,
}

fn main() {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match &cli.command {
        Some(Commands::Install) => {
            install_service();
            return;
        }
        Some(Commands::Uninstall) => {
            uninstall_service();
            return;
        }
        None => {}
    }

    let data_dir_buf;
    let data_dir = if let Some(ref d) = cli.data_dir {
        data_dir_buf = std::path::PathBuf::from(d);
        &data_dir_buf
    } else {
        let config_dir = ProjectDirs::from("com", "cdus", "agent")
            .expect("Failed to get config directory");
        data_dir_buf = config_dir.data_dir().to_path_buf();
        &data_dir_buf
    };

    std::fs::create_dir_all(data_dir).expect("Failed to create data directory");

    let store = Store::init(data_dir).expect("Failed to initialize store");
    let store = Arc::new(store);

    let active_pairing = Arc::new(Mutex::new(None::<ActivePairingState>));
    let active_pairing_daemon = Arc::clone(&active_pairing);

    // Initialize or load device identity
    let (node_id, private_key) = store.get_or_create_identity(data_dir).expect("Failed to initialize identity");
    let label = store.get_state("device_name").unwrap().unwrap_or_else(|| "Unknown".to_string());
    info!("Device identity initialized. Node ID: {}", node_id);

    // Initialize Relay Manager
    let relay = RelayManager::new(node_id.clone(), cli.relay_url.clone());
    if let Err(e) = relay.register() {
        error!("Failed to register with relay: {}. Will retry connection in background loop.", e);
    }
    relay.start_signaling_loop();

    // Start mDNS registration
    let mdns = MdnsManager::new();
    mdns.register_device(&node_id, &label, cli.port);
    let mdns = Arc::new(mdns);

    let (tx, rx) = flume::unbounded::<IpcMessage>();

    let sync_manager = Arc::new(SyncManager::new());
    let sync_manager_daemon = Arc::clone(&sync_manager);

    // Start Pairing Manager
    let pm = PairingManager::new(Arc::clone(&store), tx.clone(), node_id, private_key, cli.port, Arc::clone(&active_pairing), Arc::clone(&sync_manager));
    let pm = Arc::new(pm);
    let pm_clone = Arc::clone(&pm);
    thread::spawn(move || {
        pm_clone.start_listener();
    });

    info!("CDUS Agent starting on port {}...", cli.port);

    // Shared state for loop prevention and LWW
    let last_written = Arc::new(Mutex::new(None::<String>));
    let last_processed_timestamp = Arc::new(Mutex::new(0u64));

    // Initialize timestamp from store if available
    if let Ok(Some(ts_str)) = store.get_state("last_sync_timestamp") {
        if let Ok(ts) = ts_str.parse::<u64>() {
            *last_processed_timestamp.lock().unwrap() = ts;
            info!("Initialized LWW timestamp from store: {}", ts);
        }
    }

    // Start clipboard watcher thread
    let clipboard_tx = tx.clone();
    let last_written_watcher = Arc::clone(&last_written);
    thread::spawn(move || {
        clipboard_watcher(clipboard_tx, last_written_watcher);
    });

    let discovered_devices = Arc::new(Mutex::new(Vec::<(String, String, String, String, u16)>::new()));
    let discovered_devices_daemon = Arc::clone(&discovered_devices);

    // Start daemon logic thread
    let daemon_tx = tx.clone();
    let last_written_daemon = Arc::clone(&last_written);
    let daemon_store = Arc::clone(&store);
    let pm_daemon_loop = Arc::clone(&pm);
    let last_ts_daemon = Arc::clone(&last_processed_timestamp);
    thread::spawn(move || {
        daemon_loop(daemon_tx, rx, None, daemon_store, last_written_daemon, discovered_devices_daemon, active_pairing_daemon, sync_manager_daemon, pm_daemon_loop, last_ts_daemon);
    });


    // Setup IPC listener
    let socket_name = cli.socket.clone();
    let _ = std::fs::remove_file(&socket_name);
    let listener = LocalSocketListener::bind(&*socket_name).expect("Failed to bind local socket");

    info!("IPC Listener bound to {}", socket_name);

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let mut buffer = [0u8; 4096];
                if let Ok(n) = stream.read(&mut buffer) {
                    if let Ok(msg) = serde_json::from_slice::<IpcMessage>(&buffer[..n]) {
                        match msg {
                            IpcMessage::Ping => {
                                let resp_bytes = serde_json::to_vec(&IpcMessage::Pong).unwrap();
                                let _ = stream.write_all(&resp_bytes);
                            }
                            IpcMessage::StartScan => {
                                {
                                    let mut list = discovered_devices.lock().unwrap();
                                    list.clear();
                                }
                                mdns.start_discovery(tx.clone());
                                let resp_bytes = serde_json::to_vec(&IpcMessage::Log("Scan started".to_string())).unwrap();
                                let _ = stream.write_all(&resp_bytes);
                            }
                            IpcMessage::StopScan => {
                                mdns.stop_discovery();
                                let resp_bytes = serde_json::to_vec(&IpcMessage::Log("Scan stopped".to_string())).unwrap();
                                let _ = stream.write_all(&resp_bytes);
                            }
                            IpcMessage::GetDiscovered => {
                                let list = discovered_devices.lock().unwrap();
                                let resp_bytes = serde_json::to_vec(&IpcMessage::DiscoveredResponse(list.clone())).unwrap();
                                let _ = stream.write_all(&resp_bytes);
                            }
                            IpcMessage::PairWith { node_id } => {
                                let list = discovered_devices.lock().unwrap();
                                if let Some((_, _, _, ip, port)) = list.iter().find(|(id, _, _, _, _)| id == &node_id) {
                                    if let Ok(ip_addr) = ip.parse() {
                                        let addr = SocketAddr::new(ip_addr, *port);
                                        let pm_init = Arc::clone(&pm);
                                        thread::spawn(move || {
                                            pm_init.initiate_pairing(addr);
                                        });
                                        let resp_bytes = serde_json::to_vec(&IpcMessage::Log("Pairing initiated".to_string())).unwrap();
                                        let _ = stream.write_all(&resp_bytes);
                                    }
                                }
                            }
                            IpcMessage::PairWithIp { ip, port } => {
                                if let Ok(ip_addr) = ip.parse() {
                                    let addr = SocketAddr::new(ip_addr, port);
                                    let pm_init = Arc::clone(&pm);
                                    thread::spawn(move || {
                                        pm_init.initiate_pairing(addr);
                                    });
                                    let resp_bytes = serde_json::to_vec(&IpcMessage::Log("Manual pairing initiated".to_string())).unwrap();
                                    let _ = stream.write_all(&resp_bytes);
                                }
                            }
                            IpcMessage::ConfirmPairing(accepted) => {
                                let ap = active_pairing.lock().unwrap();
                                if let Some(ref state) = *ap {
                                    let mut res = state.confirmed.lock().unwrap();
                                    *res = Some(accepted);
                                }
                                let resp_bytes = serde_json::to_vec(&IpcMessage::Log(format!("Pairing result processed: {}", accepted))).unwrap();
                                let _ = stream.write_all(&resp_bytes);
                            }
                            IpcMessage::GetPairingStatus => {
                                let ap = active_pairing.lock().unwrap();
                                let resp = match *ap {
                                    Some(ref state) => IpcMessage::PairingStatusResponse {
                                        pin: Some(state.pin.clone()),
                                        active: true,
                                        is_initiator: state.is_initiator,
                                        remote_label: state.remote_label.clone(),
                                    },
                                    None => IpcMessage::PairingStatusResponse {
                                        pin: None,
                                        active: false,
                                        is_initiator: false,
                                        remote_label: String::new(),
                                    },
                                };
                                let resp_bytes = serde_json::to_vec(&resp).unwrap();
                                let _ = stream.write_all(&resp_bytes);
                            }
                            IpcMessage::GetPairedDevices => {
                                match store.get_paired_devices() {
                                    Ok(devices) => {
                                        let resp_bytes = serde_json::to_vec(&IpcMessage::PairedDevicesResponse(devices)).unwrap();
                                        let _ = stream.write_all(&resp_bytes);
                                    }
                                    Err(e) => {
                                        let resp_bytes = serde_json::to_vec(&IpcMessage::Log(format!("Error fetching paired devices: {}", e))).unwrap();
                                        let _ = stream.write_all(&resp_bytes);
                                    }
                                }
                            }
                            IpcMessage::UnpairDevice { node_id } => {
                                match store.remove_paired_device(&node_id) {
                                    Ok(_) => {
                                        let resp_bytes = serde_json::to_vec(&IpcMessage::Log("Device unpaired".to_string())).unwrap();
                                        let _ = stream.write_all(&resp_bytes);
                                    }
                                    Err(e) => {
                                        let resp_bytes = serde_json::to_vec(&IpcMessage::Log(format!("Error unpairing device: {}", e))).unwrap();
                                        let _ = stream.write_all(&resp_bytes);
                                    }
                                }
                            }
                            IpcMessage::GetHistory { limit } => {
                                match store.get_recent_events(limit) {
                                    Ok(history) => {
                                        let resp_bytes = serde_json::to_vec(&IpcMessage::HistoryResponse(history)).unwrap();
                                        let _ = stream.write_all(&resp_bytes);
                                    }
                                    Err(e) => {
                                        let resp_bytes = serde_json::to_vec(&IpcMessage::Log(format!("Error fetching history: {}", e))).unwrap();
                                        let _ = stream.write_all(&resp_bytes);
                                    }
                                }
                            }
                            IpcMessage::GetState { key } => {
                                match store.get_state(&key) {
                                    Ok(val) => {
                                        let resp_bytes = serde_json::to_vec(&IpcMessage::StateResponse(val)).unwrap();
                                        let _ = stream.write_all(&resp_bytes);
                                    }
                                    Err(e) => {
                                        let resp_bytes = serde_json::to_vec(&IpcMessage::Log(format!("Error fetching state: {}", e))).unwrap();
                                        let _ = stream.write_all(&resp_bytes);
                                    }
                                }
                            }
                            IpcMessage::SetState { key, value } => {
                                match store.set_state(&key, &value) {
                                    Ok(_) => {
                                        let resp_bytes = serde_json::to_vec(&IpcMessage::Log("State set successfully".to_string())).unwrap();
                                        let _ = stream.write_all(&resp_bytes);
                                    }
                                    Err(e) => {
                                        let resp_bytes = serde_json::to_vec(&IpcMessage::Log(format!("Error setting state: {}", e))).unwrap();
                                        let _ = stream.write_all(&resp_bytes);
                                    }
                                }
                            }
                            IpcMessage::SetClipboard { content, timestamp, source } => {
                                let _ = tx.send(IpcMessage::SetClipboard { content, timestamp, source });
                                let resp_bytes = serde_json::to_vec(&IpcMessage::Log("Clipboard set request queued".to_string())).unwrap();
                                let _ = stream.write_all(&resp_bytes);
                            }
                            _ => {
                                let resp_bytes = serde_json::to_vec(&IpcMessage::Log("Message received".to_string())).unwrap();
                                let _ = stream.write_all(&resp_bytes);
                            }
                        }
                    }
                }
            }
            Err(e) => error!("IPC stream error: {}", e),
        }
    }
}

fn daemon_loop(tx: Sender<IpcMessage>, rx: Receiver<IpcMessage>, iterations: Option<usize>, store: Arc<Store>, last_written: Arc<Mutex<Option<String>>>, discovered_devices: Arc<Mutex<Vec<(String, String, String, String, u16)>>>, _active_pairing: Arc<Mutex<Option<ActivePairingState>>>, sync_manager: Arc<SyncManager>, pm: Arc<PairingManager>, last_processed_timestamp: Arc<Mutex<u64>>) {
    info!("Daemon logic thread started");
    use arboard::Clipboard;
    
    let mut clipboard = Clipboard::new().ok();
    
    let mut count = 0;
    loop {
        if let Some(max) = iterations {
            if count >= max { break; }
        }
        
        if let Ok(msg) = rx.try_recv() {
            info!("Daemon processing: {:?}", msg);
            match msg {
                IpcMessage::Ping => {
                    let _ = tx.send(IpcMessage::Pong);
                }
                IpcMessage::ClipboardChanged { content, timestamp } => {
                    let mut last_ts = last_processed_timestamp.lock().unwrap();
                    if timestamp > *last_ts {
                        *last_ts = timestamp;
                        let _ = store.set_state("last_sync_timestamp", &timestamp.to_string());
                        
                        info!("Syncing clipboard content: {}", content);
                        if let Err(e) = store.append_event(content.as_bytes(), "Local") {
                            error!("Failed to store clipboard event: {}", e);
                        }
                        // Broadcast to peers
                        sync_manager.broadcast(SyncMessage::ClipboardUpdate { content, timestamp });
                    } else {
                        info!("Ignoring outdated clipboard change");
                    }
                }
                IpcMessage::SetClipboard { content, timestamp, source } => {
                    let mut last_ts = last_processed_timestamp.lock().unwrap();
                    if timestamp > *last_ts {
                        *last_ts = timestamp;
                        let _ = store.set_state("last_sync_timestamp", &timestamp.to_string());

                        info!("Writing to clipboard from {}: {}", source, content);
                        
                        // Append to local history as well
                        if let Err(e) = store.append_event(content.as_bytes(), &source) {
                            error!("Failed to store received clipboard event: {}", e);
                        }

                        if let Some(ref mut cb) = clipboard {
                            {
                                let mut lw = last_written.lock().unwrap();
                                *lw = Some(content.clone());
                            }
                            if let Err(e) = cb.set_text(content) {
                                error!("Failed to write to clipboard: {}", e);
                                let mut lw = last_written.lock().unwrap();
                                *lw = None;
                            }
                        } else {
                            clipboard = Clipboard::new().ok();
                            error!("Clipboard not available in daemon loop");
                        }
                    } else {
                        info!("Ignoring outdated SetClipboard request from {}", source);
                    }
                }
                IpcMessage::DeviceDiscovered { node_id, label, os, ip, port } => {
                    {
                        let mut list = discovered_devices.lock().unwrap();
                        if !list.iter().any(|(id, _, _, _, _)| id == &node_id) {
                            list.push((node_id.clone(), label, os, ip.clone(), port));
                        }
                    }

                    // Auto-connect to trusted peers
                    if !sync_manager.is_connected(&node_id) {
                        if let Ok(true) = store.is_device_paired(&node_id) {
                            if let Ok(ip_addr) = ip.parse() {
                                let addr = SocketAddr::new(ip_addr, port);
                                let pm_init = Arc::clone(&pm);
                                info!("Auto-connecting to trusted peer {} at {}", node_id, addr);
                                let pm_clone = Arc::clone(&pm_init);
                                thread::spawn(move || {
                                    pm_clone.initiate_pairing(addr);
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        thread::sleep(Duration::from_millis(100));
        count += 1;
    }
}

fn clipboard_watcher(tx: Sender<IpcMessage>, last_written: Arc<Mutex<Option<String>>>) {
    use arboard::Clipboard;
    
    let mut clipboard = match Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to initialize clipboard: {}", e);
            return;
        }
    };

    let mut last_content = clipboard.get_text().unwrap_or_default();
    info!("Clipboard watcher initialized with initial content length: {}", last_content.len());

    loop {
        if let Ok(current_content) = clipboard.get_text() {
            if current_content != last_content {
                let mut lw = last_written.lock().unwrap();
                if let Some(ref val) = *lw {
                    if val == &current_content {
                        info!("Ignoring clipboard change (self-triggered)");
                        last_content = current_content;
                        *lw = None;
                        continue;
                    }
                }
                
                info!("Clipboard change detected");
                last_content = current_content.clone();
                let _ = tx.send(IpcMessage::ClipboardChanged { content: current_content, timestamp: now_ms() });
            }
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn install_service() {
    info!("Installing CDUS Agent as systemd user service...");
    
    let exe_path = std::env::current_exe().expect("Failed to get current executable path");
    let service_content = format!(
        r#"[Unit]
Description=CDUS Agent Daemon
After=network.target

[Service]
ExecStart={}
Restart=on-failure

[Install]
WantedBy=default.target
"#,
        exe_path.display()
    );

    let config_dir = ProjectDirs::from("com", "cdus", "agent")
        .expect("Failed to get config directory");
    let systemd_user_dir = config_dir.config_dir().parent().unwrap().join("systemd/user");
    
    std::fs::create_dir_all(&systemd_user_dir).expect("Failed to create systemd user directory");
    let service_file_path = systemd_user_dir.join("cdus-agent.service");

    std::fs::write(&service_file_path, service_content).expect("Failed to write service file");
    info!("Service file written to {}", service_file_path.display());

    run_command("systemctl", &["--user", "daemon-reload"]);
    run_command("systemctl", &["--user", "enable", "cdus-agent.service"]);
    run_command("systemctl", &["--user", "start", "cdus-agent.service"]);
    
    info!("CDUS Agent service installed and started.");
}

fn uninstall_service() {
    info!("Uninstalling CDUS Agent systemd user service...");
    
    run_command("systemctl", &["--user", "stop", "cdus-agent.service"]);
    run_command("systemctl", &["--user", "disable", "cdus-agent.service"]);

    let config_dir = ProjectDirs::from("com", "cdus", "agent")
        .expect("Failed to get config directory");
    let systemd_user_dir = config_dir.config_dir().parent().unwrap().join("systemd/user");
    let service_file_path = systemd_user_dir.join("cdus-agent.service");

    if service_file_path.exists() {
        std::fs::remove_file(&service_file_path).expect("Failed to remove service file");
        info!("Service file removed.");
    }

    run_command("systemctl", &["--user", "daemon-reload"]);
    info!("CDUS Agent service uninstalled.");
}

fn run_command(cmd: &str, args: &[&str]) {
    let status = std::process::Command::new(cmd)
        .args(args)
        .status()
        .expect(&format!("Failed to execute {}", cmd));
    if !status.success() {
        error!("Command {} {:?} failed", cmd, args);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daemon_loop_ping_pong() {
        let (agent_tx, daemon_rx) = flume::unbounded();
        let (daemon_tx, agent_rx) = flume::unbounded();
        
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        let store = Arc::new(store);
        let lw = Arc::new(Mutex::new(None));
        let dd = Arc::new(Mutex::new(Vec::new()));
        let ap = Arc::new(Mutex::new(None));
        let pm = PairingManager::new(Arc::clone(&store), daemon_tx.clone(), "test".to_string(), vec![], 0, Arc::clone(&ap), Arc::new(SyncManager::new()));
        let pm = Arc::new(pm);
        let lpt = Arc::new(Mutex::new(0u64));
        
        agent_tx.send(IpcMessage::Ping).unwrap();
        daemon_loop(daemon_tx, daemon_rx, Some(5), store, lw, dd, ap, Arc::new(SyncManager::new()), pm, lpt);
        
        let resp = agent_rx.try_recv().expect("Should have received a Pong");
        assert_eq!(resp, IpcMessage::Pong);
    }
}
