use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use flume::Sender;
use interprocess::local_socket::LocalSocketListener;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::sync::Arc; use parking_lot::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info};

use cdus_agent::{broadcast_event, daemon_loop, EVENT_BUS};
use cdus_common::{IpcMessage, SyncMessage, TransportType};
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

    #[arg(short, long, default_value = "5200", env = "CDUS_PORT")]
    port: u16,

    #[arg(short, long, default_value = "/tmp/cdus-agent.sock", env = "CDUS_AGENT_SOCKET")]
    socket: String,

    #[arg(long, env = "CDUS_DATA_DIR")]
    data_dir: Option<String>,

    #[arg(long, env = "CDUS_DOWNLOAD_DIR")]
    download_dir: Option<String>,

    #[arg(long, default_value = "http://localhost:8080", env = "CDUS_RELAY_URL")]
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

    // Spawns a background telemetry loop
    let telemetry_store = Arc::clone(&store);
    let telemetry_relay_url = cli.relay_url.clone();
    let telemetry_node_id = node_id.clone();
    thread::spawn(move || {
        loop {
            // Check opt-in
            let opt_in = telemetry_store.get_state("telemetry_opt_in")
                .unwrap_or(None)
                .map(|val| val == "true")
                .unwrap_or(false);

            if opt_in {
                // Gather anonymous metrics
                let paired_count = telemetry_store.get_paired_devices().map(|d| d.len()).unwrap_or(0);
                let recent_events_count = telemetry_store.get_recent_events(100).map(|e| e.len()).unwrap_or(0);
                let transfer_count = telemetry_store.get_transfer_history(100).map(|h| h.len()).unwrap_or(0);

                let payload = serde_json::json!({
                    "device_type": std::env::consts::OS,
                    "paired_devices": paired_count,
                    "recent_events": recent_events_count,
                    "transfers": transfer_count,
                    "version": env!("CARGO_PKG_VERSION"),
                });

                let url = format!("{}/v1/telemetry", telemetry_relay_url);
                debug!("Uploading usage telemetry to {}...", url);

                let agent = ureq::AgentBuilder::new()
                    .timeout(std::time::Duration::from_secs(5))
                    .build();

                let payload_str = payload.to_string();
                let body = serde_json::json!({
                    "device_uuid": telemetry_node_id,
                    "payload": payload_str,
                });

                if let Err(e) = agent.post(&url).send_json(body) {
                    error!("Error uploading telemetry: {}", e);
                }
            }

            // Upload telemetry once every hour
            thread::sleep(std::time::Duration::from_secs(3600));
        }
    });

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
        Libp2pManager::new_with_download_dir(
            private_key.clone(),
            tx.clone(),
            Arc::clone(&store),
            Arc::clone(&transfer_manager),
            cli.download_dir.map(std::path::PathBuf::from),
        )
        .expect("Failed to initialize Libp2pManager"),
    );
    let libp2p_sync_tx = libp2p_manager.get_sync_tx();

    // Initialize mDNS Manager
    let mdns = MdnsManager::new();
    let mdns = Arc::new(mdns);

    // Register and start mDNS discovery at startup
    mdns.register_device(&node_id, &label, cli.port);
    mdns.start_discovery(tx.clone());

    let sync_manager = Arc::new(SyncManager::new());
    sync_manager.add_peer(
        "libp2p_broadcast".to_string(),
        libp2p_sync_tx,
        TransportType::P2p,
    );
    let sync_manager_daemon = Arc::clone(&sync_manager);

    libp2p_manager.start(Arc::clone(&sync_manager));

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
    // let pm_reconnect = Arc::clone(&pm);
    // thread::spawn(move || {
    //     pm_reconnect.start_auto_reconnect_loop();
    // });

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
    let store_watcher = Arc::clone(&store);
    thread::spawn(move || {
        clipboard_watcher(clipboard_tx, last_written_watcher, store_watcher);
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
                let transfer_manager_clone = Arc::clone(&transfer_manager);
                let libp2p_manager_clone = Arc::clone(&libp2p_manager);

                thread::spawn(move || {
                    let mut buffer = [0u8; 4096];
                    loop {
                        match stream.read(&mut buffer) {
                            Ok(0) => break,
                            Ok(n) => {
                                let raw_json = String::from_utf8_lossy(&buffer[..n]);
                                let is_noisy = raw_json.contains("Ping") || raw_json.contains("GetPairingStatus") || raw_json.contains("GetPairedDevices") || (raw_json.contains("GetState") && raw_json.contains("sync_enabled"));
                                if is_noisy {
                                    debug!("IPC: Received raw: {}", raw_json);
                                } else {
                                    info!("IPC: Received raw: {}", raw_json);
                                }
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
                                        IpcMessage::GetQrPairingPayload => {
                                            match pm_clone.generate_qr_payload() {
                                                Ok(payload) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::QrPairingPayloadResponse {
                                                            payload,
                                                        },
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!(
                                                            "Error generating QR: {}",
                                                            e
                                                        )),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::SubmitFeedback { text, attach_logs } => {
                                            let node_id = relay_ipc.node_id().to_string();
                                            let relay_url = relay_ipc.relay_url().to_string();
                                            
                                            let logs_str = if attach_logs {
                                                match store_clone.get_audit_logs(100) {
                                                    Ok(logs) => {
                                                        logs.into_iter()
                                                            .map(|l| format!("[{}] {}: {}", l.timestamp, l.event_type, l.content))
                                                            .collect::<Vec<_>>()
                                                            .join("\n")
                                                    }
                                                    Err(_) => "".to_string(),
                                                }
                                            } else {
                                                "".to_string()
                                            };

                                            let store_cb = Arc::clone(&store_clone);
                                            thread::spawn(move || {
                                                let payload = serde_json::json!({
                                                    "device_uuid": node_id,
                                                    "content": text,
                                                    "logs": logs_str,
                                                });
                                                
                                                let url = format!("{}/v1/feedback", relay_url);
                                                info!("Uploading user feedback to {}...", url);
                                                
                                                let agent = ureq::AgentBuilder::new()
                                                    .timeout(std::time::Duration::from_secs(5))
                                                    .build();
                                                    
                                                match agent.post(&url).send_json(payload) {
                                                    Ok(resp) if resp.status() == 200 => {
                                                        info!("Feedback uploaded successfully.");
                                                        let _ = store_cb.append_audit_log("system", "User feedback uploaded successfully to relay");
                                                    }
                                                    Ok(resp) => {
                                                        error!("Failed to upload feedback: status {}", resp.status());
                                                        let _ = store_cb.append_audit_log("system", &format!("Failed to upload feedback: status {}", resp.status()));
                                                    }
                                                    Err(e) => {
                                                        error!("Error uploading feedback: {}", e);
                                                        let _ = store_cb.append_audit_log("system", &format!("Error uploading feedback: {}", e));
                                                    }
                                                }
                                            });

                                            let resp_bytes = serde_json::to_vec(
                                                &IpcMessage::Log("Feedback queued for submission".to_string())
                                            ).unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::SetTelemetryOptIn { opt_in } => {
                                            let val = if opt_in { "true" } else { "false" };
                                            match store_clone.set_state("telemetry_opt_in", val) {
                                                Ok(_) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!("Telemetry opt-in set to {}", opt_in))
                                                    ).unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!("Error setting telemetry opt-in: {}", e))
                                                    ).unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::GetTelemetryOptIn => {
                                            let opt_in = store_clone.get_state("telemetry_opt_in")
                                                .unwrap_or(None)
                                                .map(|val| val == "true")
                                                .unwrap_or(false);
                                            let resp_bytes = serde_json::to_vec(
                                                &IpcMessage::TelemetryOptInResponse(opt_in)
                                            ).unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::Search { query } => {
                                            match store_clone.search(&query) {
                                                Ok(results) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::SearchResponse(results)
                                                    ).unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!("Error performing search: {}", e))
                                                    ).unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::GetAuditLogs { limit } => {
                                            match store_clone.get_audit_logs(limit) {
                                                Ok(logs) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::AuditLogsResponse(logs)
                                                    ).unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!("Error fetching audit logs: {}", e))
                                                    ).unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::GetActiveNotifications => {
                                            let list: Vec<cdus_common::NotificationPayload> = cdus_agent::ACTIVE_NOTIFICATIONS.lock().values().cloned().collect();
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::ActiveNotificationsResponse(list)).unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::DismissNotification { key } => {
                                            let _ = tx_clone.send(IpcMessage::DismissNotification { key: key.clone() });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::NotificationDismissed { key }).unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::ClearAuditLogs => {
                                            match store_clone.clear_audit_logs() {
                                                Ok(_) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log("Audit logs cleared".to_string())
                                                    ).unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!("Error clearing audit logs: {}", e))
                                                    ).unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::AppendAuditLog { event_type, content } => {
                                            match store_clone.append_audit_log(&event_type, &content) {
                                                Ok(_) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log("Audit log appended".to_string())
                                                    ).unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!("Error appending audit log: {}", e))
                                                    ).unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::ClearFinishedTransfers => {
                                            info!("IPC: ClearFinishedTransfers requested");
                                            match store_clone.clear_finished_transfers() {
                                                Ok(_) => {
                                                    let resp_bytes =
                                                        serde_json::to_vec(&IpcMessage::Log(
                                                            "Finished transfers cleared"
                                                                .to_string(),
                                                        ))
                                                        .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    error!("IPC: Failed to clear finished transfers: {}", e);
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!(
                                                            "Error clearing transfers: {}",
                                                            e
                                                        )),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::DeleteFileTransfer { transfer_id } => {
                                            info!("IPC: DeleteFileTransfer requested for ID: {}", transfer_id);
                                            match store_clone.delete_transfer(&transfer_id) {
                                                Ok(_) => {
                                                    let resp_bytes =
                                                        serde_json::to_vec(&IpcMessage::Log(
                                                            "Transfer deleted"
                                                                .to_string(),
                                                        ))
                                                        .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    error!("IPC: Failed to delete transfer {}: {}", transfer_id, e);
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!(
                                                            "Error deleting transfer: {}",
                                                            e
                                                        )),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::DeleteFilePermanently { transfer_id } => {
                                            info!("IPC: DeleteFilePermanently requested for ID: {}", transfer_id);
                                            let mut file_deleted = false;
                                            let mut file_path_str = String::new();
                                            if let Ok(Some(transfer)) = store_clone.get_transfer(&transfer_id) {
                                                file_path_str = transfer.file_path.clone();
                                                let path = std::path::Path::new(&transfer.file_path);
                                                if path.exists() {
                                                    if let Err(e) = std::fs::remove_file(path) {
                                                        error!("Failed to delete physical file {:?}: {:?}", path, e);
                                                    } else {
                                                        file_deleted = true;
                                                        info!("Successfully deleted physical file {:?}", path);
                                                    }
                                                }
                                            }

                                            match store_clone.delete_transfer(&transfer_id) {
                                                Ok(_) => {
                                                    let msg = if file_deleted {
                                                        format!("Transfer and physical file deleted: {}", file_path_str)
                                                    } else {
                                                        format!("Transfer deleted (physical file not found or failed to delete): {}", file_path_str)
                                                    };
                                                    let resp_bytes =
                                                        serde_json::to_vec(&IpcMessage::Log(msg))
                                                        .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    error!("IPC: Failed to delete transfer {} from DB: {}", transfer_id, e);
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!(
                                                            "Error deleting transfer from DB: {}",
                                                            e
                                                        )),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::PairWithQr { payload } => {
                                            match pm_clone.parse_qr_payload(&payload) {
                                                Ok((node_id, secret, label, port, ips)) => {
                                                    if pm_clone.is_device_paired(&node_id) {
                                                        let _ = stream.write_all(&serde_json::to_vec(&IpcMessage::Log("Already paired".to_string())).unwrap());
                                                        continue;
                                                    }

                                                    info!("IPC: Scanned QR for {} ({}). IPs: {:?}, Port: {}. Setting OOB secret and starting direct pairing.", label, node_id, ips, port);
                                                    pm_clone.set_target_oob_secret(node_id.clone(), secret);

                                                    // Pre-populate peer_map with data from QR
                                                    {
                                                        let mut map = peer_map_clone.lock();
                                                        map.insert(
                                                            node_id.clone(),
                                                            (
                                                                label.clone(),
                                                                "Unknown".to_string(),
                                                                ips.clone(),
                                                                port,
                                                                std::time::Instant::now(),
                                                            ),
                                                        );
                                                    }

                                                    let pm_init = Arc::clone(&pm_clone);
                                                    let node_id_inner = node_id.clone();
                                                    let ips_inner = ips.clone();
                                                    thread::spawn(move || {
                                                        let mut success = false;
                                                        for ip in ips_inner {
                                                            if let Ok(ip_addr) = ip.parse() {
                                                                let addr = SocketAddr::new(ip_addr, port);
                                                                if pm_init.initiate_pairing(addr, Some(node_id_inner.clone())) {
                                                                    success = true;
                                                                    break;
                                                                }
                                                            }
                                                        }
                                                        
                                                        if !success {
                                                            info!("Direct connection failed for QR device {}, falling back to relay", node_id_inner);
                                                            pm_init.initiate_remote_pairing(node_id_inner);
                                                        }
                                                    });

                                                    let resp_bytes = serde_json::to_vec(&IpcMessage::Log("QR pairing process started".to_string())).unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(&IpcMessage::Log(format!("Error processing QR: {}", e))).unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
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
                                                            if pm_init.initiate_pairing(addr, Some(node_id_clone.clone())) {
                                                                info!("Manual pairing initiated with {} at {}", node_id_clone, addr);
                                                                success = true;
                                                                break;
                                                            }
                                                        }
                                                    }
                                                    
                                                    if !success {
                                                        info!("mDNS failed for {}, falling back to relay", node_id_clone);
                                                        if pm_init.is_device_paired(&node_id_clone) {
                                                            pm_init.reconnect_known_device(node_id_clone);
                                                        } else {
                                                            pm_init.initiate_remote_pairing(node_id_clone);
                                                        }
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
                                                    if pm_init.is_device_paired(&node_id_clone) {
                                                        pm_init.reconnect_known_device(node_id_clone);
                                                    } else {
                                                        pm_init.initiate_remote_pairing(node_id_clone);
                                                    }
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
                                                    pm_init.initiate_pairing(addr, None);
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
                                                if pm_init.is_device_paired(&uuid) {
                                                    pm_init.reconnect_known_device(uuid);
                                                } else {
                                                    pm_init.initiate_remote_pairing(uuid);
                                                }
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
                                                    let active = !state.is_reconnect;
                                                    IpcMessage::PairingStatusResponse {
                                                        pin: Some(state.pin.clone()),
                                                        active,
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
                                        IpcMessage::DisconnectDevice { node_id } => {
                                            if !sync_manager_ipc.send_to_peer(&node_id, SyncMessage::Disconnect) {
                                                sync_manager_ipc.remove_peer(&node_id);
                                            }
                                            transfer_manager_clone.cancel_all_transfers_for_peer(&node_id);
                                            if let Ok(peer_id) = node_id.parse::<libp2p::PeerId>() {
                                                libp2p_manager_clone.disconnect_peer(peer_id);
                                            }
                                            broadcast_event(IpcMessage::PeerDisconnected { node_id: node_id.clone() });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log("Disconnected".to_string())).unwrap();
                                            let _ = stream.write_all(&resp_bytes);
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
                                        IpcMessage::DeleteHistoryItem { id } => {
                                            match store_clone.delete_event(id) {
                                                Ok(_) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log("Clipboard item deleted".to_string()),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!(
                                                            "Error deleting item: {}",
                                                            e
                                                        )),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::ToggleLocalOnly { id, local_only } => {
                                            match store_clone.set_local_only(id, local_only) {
                                                Ok(_) => {
                                                    if !local_only {
                                                        if let Ok(Some(event)) = store_clone.get_event_by_id(id) {
                                                            let timestamp = std::time::SystemTime::now()
                                                                .duration_since(std::time::UNIX_EPOCH)
                                                                .unwrap_or_default()
                                                                .as_millis() as u64;
                                                            let _ = store_clone.set_state("last_sync_timestamp", &timestamp.to_string());
                                                            let _ = store_clone.set_state("last_clipboard_content", &event.content);
                                                            sync_manager_ipc.broadcast(SyncMessage::ClipboardUpdate {
                                                                content: event.content,
                                                                timestamp,
                                                            });
                                                        }
                                                    }
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log("Clipboard local_only toggled".to_string()),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!(
                                                            "Error toggling local_only: {}",
                                                            e
                                                        )),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
                                        }
                                        IpcMessage::ClearHistory => {
                                            match store_clone.clear_events() {
                                                Ok(_) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log("Clipboard history cleared".to_string()),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!(
                                                            "Error clearing history: {}",
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
                                                    if key == "device_name" {
                                                        mdns_clone.register_device(&node_id_clone, &value, port);
                                                    }
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
                                        IpcMessage::SimulateCrash { transfer_id } => {
                                            let _ = tx_clone
                                                .send(IpcMessage::SimulateCrash { transfer_id });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "Crash simulation requested".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::SetCrashTrigger {
                                            transfer_id,
                                            offset,
                                        } => {
                                            let _ = tx_clone.send(IpcMessage::SetCrashTrigger {
                                                transfer_id,
                                                offset,
                                            });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "Crash trigger set".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::ResumeFileTransfer { transfer_id } => {
                                            let _ = tx_clone
                                                .send(IpcMessage::ResumeFileTransfer { transfer_id });
                                            let resp_bytes = serde_json::to_vec(&IpcMessage::Log(
                                                "File transfer resume requested".to_string(),
                                            ))
                                            .unwrap();
                                            let _ = stream.write_all(&resp_bytes);
                                        }
                                        IpcMessage::GetFileTransferHistory { limit } => {
                                            match store_clone.get_transfer_history(limit) {
                                                Ok(history) => {
                                                    let mapped_history = history
                                                        .into_iter()
                                                        .map(|t| cdus_common::FileTransferRecord {
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
                                                        .collect();
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::FileTransferHistoryResponse(
                                                            mapped_history,
                                                        ),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                                Err(e) => {
                                                    let resp_bytes = serde_json::to_vec(
                                                        &IpcMessage::Log(format!(
                                                            "Error fetching transfer history: {}",
                                                            e
                                                        )),
                                                    )
                                                    .unwrap();
                                                    let _ = stream.write_all(&resp_bytes);
                                                }
                                            }
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

fn clipboard_watcher(
    tx: Sender<IpcMessage>,
    last_written: Arc<Mutex<Option<String>>>,
    store: Arc<Store>,
) {
    #[cfg(not(target_os = "android"))]
    {
        use arboard::Clipboard;

        let mut clipboard_opt = Clipboard::new().ok();
        if clipboard_opt.is_none() {
            tracing::warn!("Failed to initialize clipboard on startup, will retry in background");
        }

        // Initialize last_content_hash from state DB content
        let db_content = store.get_state("last_clipboard_content").ok().flatten();
        let mut last_content_hash = String::new();
        if let Some(ref val) = db_content {
            last_content_hash = blake3::hash(val.as_bytes()).to_hex().to_string();
        }

        loop {
            if let Some(ref mut clipboard) = clipboard_opt {
                // 1. Try reading image first
                if let Ok(image) = clipboard.get_image() {
                    let img_hash = blake3::hash(&image.bytes).to_hex().to_string();
                    if img_hash != last_content_hash {
                        let mut lw = last_written.lock();
                        if let Some(ref val) = *lw {
                            let expected_hash = format!("CDUS_IMAGE_HASH:{}", img_hash);
                            if val == &expected_hash {
                                info!("Ignoring image clipboard change (self/remote-triggered)");
                                last_content_hash = img_hash;
                                *lw = None;
                                continue;
                            }
                        }

                        info!("Image clipboard change detected");
                        if let Ok(png_bytes) = encode_image_to_png(&image) {
                            use base64::Engine;
                            let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
                            let data_url = format!("data:image/png;base64,{}", b64);
                            let payload = serde_json::json!({
                                "type": "image",
                                "data": data_url
                            }).to_string();

                            last_content_hash = img_hash;
                            let _ = tx.send(IpcMessage::ClipboardChanged {
                                content: payload,
                                timestamp: now_ms(),
                            });
                        }
                    }
                }
                // 2. Try reading text if no image
                else if let Ok(current_content) = clipboard.get_text() {
                    if !current_content.trim().is_empty() {
                        let text_hash = blake3::hash(current_content.as_bytes()).to_hex().to_string();
                        if text_hash != last_content_hash {
                            let mut lw = last_written.lock();
                            if let Some(ref val) = *lw {
                                if val == &current_content {
                                    info!("Ignoring text clipboard change (self/remote-triggered)");
                                    last_content_hash = text_hash;
                                    *lw = None;
                                    continue;
                                }
                            }

                            last_content_hash = text_hash;

                            // Check if it is a URL
                            let is_url = (current_content.trim().starts_with("http://") || current_content.trim().starts_with("https://")) 
                                && url::Url::parse(current_content.trim()).is_ok();
                            
                            if is_url {
                                info!("URL clipboard change detected: {}", current_content);
                                let tx_clone = tx.clone();
                                let url_str = current_content.trim().to_string();
                                thread::spawn(move || {
                                    let resolved = cdus_agent::utils::resolve_url_metadata(&url_str);
                                    let content = if let Some((title, favicon)) = resolved {
                                        serde_json::json!({
                                            "type": "url",
                                            "url": url_str,
                                            "title": title,
                                            "favicon": favicon
                                        }).to_string()
                                    } else {
                                        url_str
                                    };
                                    let _ = tx_clone.send(IpcMessage::ClipboardChanged {
                                        content,
                                        timestamp: now_ms(),
                                    });
                                });
                            } else {
                                info!("Text clipboard change detected");
                                let _ = tx.send(IpcMessage::ClipboardChanged {
                                    content: current_content,
                                    timestamp: now_ms(),
                                });
                            }
                        }
                    }
                }
            } else {
                // Retry initializing clipboard
                match Clipboard::new() {
                    Ok(c) => {
                        info!("Clipboard watcher successfully initialized on retry");
                        clipboard_opt = Some(c);
                    }
                    Err(_) => {}
                }
            }
            thread::sleep(Duration::from_secs(2));
        }
    }
    #[cfg(target_os = "android")]
    {
        info!("Clipboard watcher not implemented for Android yet");
    }
}

#[cfg(not(target_os = "android"))]
fn encode_image_to_png(image: &arboard::ImageData) -> Result<Vec<u8>, anyhow::Error> {
    use image::{ImageBuffer, Rgba};
    use std::io::Cursor;
    
    let width = image.width as u32;
    let height = image.height as u32;
    let raw_pixels = image.bytes.to_vec();
    
    let img_buffer = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width, height, raw_pixels)
        .ok_or_else(|| anyhow::anyhow!("Failed to create ImageBuffer from raw pixels"))?;
        
    let mut png_bytes = Vec::new();
    img_buffer.write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)?;
    Ok(png_bytes)
}


#[cfg(target_os = "macos")]
fn install_service() {
    info!("Installing CDUS Agent as macOS launchd agent...");

    let exe_path = std::env::current_exe().expect("Failed to get current executable path");
    let home = std::env::var("HOME").expect("Failed to get HOME environment variable");
    let plist_dir = std::path::PathBuf::from(home).join("Library/LaunchAgents");
    let plist_path = plist_dir.join("com.cdus.agent.plist");

    let service_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.cdus.agent</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>KeepAlive</key>
    <true/>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/cdus-agent.out.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/cdus-agent.err.log</string>
</dict>
</plist>
"#,
        exe_path.display()
    );

    std::fs::create_dir_all(&plist_dir).expect("Failed to create LaunchAgents directory");
    std::fs::write(&plist_path, service_content).expect("Failed to write launchd plist file");
    info!("Launchd plist written to {}", plist_path.display());

    run_command("launchctl", &["load", "-w", plist_path.to_str().unwrap()]);
    run_command("launchctl", &["start", "com.cdus.agent"]);

    info!("CDUS Agent service installed and started.");
}

#[cfg(target_os = "macos")]
fn uninstall_service() {
    info!("Uninstalling CDUS Agent macOS launchd agent...");

    let home = std::env::var("HOME").expect("Failed to get HOME environment variable");
    let plist_path = std::path::PathBuf::from(home)
        .join("Library/LaunchAgents/com.cdus.agent.plist");

    if plist_path.exists() {
        run_command("launchctl", &["unload", "-w", plist_path.to_str().unwrap()]);
        std::fs::remove_file(&plist_path).expect("Failed to remove launchd plist file");
        info!("Launchd plist file removed.");
    }

    info!("CDUS Agent service uninstalled.");
}

#[cfg(target_os = "windows")]
fn install_service() {
    info!("Installing CDUS Agent as Windows startup program...");

    let exe_path = std::env::current_exe().expect("Failed to get current executable path");
    let exe_str = exe_path.to_str().expect("Failed to convert exe path to string");

    // Add to HKCU Registry Run key using reg.exe command line
    run_command(
        "reg",
        &[
            "add",
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
            "/v",
            "CDUSAgent",
            "/t",
            "REG_SZ",
            "/d",
            exe_str,
            "/f",
        ],
    );

    // Spawn the daemon process now in the background
    let _ = std::process::Command::new(&exe_path)
        .spawn()
        .expect("Failed to start CDUS Agent process");

    info!("CDUS Agent registry run key installed and daemon started.");
}

#[cfg(target_os = "windows")]
fn uninstall_service() {
    info!("Uninstalling CDUS Agent Windows startup program...");

    run_command(
        "reg",
        &[
            "delete",
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
            "/v",
            "CDUSAgent",
            "/f",
        ],
    );

    // Best-effort termination of running agent processes
    let _ = std::process::Command::new("taskkill")
        .args(&["/IM", "cdus-agent.exe", "/F"])
        .status();

    info!("CDUS Agent registry run key deleted and processes terminated.");
}

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn install_service() {
    info!("Service installation not supported on this platform.");
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn uninstall_service() {
    info!("Service uninstallation not supported on this platform.");
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
