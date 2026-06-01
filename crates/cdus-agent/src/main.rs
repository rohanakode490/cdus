use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use flume::Sender;
use interprocess::local_socket::LocalSocketListener;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::sync::Arc; use parking_lot::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{error, info};

use cdus_agent::{broadcast_event, daemon_loop, EVENT_BUS};
use cdus_common::{IpcMessage, TransportType};
use cdus_agent::libp2p_manager::Libp2pManager;
use cdus_agent::mdns::MdnsManager;
use cdus_agent::pairing::{ActivePairingState, PairingManager, SyncManager};
use cdus_agent::relay::RelayManager;
use cdus_agent::store::Store;
use cdus_agent::turn_manager::TurnManager;

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
    let relay_rx_opt = Arc::new(Mutex::new(Some(relay_rx)));

    // Auto-connect to relay on startup
    let relay_clone = Arc::clone(&relay);
    let relay_rx_opt_init = Arc::clone(&relay_rx_opt);
    thread::spawn(move || {
        info!("Auto-connecting to relay on startup...");
        if let Err(e) = relay_clone.register() {
            error!("Failed to register with relay: {}", e);
        }
        let rx = {
            let mut opt = relay_rx_opt_init.lock();
            opt.take()
        };
        if let Some(rx) = rx {
            relay_clone.start_signaling_loop(rx);
        }
    });

    // Initialize Turn Manager
    let turn_manager = Arc::new(TurnManager::new().expect("Failed to initialize TurnManager"));

    // Phase 5.3: Stale transfer cleanup
    if let Err(e) = cdus_agent::file_transfer::cleanup_stale_transfers(&store) {
        error!("Failed to cleanup stale transfers: {}", e);
    }

    let (progress_tx, progress_rx) = flume::unbounded();
    let transfer_manager = Arc::new(cdus_agent::file_transfer::FileTransferManager::new(Arc::clone(&store), progress_tx));

    // Start progress forwarder to broadcast events to all IPC clients
    let _progress_tx_for_forwarder = tx.clone();
    thread::spawn(move || {
        while let Ok(event) = progress_rx.recv() {
            broadcast_event(IpcMessage::FileProgress(event));
        }
    });

    // Initialize Libp2p Manager
    let libp2p_manager = Arc::new(
        Libp2pManager::new(
            private_key.clone(),
            tx.clone(),
            Arc::clone(&store),
            Arc::clone(&transfer_manager),
        )
        .expect("Failed to initialize Libp2pManager"),
    );
    libp2p_manager.start();
    let libp2p_sync_tx = libp2p_manager.get_sync_tx();

    // Initialize mDNS Manager
    let mdns = MdnsManager::new();
    let mdns = Arc::new(mdns);

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
        node_id.clone(),
        private_key,
        cli.port,
        Arc::clone(&active_pairing),
        Arc::clone(&sync_manager),
        Arc::clone(&relay),
        Arc::clone(&turn_manager),
        Arc::clone(&libp2p_manager),
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
            *last_processed_timestamp.lock() = ts;
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
        Vec::<(String, String, String, Vec<String>, u16)>::new(),
    ));
    let peer_map = Arc::new(Mutex::new(std::collections::HashMap::<String, (String, String, Vec<String>, u16, std::time::Instant)>::new()));
    
    // Populate peer_map from store for paired devices to enable reconnection after restart
    if let Ok(paired_devices) = store.get_paired_devices() {
        let mut map = peer_map.lock();
        for record in paired_devices {
            if let (Some(ips), Some(port)) = (record.last_known_ips, record.last_known_port) {
                map.insert(
                    record.node_id,
                    (
                        record.label,
                        "Unknown".to_string(),
                        ips,
                        port,
                        std::time::Instant::now() - std::time::Duration::from_secs(3600),
                    ),
                );
            }
        }
    }

    let discovered_devices_daemon = Arc::clone(&discovered_devices);
    let peer_map_daemon = Arc::clone(&peer_map);

    // Start daemon logic thread
    let daemon_tx = tx.clone();
    let last_written_daemon = Arc::clone(&last_written);
    let daemon_store = Arc::clone(&store);
    let pm_daemon_loop = Arc::clone(&pm);
    let last_ts_daemon = Arc::clone(&last_processed_timestamp);
    let libp2p_request_tx_daemon = libp2p_manager.get_request_tx();
    let libp2p_manager_daemon = Arc::clone(&libp2p_manager);
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
            peer_map_daemon,
            Some(libp2p_request_tx_daemon),
            libp2p_manager_daemon,
        );
    });

    // Setup IPC listener
    let socket_name = cli.socket.clone();
    let _ = std::fs::remove_file(&socket_name);
    let listener = LocalSocketListener::bind(&*socket_name).expect("Failed to bind local socket");

    info!("IPC Listener bound to {}", socket_name);

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let tx_clone = tx.clone();
                let pm_clone = Arc::clone(&pm);
                let discovered_devices_clone = Arc::clone(&discovered_devices);
                let peer_map_clone = Arc::clone(&peer_map);
                let active_pairing_clone = Arc::clone(&active_pairing);
                let sync_manager_ipc = Arc::clone(&sync_manager);
                let relay_ipc = Arc::clone(&relay);
                let store_clone = Arc::clone(&store);
                let mdns_clone = Arc::clone(&mdns);
                let relay_rx_opt_clone = Arc::clone(&relay_rx_opt);
                let node_id_clone = node_id.clone();
                let label_clone = label.clone();
                let port = cli.port;

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
                                                let mut bus = EVENT_BUS.lock();
                                                bus.push(event_tx);
                                            }
                                            while let Ok(event) = event_rx.recv() {
                                                if let Ok(mut bytes) = serde_json::to_vec(&event) {
                                                    bytes.push(b'\n');
                                                    if stream.write_all(&bytes).is_err() {
                                                        break;
                                                    }
                                                }
                                            }
                                            break;
                                        }
                                        IpcMessage::StartScan => {
                                            info!("IPC: Received StartScan request. Registering device and starting discovery.");
                                            mdns_clone.register_device(&node_id_clone, &label_clone, port);
                                            {
                                                let mut list =
                                                    discovered_devices_clone.lock();
                                                list.clear();
                                            }
                                            mdns_clone.start_discovery(tx_clone.clone());
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "Scan started".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::ConnectRelay => {
                                            info!("IPC: Received ConnectRelay request");
                                            let relay_clone = Arc::clone(&relay_ipc);
                                            let relay_rx_opt = Arc::clone(&relay_rx_opt_clone);
                                            thread::spawn(move || {
                                                if let Err(e) = relay_clone.register() {
                                                    error!("Failed to register with relay: {}", e);
                                                }
                                                let rx = {
                                                    let mut opt = relay_rx_opt.lock();
                                                    opt.take()
                                                };
                                                if let Some(rx) = rx {
                                                    relay_clone.start_signaling_loop(rx);
                                                }
                                            });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "Relay connection initiated".to_string(),
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
                                            let list = discovered_devices_clone.lock();
                                            let resp_bytes = serde_json::to_vec(
                                                &IpcMessage::DiscoveredResponse(list.clone()),
                                            )
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::PairWith { node_id } => {
                                            let device_info = {
                                                let list = discovered_devices_clone.lock();
                                                let found_in_list = list.iter().find(|(id, _, _, _, _)| id == &node_id)
                                                    .map(|(_, _, _, ips, port)| (ips.clone(), *port));

                                                found_in_list.or_else(|| {
                                                    let map = peer_map_clone.lock();
                                                    map.get(&node_id).map(|(_, _, ips, port, _)| (ips.clone(), *port))
                                                })
                                            };

                                            if let Some((ips, port)) = device_info {
                                                let pm_init = Arc::clone(&pm_clone);
                                                let node_id_clone = node_id.clone();
                                                thread::spawn(move || {
                                                    let mut success = false;
                                                    for ip in ips {
                                                        if let Ok(ip_addr) = ip.parse() {
                                                            let addr = SocketAddr::new(ip_addr, port);
                                                            info!("Attempting manual pairing with {} at {}", node_id_clone, addr);
                                                            if pm_init.initiate_pairing(addr) {
                                                                info!("Manual pairing initiated with {} at {}", node_id_clone, addr);
                                                                success = true;
                                                                break;
                                                            }
                                                        }
                                                    }
                                                    
                                                    if !success {
                                                        info!("mDNS failed for {}, falling back to relay", node_id_clone);
                                                        pm_init.initiate_remote_pairing(node_id_clone);
                                                    }
                                                });
                                                let resp_bytes =
                                                    serde_json::to_vec(&IpcMessage::Log(
                                                        "Pairing process started".to_string(),
                                                    ))
                                                    .unwrap();
                                                let _ = stream.write_all(&resp_bytes);
                                            } else {
                                                // Device not in mDNS list at all, try relay immediately
                                                let pm_init = Arc::clone(&pm_clone);
                                                let node_id_clone = node_id.clone();
                                                thread::spawn(move || {
                                                    info!("Device {} not found in local discovery, trying relay", node_id_clone);
                                                    pm_init.initiate_remote_pairing(node_id_clone);
                                                });
                                                let resp_bytes =
                                                    serde_json::to_vec(&IpcMessage::Log(
                                                        "Relay pairing initiated".to_string(),
                                                    ))
                                                    .unwrap();
                                                let _ = stream.write_all(&resp_bytes);
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
                                            let ap = active_pairing_clone.lock();
                                            if let Some(ref state) = *ap {
                                                let mut res = state.confirmed.lock();
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
                                        IpcMessage::AcceptFileTransfer { transfer_id } => {
                                            let _ = tx_clone
                                                .send(IpcMessage::AcceptFileTransfer { transfer_id });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "File transfer accepted".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::RejectFileTransfer { transfer_id } => {
                                            let _ = tx_clone
                                                .send(IpcMessage::RejectFileTransfer { transfer_id });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "File transfer rejected".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::CancelFileTransfer { transfer_id } => {
                                            let _ = tx_clone
                                                .send(IpcMessage::CancelFileTransfer { transfer_id });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "File transfer cancellation requested".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::GetPairingStatus => {
                                            let ap = active_pairing_clone.lock();
                                            let resp = match *ap {
                                                Some(ref state) => {
                                                    IpcMessage::PairingStatusResponse {
                                                        pin: Some(state.pin.clone()),
                                                        active: true,
                                                        is_initiator: state.is_initiator,
                                                        remote_label: state.remote_label.clone(),
                                                        silent: state.silent,
                                                    }
                                                }
                                                None => IpcMessage::PairingStatusResponse {
                                                    pin: None,
                                                    active: false,
                                                    is_initiator: false,
                                                    remote_label: String::new(),
                                                    silent: false,
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
                                                        Option<cdus_common::TransportType>,
                                                    )> = devices
                                                        .into_iter()
                                                        .map(|record| {
                                                            let mut transport = sync_manager_ipc
                                                                .get_peer_transport(&record.node_id);
                                                            if transport.is_none() {
                                                                let map = peer_map_clone.lock();
                                                                if let Some((_, _, _, _, last_seen)) = map.get(&record.node_id) {
                                                                    if last_seen.elapsed() < std::time::Duration::from_secs(30) {
                                                                        transport = Some(cdus_common::TransportType::Lan);
                                                                    }
                                                                }
                                                            }
                                                            (record.node_id, record.label, transport)
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
                                        IpcMessage::StartBenchmark { node_id } => {
                                            let _ = tx_clone.send(IpcMessage::StartBenchmark { node_id });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "Benchmark initiated".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
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
                    let mut lw = last_written.lock();
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
        .unwrap_or_else(|_| panic!("Failed to execute {}", cmd));
    if !status.success() {
        error!("Command {} {:?} failed", cmd, args);
    }
}
