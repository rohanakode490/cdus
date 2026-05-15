use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use flume::{Receiver, Sender};
use interprocess::local_socket::LocalSocketListener;
use once_cell::sync::Lazy;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{error, info};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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

use cdus_common::{IpcMessage, SyncMessage, TransferProgress, TransportType};
use libp2p_manager::Libp2pManager;
use mdns::MdnsManager;
use pairing::{ActivePairingState, PairingManager, SyncManager};
use relay::RelayManager;
use store::Store;
use turn_manager::TurnManager;

mod file_transfer;
mod integration_tests;
mod libp2p_manager;
mod mdns;
mod pairing;
mod relay;
mod store;
mod turn_manager;
mod utils;

static EVENT_BUS: Lazy<Arc<Mutex<Vec<Sender<IpcMessage>>>>> =
    Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

fn broadcast_event(msg: IpcMessage) {
    let mut bus = EVENT_BUS.lock().unwrap();
    bus.retain(|tx: &Sender<IpcMessage>| tx.send(msg.clone()).is_ok());
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
        let config_dir =
            ProjectDirs::from("com", "cdus", "agent").expect("Failed to get config directory");
        data_dir_buf = config_dir.data_dir().to_path_buf();
        &data_dir_buf
    };

    std::fs::create_dir_all(data_dir).expect("Failed to create data directory");

    let store = Store::init(data_dir).expect("Failed to initialize store");
    let store = Arc::new(store);

    let active_pairing = Arc::new(Mutex::new(None::<ActivePairingState>));
    let active_pairing_daemon = Arc::clone(&active_pairing);

    // Initialize or load device identity
    let (node_id, private_key) = store
        .get_or_create_identity(data_dir)
        .expect("Failed to initialize identity");
    let label = store
        .get_state("device_name")
        .unwrap()
        .unwrap_or_else(|| "Unknown".to_string());
    info!("Device identity initialized. Node ID: {}", node_id);

    // Initialize Relay Manager
    let (tx, rx) = flume::unbounded::<IpcMessage>();
    let (relay, relay_rx) = RelayManager::new(node_id.clone(), cli.relay_url.clone(), tx.clone());
    let relay = Arc::new(relay);
    if let Err(e) = relay.register() {
        error!(
            "Failed to register with relay: {}. Will retry connection in background loop.",
            e
        );
    }
    relay.start_signaling_loop(relay_rx);

    // Initialize Turn Manager
    let turn_manager = Arc::new(TurnManager::new().expect("Failed to initialize TurnManager"));

    let active_transfers: Arc<
        Mutex<std::collections::HashMap<String, (std::path::PathBuf, cdus_common::FileManifest)>>,
    > = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let active_transfers_libp2p = Arc::clone(&active_transfers);
    let active_transfers_daemon = Arc::clone(&active_transfers);

    let received_manifests: Arc<Mutex<std::collections::HashMap<String, TransferProgress>>> =
        Arc::new(Mutex::new(std::collections::HashMap::new()));
    let received_manifests_libp2p = Arc::clone(&received_manifests);
    let received_manifests_daemon = Arc::clone(&received_manifests);

    // Initialize Libp2p Manager
    let libp2p_manager = Arc::new(
        Libp2pManager::new(
            private_key.clone(),
            tx.clone(),
            Arc::clone(&store),
            active_transfers_libp2p,
            received_manifests_libp2p,
        )
        .expect("Failed to initialize Libp2pManager"),
    );
    libp2p_manager.start();
    let libp2p_sync_tx = libp2p_manager.get_sync_tx();

    // Start mDNS registration
    let mdns = MdnsManager::new();
    mdns.register_device(&node_id, &label, cli.port);
    let mdns = Arc::new(mdns);

    // Auto-start discovery in background to find paired devices
    info!("Starting background mDNS discovery for auto-reconnect...");
    mdns.start_discovery(tx.clone());

    let sync_manager = Arc::new(SyncManager::new());
    sync_manager.add_peer(
        "libp2p_broadcast".to_string(),
        libp2p_sync_tx,
        TransportType::P2p,
    );
    let sync_manager_daemon = Arc::clone(&sync_manager);

    // Start Pairing Manager
    let pm = PairingManager::new(
        Arc::clone(&store),
        tx.clone(),
        node_id,
        private_key,
        cli.port,
        Arc::clone(&active_pairing),
        Arc::clone(&sync_manager),
        Arc::clone(&relay),
        Arc::clone(&turn_manager),
    );
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

    let discovered_devices = Arc::new(Mutex::new(
        Vec::<(String, String, String, String, u16)>::new(),
    ));
    let discovered_devices_daemon = Arc::clone(&discovered_devices);

    // Start daemon logic thread
    let daemon_tx = tx.clone();
    let last_written_daemon = Arc::clone(&last_written);
    let daemon_store = Arc::clone(&store);
    let pm_daemon_loop = Arc::clone(&pm);
    let last_ts_daemon = Arc::clone(&last_processed_timestamp);
    let libp2p_request_tx_daemon = libp2p_manager.get_request_tx();
    thread::spawn(move || {
        daemon_loop(
            daemon_tx,
            rx,
            None,
            daemon_store,
            last_written_daemon,
            discovered_devices_daemon,
            active_pairing_daemon,
            sync_manager_daemon,
            pm_daemon_loop,
            last_ts_daemon,
            Some(libp2p_request_tx_daemon),
            active_transfers_daemon,
            received_manifests_daemon,
        );
    });

    // Setup IPC listener
    let socket_name = cli.socket.clone();
    let _ = std::fs::remove_file(&socket_name);
    let listener = LocalSocketListener::bind(&*socket_name).expect("Failed to bind local socket");

    info!("IPC Listener bound to {}", socket_name);

    let sync_manager_ipc = Arc::clone(&sync_manager);
    let relay_ipc = Arc::clone(&relay);

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let tx_clone = tx.clone();
                let pm_clone = Arc::clone(&pm);
                let discovered_devices_clone = Arc::clone(&discovered_devices);
                let active_pairing_clone = Arc::clone(&active_pairing);
                let sync_manager_ipc = Arc::clone(&sync_manager);
                let relay_ipc = Arc::clone(&relay);
                let store_clone = Arc::clone(&store);
                let mdns_clone = Arc::clone(&mdns);

                thread::spawn(move || {
                    let mut buffer = [0u8; 4096];
                    loop {
                        match stream.read(&mut buffer) {
                            Ok(0) => break,
                            Ok(n) => {
                                if let Ok(msg) = serde_json::from_slice::<IpcMessage>(&buffer[..n])
                                {
                                    match msg {
                                        IpcMessage::Ping => {
                                            let resp_bytes =
                                                serde_json::to_vec(&IpcMessage::Pong).unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::ListenEvents => {
                                            info!("IPC: Client subscribed to event stream");
                                            let (event_tx, event_rx) = flume::unbounded();
                                            {
                                                let mut bus = EVENT_BUS.lock().unwrap();
                                                bus.push(event_tx);
                                            }
                                            while let Ok(event) = event_rx.recv() {
                                                if let Ok(mut bytes) = serde_json::to_vec(&event) {
                                                    // Add a newline as a simple delimiter for multiple JSON messages
                                                    bytes.push(b'\n');
                                                    if let Err(_) = stream.write_all(&bytes) {
                                                        break;
                                                    }
                                                }
                                            }
                                            break;
                                        }
                                        IpcMessage::StartScan => {
                                            info!("IPC: Received StartScan request");
                                            {
                                                let mut list =
                                                    discovered_devices_clone.lock().unwrap();
                                                list.clear();
                                            }
                                            mdns_clone.start_discovery(tx_clone.clone());
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "Scan started".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::StopScan => {
                                            info!("IPC: Received StopScan request");
                                            mdns_clone.stop_discovery();
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "Scan stopped".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::GetDiscovered => {
                                            let list = discovered_devices_clone.lock().unwrap();
                                            let resp_bytes = serde_json::to_vec(
                                                &IpcMessage::DiscoveredResponse(list.clone()),
                                            )
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::PairWith { node_id } => {
                                            let list = discovered_devices_clone.lock().unwrap();
                                            if let Some((_, _, _, ip, port)) =
                                                list.iter().find(|(id, _, _, _, _)| id == &node_id)
                                            {
                                                if let Ok(ip_addr) = ip.parse() {
                                                    let addr = SocketAddr::new(ip_addr, *port);
                                                    let pm_init = Arc::clone(&pm_clone);
                                                    thread::spawn(move || {
                                                        pm_init.initiate_pairing(addr);
                                                    });
                                                    let resp_bytes =
                                                        serde_json::to_vec(&IpcMessage::Log(
                                                            "Pairing initiated".to_string(),
                                                        ))
                                                        .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::PairWithIp { ip, port } => {
                                            if let Ok(ip_addr) = ip.parse() {
                                                let addr = SocketAddr::new(ip_addr, port);
                                                let pm_init = Arc::clone(&pm_clone);
                                                thread::spawn(move || {
                                                    pm_init.initiate_pairing(addr);
                                                });
                                                let resp_bytes =
                                                    serde_json::to_vec(&IpcMessage::Log(
                                                        "Manual pairing initiated".to_string(),
                                                    ))
                                                    .unwrap();
                                                let _ = stream.write_all(&resp_bytes);
                                            }
                                        }
                                        IpcMessage::PairWithRemote { uuid } => {
                                            let pm_init = Arc::clone(&pm_clone);
                                            thread::spawn(move || {
                                                pm_init.initiate_remote_pairing(uuid);
                                            });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "Remote pairing initiated".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::ConfirmPairing(accepted) => {
                                            let ap = active_pairing_clone.lock().unwrap();
                                            if let Some(ref state) = *ap {
                                                let mut res = state.confirmed.lock().unwrap();
                                                *res = Some(accepted);
                                            }
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                format!("Pairing result processed: {}", accepted),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::SendFile { node_id, path } => {
                                            let _ = tx_clone
                                                .send(IpcMessage::SendFile { node_id, path });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "File transfer initiated".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::AcceptFileTransfer { file_hash } => {
                                            let _ = tx_clone
                                                .send(IpcMessage::AcceptFileTransfer { file_hash });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "File transfer accepted".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::RejectFileTransfer { file_hash } => {
                                            let _ = tx_clone
                                                .send(IpcMessage::RejectFileTransfer { file_hash });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "File transfer rejected".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::GetPairingStatus => {
                                            let ap = active_pairing_clone.lock().unwrap();
                                            let resp = match *ap {
                                                Some(ref state) => {
                                                    IpcMessage::PairingStatusResponse {
                                                        pin: Some(state.pin.clone()),
                                                        active: true,
                                                        is_initiator: state.is_initiator,
                                                        remote_label: state.remote_label.clone(),
                                                    }
                                                }
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
                                            match store_clone.get_paired_devices() {
                                                Ok(devices) => {
                                                    let merged_devices: Vec<(
                                                        String,
                                                        String,
                                                        Option<TransportType>,
                                                    )> = devices
                                                        .into_iter()
                                                        .map(|(id, label)| {
                                                            let transport = sync_manager_ipc
                                                                .get_peer_transport(&id);
                                                            (id, label, transport)
                                                        })
                                                        .collect();
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::PairedDevicesResponse(
                                                            merged_devices,
                                                        ),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!(
                                                            "Error fetching paired devices: {}",
                                                            e
                                                        )),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::UnpairDevice { node_id } => {
                                            match store_clone.remove_paired_device(&node_id) {
                                                Ok(_) => {
                                                    let resp_bytes =
                                                        serde_json::to_vec(&IpcMessage::Log(
                                                            "Device unpaired".to_string(),
                                                        ))
                                                        .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!(
                                                            "Error unpairing device: {}",
                                                            e
                                                        )),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::RevokeDevice { uuid } => {
                                            let _ = relay_ipc.revoke_device(uuid.clone());
                                            match store_clone.remove_paired_device(&uuid) {
                                                Ok(_) => {
                                                    sync_manager_ipc.remove_peer(&uuid);
                                                    let resp_bytes =
                                                        serde_json::to_vec(&IpcMessage::Log(
                                                            "Device revoked and unpaired"
                                                                .to_string(),
                                                        ))
                                                        .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!(
                                                            "Error removing revoked device: {}",
                                                            e
                                                        )),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::GetHistory { limit } => {
                                            match store_clone.get_recent_events(limit) {
                                                Ok(history) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::HistoryResponse(history),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!(
                                                            "Error fetching history: {}",
                                                            e
                                                        )),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::GetState { key } => {
                                            match store_clone.get_state(&key) {
                                                Ok(val) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::StateResponse(val),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes =
                                                        serde_json::to_vec(&IpcMessage::Log(
                                                            format!("Error fetching state: {}", e),
                                                        ))
                                                        .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::SetState { key, value } => {
                                            match store_clone.set_state(&key, &value) {
                                                Ok(_) => {
                                                    let resp_bytes =
                                                        serde_json::to_vec(&IpcMessage::Log(
                                                            "State set successfully".to_string(),
                                                        ))
                                                        .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes =
                                                        serde_json::to_vec(&IpcMessage::Log(
                                                            format!("Error setting state: {}", e),
                                                        ))
                                                        .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::SetClipboard {
                                            content,
                                            timestamp,
                                            source,
                                        } => {
                                            let _ = tx_clone.send(IpcMessage::SetClipboard {
                                                content,
                                                timestamp,
                                                source,
                                            });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "Clipboard set request queued".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        _ => {
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "Message received".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                    }
                                }
                                break;
                            }
                            Err(_) => break,
                        }
                    }
                });
            }
            Err(e) => error!("IPC stream error: {}", e),
        }
    }
}

fn daemon_loop(
    tx: Sender<IpcMessage>,
    rx: Receiver<IpcMessage>,
    iterations: Option<usize>,
    store: Arc<Store>,
    last_written: Arc<Mutex<Option<String>>>,
    discovered_devices: Arc<Mutex<Vec<(String, String, String, String, u16)>>>,
    _active_pairing: Arc<Mutex<Option<ActivePairingState>>>,
    sync_manager: Arc<SyncManager>,
    pm: Arc<PairingManager>,
    last_processed_timestamp: Arc<Mutex<u64>>,
    libp2p_request_tx: Option<Sender<(libp2p::PeerId, SyncMessage)>>,
    active_transfers: Arc<
        Mutex<std::collections::HashMap<String, (std::path::PathBuf, cdus_common::FileManifest)>>,
    >,
    received_manifests: Arc<Mutex<std::collections::HashMap<String, TransferProgress>>>,
) {
    info!("Daemon logic thread started");
    #[cfg(not(target_os = "android"))]
    use arboard::Clipboard;

    #[cfg(not(target_os = "android"))]
    let mut clipboard = Clipboard::new().ok();

    let mut count = 0;
    loop {
        if let Some(max) = iterations {
            if count >= max {
                break;
            }
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
                        sync_manager.broadcast(SyncMessage::ClipboardUpdate {
                            content: content.clone(),
                            timestamp,
                        });
                        broadcast_event(IpcMessage::ClipboardChanged { content, timestamp });
                    } else {
                        info!("Ignoring outdated clipboard change");
                    }
                }
                IpcMessage::SetClipboard {
                    content,
                    timestamp,
                    source,
                } => {
                    let mut last_ts = last_processed_timestamp.lock().unwrap();
                    if timestamp > *last_ts {
                        *last_ts = timestamp;
                        let _ = store.set_state("last_sync_timestamp", &timestamp.to_string());

                        info!("Writing to clipboard from {}: {}", source, content);

                        // Append to local history as well
                        if let Err(e) = store.append_event(content.as_bytes(), &source) {
                            error!("Failed to store received clipboard event: {}", e);
                        }

                        #[cfg(not(target_os = "android"))]
                        {
                            if let Some(ref mut cb) = clipboard {
                                {
                                    let mut lw = last_written.lock().unwrap();
                                    *lw = Some(content.clone());
                                }
                                if let Err(e) = cb.set_text(content.clone()) {
                                    error!("Failed to write to clipboard: {}", e);
                                    let mut lw = last_written.lock().unwrap();
                                    *lw = None;
                                }
                            } else {
                                clipboard = Clipboard::new().ok();
                                error!("Clipboard not available in daemon loop");
                            }
                        }
                        broadcast_event(IpcMessage::SetClipboard {
                            content,
                            timestamp,
                            source,
                        });
                    } else {
                        info!("Ignoring outdated SetClipboard request from {}", source);
                    }
                }
                IpcMessage::DeviceDiscovered {
                    node_id,
                    label,
                    os,
                    ip,
                    port,
                } => {
                    {
                        let mut list = discovered_devices.lock().unwrap();
                        if !list.iter().any(|(id, _, _, _, _)| id == &node_id) {
                            list.push((
                                node_id.clone(),
                                label.clone(),
                                os.clone(),
                                ip.clone(),
                                port,
                            ));
                        }
                    }
                    broadcast_event(IpcMessage::DeviceDiscovered {
                        node_id: node_id.clone(),
                        label: label.clone(),
                        os,
                        ip: ip.clone(),
                        port,
                    });

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
                IpcMessage::DeviceLost { node_id } => {
                    let mut list = discovered_devices.lock().unwrap();
                    list.retain(|(id, _, _, _, _)| !id.starts_with(&node_id));
                    info!(
                        "Removed device from discovery list: {} (or prefix)",
                        node_id
                    );
                    broadcast_event(IpcMessage::DeviceLost { node_id });
                }
                IpcMessage::RelayMessage {
                    source_uuid,
                    payload,
                } => {
                    pm.handle_relay_message(source_uuid, payload);
                }
                IpcMessage::SendFile { node_id, path } => {
                    let path_buf = std::path::PathBuf::from(path);
                    let sync_manager_clone = Arc::clone(&sync_manager);
                    let tx_clone = tx.clone();
                    let active_transfers_clone = Arc::clone(&active_transfers);
                    thread::spawn(move || {
                        info!("Generating manifest for {:?}", path_buf);
                        match file_transfer::generate_manifest(&path_buf) {
                            Ok(manifest) => {
                                info!("Manifest generated, sending request to {}", node_id);
                                let file_hash = manifest.file_hash.clone();
                                {
                                    let mut at = active_transfers_clone.lock().unwrap();
                                    at.insert(file_hash, (path_buf, manifest.clone()));
                                }

                                let msg = SyncMessage::FileTransferRequest(manifest);
                                if node_id == "all" || node_id == "unknown" {
                                    sync_manager_clone.broadcast(msg);
                                } else {
                                    if !sync_manager_clone.send_to_peer(&node_id, msg.clone()) {
                                        sync_manager_clone.broadcast(msg);
                                    }
                                }
                            }
                            Err(e) => {
                                error!("Failed to generate manifest: {}", e);
                                let _ = tx_clone.send(IpcMessage::FileTransferError {
                                    file_hash: "unknown".to_string(),
                                    error: e.to_string(),
                                });
                            }
                        }
                    });
                }
                IpcMessage::AcceptFileTransfer { file_hash } => {
                    let progress = {
                        let rm = received_manifests.lock().unwrap();
                        rm.get(&file_hash)
                            .map(|p| (p.node_id.clone(), p.manifest.clone()))
                    };

                    if let Some((node_id, manifest)) = progress {
                        info!("Accepting file transfer for {} from {}", file_hash, node_id);
                        sync_manager.broadcast(SyncMessage::FileTransferAccepted {
                            file_hash: file_hash.clone(),
                        });

                        if let Some(ref req_tx) = libp2p_request_tx {
                            if let Ok(peer_id) = node_id.parse::<libp2p::PeerId>() {
                                let req_tx_clone = req_tx.clone();
                                thread::spawn(move || {
                                    for chunk in manifest.chunks {
                                        let _ = req_tx_clone.send((
                                            peer_id,
                                            SyncMessage::ChunkRequest {
                                                file_hash: file_hash.clone(),
                                                chunk_hash: chunk.hash.clone(),
                                            },
                                        ));
                                    }
                                });
                            }
                        }
                    }
                }
                IpcMessage::RejectFileTransfer { file_hash } => {
                    sync_manager.broadcast(SyncMessage::FileTransferRejected { file_hash });
                }
                IpcMessage::IncomingFileRequest { node_id, manifest } => {
                    let file_hash = manifest.file_hash.clone();
                    {
                        let mut rm = received_manifests.lock().unwrap();
                        rm.insert(
                            file_hash.clone(),
                            TransferProgress {
                                node_id: node_id.clone(),
                                manifest: manifest.clone(),
                                completed_hashes: std::collections::HashSet::new(),
                            },
                        );
                    }
                    broadcast_event(IpcMessage::IncomingFileRequest { node_id, manifest });
                }
                IpcMessage::FileTransferProgress {
                    file_hash,
                    progress,
                } => {
                    broadcast_event(IpcMessage::FileTransferProgress {
                        file_hash,
                        progress,
                    });
                }
                IpcMessage::FileTransferComplete { file_hash } => {
                    broadcast_event(IpcMessage::FileTransferComplete { file_hash });
                }
                IpcMessage::FileTransferError { file_hash, error } => {
                    broadcast_event(IpcMessage::FileTransferError { file_hash, error });
                }
                IpcMessage::ChunkReceived {
                    file_hash,
                    chunk_hash,
                    data,
                } => {
                    let mut rm = received_manifests.lock().unwrap();
                    if let Some(progress) = rm.get_mut(&file_hash) {
                        if let Some(chunk) = progress
                            .manifest
                            .chunks
                            .iter()
                            .find(|c| c.hash == chunk_hash)
                        {
                            let actual_hash = blake3::hash(&data).to_string();
                            if actual_hash == chunk_hash {
                                let mut path = std::env::temp_dir();
                                path.push(format!("{}.part", file_hash));

                                let mut file = std::fs::OpenOptions::new()
                                    .create(true)
                                    .write(true)
                                    .open(&path)
                                    .expect("Failed to open part file");

                                use std::io::{Seek, Write};
                                file.seek(std::io::SeekFrom::Start(chunk.offset))
                                    .expect("Failed to seek");
                                file.write_all(&data).expect("Failed to write chunk");

                                progress.completed_hashes.insert(chunk_hash);

                                let total = progress.manifest.chunks.len();
                                let completed = progress.completed_hashes.len();
                                let percent = (completed as f32 / total as f32) * 100.0;
                                info!(
                                    "File {} progress: {}/{} ({:.2}%)",
                                    file_hash, completed, total, percent
                                );

                                let _ = tx.send(IpcMessage::FileTransferProgress {
                                    file_hash: file_hash.clone(),
                                    progress: percent,
                                });

                                if completed == total {
                                    info!("File {} download complete!", file_hash);
                                    let mut final_path = std::env::current_dir().unwrap();
                                    final_path.push(&progress.manifest.file_name);
                                    let _ = std::fs::rename(&path, &final_path);
                                    let _ = tx.send(IpcMessage::FileTransferComplete {
                                        file_hash: file_hash.clone(),
                                    });
                                }
                            }
                        }
                    }

                    // Separate check for removal
                    let mut complete = false;
                    if let Some(p) = rm.get(&file_hash) {
                        if p.completed_hashes.len() == p.manifest.chunks.len() {
                            complete = true;
                        }
                    }
                    if complete {
                        rm.remove(&file_hash);
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
    #[cfg(not(target_os = "android"))]
    {
        use arboard::Clipboard;

        let mut clipboard = match Clipboard::new() {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to initialize clipboard: {}", e);
                return;
            }
        };

        let mut last_content = clipboard.get_text().unwrap_or_default();
        info!(
            "Clipboard watcher initialized with initial content length: {}",
            last_content.len()
        );

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
                    let _ = tx.send(IpcMessage::ClipboardChanged {
                        content: current_content,
                        timestamp: now_ms(),
                    });
                }
            }
            thread::sleep(Duration::from_secs(1));
        }
    }
    #[cfg(target_os = "android")]
    {
        info!("Clipboard watcher not implemented for Android yet");
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

    let config_dir =
        ProjectDirs::from("com", "cdus", "agent").expect("Failed to get config directory");
    let systemd_user_dir = config_dir
        .config_dir()
        .parent()
        .unwrap()
        .join("systemd/user");

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

    let config_dir =
        ProjectDirs::from("com", "cdus", "agent").expect("Failed to get config directory");
    let systemd_user_dir = config_dir
        .config_dir()
        .parent()
        .unwrap()
        .join("systemd/user");
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
        let tm = Arc::new(TurnManager::new().unwrap());
        let (relay, _) = RelayManager::new(
            "test".to_string(),
            "http://localhost".to_string(),
            daemon_tx.clone(),
        );
        let pm = PairingManager::new(
            Arc::clone(&store),
            daemon_tx.clone(),
            "test".to_string(),
            vec![],
            0,
            Arc::clone(&ap),
            Arc::new(SyncManager::new()),
            Arc::new(relay),
            tm,
        );
        let pm = Arc::new(pm);
        let lpt = Arc::new(Mutex::new(0u64));

        agent_tx.send(IpcMessage::Ping).unwrap();
        let at = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let rm = Arc::new(Mutex::new(std::collections::HashMap::new()));
        daemon_loop(
            daemon_tx,
            daemon_rx,
            Some(5),
            store,
            lw,
            dd,
            ap,
            Arc::new(SyncManager::new()),
            pm,
            lpt,
            None,
            at,
            rm,
        );

        let resp = agent_rx.try_recv().expect("Should have received a Pong");
        assert_eq!(resp, IpcMessage::Pong);
    }
}
