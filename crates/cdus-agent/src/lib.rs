pub mod file_transfer;
pub mod libp2p_manager;
pub mod mdns;
pub mod pairing;
pub mod relay;
pub mod store;
pub mod turn_manager;
pub mod utils;

#[cfg(test)]
pub mod integration_tests;

use cdus_common::{IpcMessage, SyncMessage};
use flume::{Receiver, Sender};
use parking_lot::Mutex;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tracing::{debug, error, info};

use crate::libp2p_manager::Libp2pManager;
use crate::pairing::{ActivePairingState, PairingManager, SyncManager};
use crate::store::Store;

pub static EVENT_BUS: Mutex<Vec<Sender<IpcMessage>>> = Mutex::new(Vec::new());

use once_cell::sync::Lazy;
pub static ACTIVE_NOTIFICATIONS: Lazy<Mutex<std::collections::HashMap<String, cdus_common::NotificationPayload>>> =
    Lazy::new(|| Mutex::new(std::collections::HashMap::new()));
pub static LAST_ALERT_TIMESTAMPS: Lazy<Mutex<std::collections::HashMap<String, std::time::Instant>>> =
    Lazy::new(|| Mutex::new(std::collections::HashMap::new()));

pub fn broadcast_event(msg: IpcMessage) {
    let mut bus = EVENT_BUS.lock();
    bus.retain(|tx| tx.send(msg.clone()).is_ok());
}

pub fn daemon_loop(
    tx: Sender<IpcMessage>,
    rx: Receiver<IpcMessage>,
    iterations: Option<usize>,
    store: Arc<Store>,
    #[allow(unused_variables)] last_written: Arc<Mutex<Option<String>>>,
    discovered_devices: Arc<Mutex<Vec<(String, String, String, Vec<String>, u16)>>>,
    _active_pairing: Arc<Mutex<Option<ActivePairingState>>>,
    sync_manager: Arc<SyncManager>,
    pm: Arc<PairingManager>,
    last_processed_timestamp: Arc<Mutex<u64>>,
    peer_map: Arc<
        Mutex<
            std::collections::HashMap<
                String,
                (String, String, Vec<String>, u16, std::time::Instant),
            >,
        >,
    >,
    libp2p_request_tx: Option<Sender<(libp2p::PeerId, SyncMessage)>>,
    libp2p_manager: Arc<Libp2pManager>,
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

        // Process all available messages in the queue before sleeping
        while let Ok(msg) = rx.try_recv() {
            debug!("Daemon processing: {:?}", msg);
            match msg {
                IpcMessage::Ping => {
                    let _ = tx.send(IpcMessage::Pong);
                }
                IpcMessage::ToggleLocalOnly { id, local_only } => {
                    info!("Toggling local_only to {} for event {}", local_only, id);
                    if let Err(e) = store.set_local_only(id, local_only) {
                        error!("Failed to toggle local_only: {}", e);
                    } else if !local_only {
                        if let Ok(Some(event)) = store.get_event_by_id(id) {
                            let timestamp = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64;
                            let _ = store.set_state("last_sync_timestamp", &timestamp.to_string());
                            let _ = store.set_state("last_clipboard_content", &event.content);
                            sync_manager.broadcast(SyncMessage::ClipboardUpdate {
                                content: event.content,
                                timestamp,
                            });
                        }
                    }
                    if let Ok(history) = store.get_recent_events(50) {
                        broadcast_event(IpcMessage::HistoryResponse(history));
                    }
                }
                IpcMessage::ClipboardChanged { content, timestamp } => {
                    if content.trim().is_empty() {
                        continue;
                    }
                    let mut last_ts = last_processed_timestamp.lock();
                    if timestamp > *last_ts {
                        *last_ts = timestamp;
                        let _ = store.set_state("last_sync_timestamp", &timestamp.to_string());
                        let _ = store.set_state("last_clipboard_content", &content);

                        info!("Syncing clipboard content: {}", content);
                        if let Err(e) = store.append_event(content.as_bytes(), "Local") {
                            error!("Failed to store clipboard event: {}", e);
                        }

                        let display_len = std::cmp::min(20, content.len());
                        let _ = store.append_audit_log("sync", &format!("Outgoing clipboard sync: {}...", &content[..display_len]));

                        let is_local = store.is_content_local_only(&content).unwrap_or(false);
                        if !is_local {
                            // Broadcast to peers
                            sync_manager.broadcast(SyncMessage::ClipboardUpdate {
                                content: content.clone(),
                                timestamp,
                            });
                        } else {
                            info!("Skipping remote sync: content is marked local-only");
                        }

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
                    if content.trim().is_empty() {
                        continue;
                    }

                    if store.is_current_local_only().unwrap_or(false) {
                        info!("Ignoring remote clipboard update: current local clipboard item is marked local-only");
                        continue;
                    }
                    if store.is_content_local_only(&content).unwrap_or(false) {
                        info!("Ignoring remote clipboard update: incoming content is marked local-only");
                        continue;
                    }

                    let mut last_ts = last_processed_timestamp.lock();
                    if timestamp > *last_ts {
                        *last_ts = timestamp;
                        let _ = store.set_state("last_sync_timestamp", &timestamp.to_string());
                        let _ = store.set_state("last_clipboard_content", &content);

                        info!("Writing to clipboard from {}: {}", source, content);

                        // Append to local history as well
                        if let Err(e) = store.append_event(content.as_bytes(), &source) {
                            error!("Failed to store received clipboard event: {}", e);
                        }

                        let display_len = std::cmp::min(20, content.len());
                        let _ = store.append_audit_log("sync", &format!("Incoming clipboard sync from {}: {}...", source, &content[..display_len]));

                        #[cfg(not(target_os = "android"))]
                        {
                            if let Some(ref mut cb) = clipboard {
                                let mut text_to_set = None;
                                let mut image_to_set = None;
                                let mut raw_content_hash = String::new();

                                if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&content) {
                                    if let Some(typ) = json_val.get("type").and_then(|v| v.as_str()) {
                                        match typ {
                                            "image" => {
                                                if let Some(data_url) = json_val.get("data").and_then(|v| v.as_str()) {
                                                    if let Some(b64) = data_url.strip_prefix("data:image/png;base64,") {
                                                        use base64::Engine;
                                                        if let Ok(png_bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                                                            if let Ok(img_data) = decode_png_to_image(&png_bytes) {
                                                                raw_content_hash = format!("CDUS_IMAGE_HASH:{}", blake3::hash(&img_data.bytes).to_hex().to_string());
                                                                image_to_set = Some(img_data);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            "url" => {
                                                if let Some(url_val) = json_val.get("url").and_then(|v| v.as_str()) {
                                                    raw_content_hash = url_val.to_string();
                                                    text_to_set = Some(url_val.to_string());
                                                }
                                            }
                                            _ => {
                                                raw_content_hash = content.clone();
                                                text_to_set = Some(content.clone());
                                            }
                                        }
                                    } else {
                                        raw_content_hash = content.clone();
                                        text_to_set = Some(content.clone());
                                    }
                                } else {
                                    raw_content_hash = content.clone();
                                    text_to_set = Some(content.clone());
                                }

                                {
                                    let mut lw = last_written.lock();
                                    *lw = Some(raw_content_hash);
                                }

                                let write_res = if let Some(img_data) = image_to_set {
                                    cb.set_image(img_data).map_err(|e| e.to_string())
                                } else if let Some(txt) = text_to_set {
                                    cb.set_text(txt).map_err(|e| e.to_string())
                                } else {
                                    Ok(())
                                };

                                if let Err(e) = write_res {
                                    error!("Failed to write to clipboard: {}", e);
                                    let mut lw = last_written.lock();
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
                    ips,
                    port,
                } => {
                    // Update global peer map for connection fallback
                    {
                        let mut map = peer_map.lock();
                        map.insert(node_id.clone(), (label.clone(), os.clone(), ips.clone(), port, std::time::Instant::now()));
                    }

                    let already_paired = store.is_device_paired(&node_id).unwrap_or(false);

                    if already_paired {
                        // Persist last known network info for reconnection
                        if let Err(e) = store.update_paired_device_network_info(&node_id, &ips, port) {
                            error!("Failed to update network info for paired device {}: {}", node_id, e);
                        }
                    } else {
                        let mut list = discovered_devices.lock();
                        if !list.iter().any(|(id, _, _, _, _)| id == &node_id) {
                            list.push((
                                node_id.clone(),
                                label.clone(),
                                os.clone(),
                                ips.clone(),
                                port,
                            ));
                        }
                    }
                    
                    if !already_paired {
                        broadcast_event(IpcMessage::DeviceDiscovered {
                            node_id: node_id.clone(),
                            label: label.clone(),
                            os: os.clone(),
                            ips: ips.clone(),
                            port,
                        });
                    }
                }
                IpcMessage::PairingResult { success, node_id, label, error } => {
                    if success {
                        info!("Pairing successful with {} ({}). Removing from discovery list.", label, node_id);
                        let mut list = discovered_devices.lock();
                        list.retain(|(id, _, _, _, _)| id != &node_id);
                        let _ = store.append_audit_log("pairing", &format!("Successfully paired with device '{}' (#{})", label, &node_id[..std::cmp::min(8, node_id.len())]));
                    } else {
                        let _ = store.append_audit_log("pairing", &format!("Pairing failed with device '{}' (#{}): {}", label, &node_id[..std::cmp::min(8, node_id.len())], error.as_deref().unwrap_or("unknown")));
                    }
                    broadcast_event(IpcMessage::PairingResult { success, node_id, label, error });
                }
                IpcMessage::RelayStatus { connected, error } => {
                    broadcast_event(IpcMessage::RelayStatus { connected, error });
                }
                IpcMessage::AlreadyPaired { node_id, label } => {
                    broadcast_event(IpcMessage::AlreadyPaired { node_id, label });
                }
                IpcMessage::StalePairing { node_id, label } => {
                    broadcast_event(IpcMessage::StalePairing { node_id, label });
                }
                IpcMessage::DeviceLost { node_id } => {
                    let mut list = discovered_devices.lock();
                    list.retain(|(id, _, _, _, _)| !id.starts_with(&node_id));
                    broadcast_event(IpcMessage::DeviceLost { node_id });
                }
                IpcMessage::PeerDisconnected { node_id } => {
                    let tm = libp2p_manager.get_transfer_manager();
                    tm.cancel_all_transfers_for_peer(&node_id);
                    if let Ok(peer_id) = node_id.parse::<libp2p::PeerId>() {
                        libp2p_manager.disconnect_peer(peer_id);
                    }
                    broadcast_event(IpcMessage::PeerDisconnected { node_id });
                }
                IpcMessage::PeerConnected { node_id } => {
                    {
                        let mut list = discovered_devices.lock();
                        list.retain(|(id, _, _, _, _)| id != &node_id);
                    }
                    broadcast_event(IpcMessage::PeerConnected { node_id });
                }
                IpcMessage::RelayMessage {
                    source_uuid,
                    payload,
                } => {
                    let pm_clone = Arc::clone(&pm);
                    thread::spawn(move || {
                        pm_clone.handle_relay_message(source_uuid, payload);
                    });
                }
                IpcMessage::SendFile { node_id, path } => {
                    if !sync_manager.is_connected(&node_id) {
                        error!("Cannot send file: Peer {} is disconnected", node_id);
                        let tm = libp2p_manager.get_transfer_manager();
                        let _ = tm.progress_tx.send(cdus_common::ProgressEvent::Failed {
                            transfer_id: "".to_string(),
                            reason: "Peer is disconnected".to_string(),
                        });
                        continue;
                    }
                    let path_buf = std::path::PathBuf::from(path);
                    let store_clone = Arc::clone(&store);
                    let libp2p_manager_clone = Arc::clone(&libp2p_manager);
                    let transfer_manager = libp2p_manager_clone.get_transfer_manager();
                    
                    thread::spawn(move || {
                        let file_name = match path_buf.file_name() {
                            Some(n) => n.to_string_lossy().to_string(),
                            None => {
                                error!("Invalid file path: {:?}", path_buf);
                                return;
                            }
                        };
                        let total_bytes = match path_buf.metadata() {
                            Ok(m) => m.len(),
                            Err(e) => {
                                error!("Failed to get file metadata: {}", e);
                                return;
                            }
                        };
                        
                        info!("Hashing file: {:?}", path_buf);
                        let file_hash = match crate::file_transfer::hash_file(&path_buf) {
                            Ok(h) => h,
                            Err(e) => {
                                error!("Failed to hash file: {}", e);
                                return;
                            }
                        };
                        
                        let transfer_id = uuid::Uuid::new_v4().to_string();
                        
                        if let Err(e) = store_clone.create_transfer(
                            &transfer_id,
                            "outgoing",
                            &node_id,
                            &path_buf.to_string_lossy(),
                            &file_name,
                            total_bytes,
                            262144, // 256KB
                            &file_hash,
                        ) {
                            error!("Failed to create transfer in DB: {}", e);
                            return;
                        }

                        if let Ok(peer_id) = node_id.parse::<libp2p::PeerId>() {
                             match libp2p_manager_clone.open_file_stream(peer_id) {
                               Ok(wrapped_stream) => {
                                    let session_key = crate::file_transfer::SessionKey([0u8; 32]);
                                    if let Err(e) = crate::file_transfer::handle_outgoing_transfer(
                                        Box::new(wrapped_stream),
                                        store_clone,
                                        transfer_id,
                                        session_key,
                                        transfer_manager,
                                    ) {
                                        error!("File transfer failed: {}", e);
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to open stream to {}: {}", peer_id, e);
                                    let _ = transfer_manager.progress_tx.send(cdus_common::ProgressEvent::Failed { 
                                        transfer_id: transfer_id.clone(), 
                                        reason: format!("Connection failed: {}", e) 
                                    });
                                }
                             }
                        } else {
                            error!("Invalid PeerId: {}", node_id);
                        }
                    });
                }
                IpcMessage::StartBenchmark { node_id } => {
                    let total_bytes = 1024 * 1024 * 1024; // 1GB
                    let store_clone = Arc::clone(&store);
                    let libp2p_manager_clone = Arc::clone(&libp2p_manager);
                    let transfer_manager = libp2p_manager_clone.get_transfer_manager();
                    
                    thread::spawn(move || {
                        let transfer_id = "ffffffff-ffff-ffff-ffff-ffffffffffff".to_string();
                        let _ = store_clone.delete_transfer(&transfer_id);

                        if let Err(e) = store_clone.create_transfer(
                            &transfer_id,
                            "outgoing",
                            &node_id,
                            "/dev/null/benchmark",
                            "benchmark.bin",
                            total_bytes,
                            1048576, // 1MB
                            "benchmark-hash",
                        ) {
                            error!("Failed to create benchmark transfer in DB: {}", e);
                            return;
                        }

                        if let Ok(peer_id) = node_id.parse::<libp2p::PeerId>() {
                             match libp2p_manager_clone.open_file_stream(peer_id) {
                               Ok(wrapped_stream) => {
                                    let session_key = crate::file_transfer::SessionKey([0u8; 32]);
                                    if let Err(e) = crate::file_transfer::handle_outgoing_transfer(
                                        Box::new(wrapped_stream),
                                        store_clone,
                                        transfer_id,
                                        session_key,
                                        transfer_manager,
                                    ) {
                                        error!("Benchmark transfer failed: {}", e);
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to open benchmark stream to {}: {}", peer_id, e);
                                    let _ = transfer_manager.progress_tx.send(cdus_common::ProgressEvent::Failed { 
                                        transfer_id: transfer_id.clone(), 
                                        reason: format!("Connection failed: {}", e) 
                                    });
                                }
                             }
                        } else {
                            error!("Invalid PeerId for benchmark: {}", node_id);
                        }
                    });
                }
                IpcMessage::AcceptFileTransfer { transfer_id } => {
                    libp2p_manager.get_transfer_manager().handle_decision(&transfer_id, true);
                }
                IpcMessage::RejectFileTransfer { transfer_id } => {
                    libp2p_manager.get_transfer_manager().handle_decision(&transfer_id, false);
                }
                IpcMessage::CancelFileTransfer { transfer_id } => {
                    libp2p_manager.get_transfer_manager().cancel_transfer(&transfer_id);
                }
                IpcMessage::SimulateCrash { transfer_id } => {
                    libp2p_manager.get_transfer_manager().simulate_crash(&transfer_id);
                }
                IpcMessage::SetCrashTrigger { transfer_id, offset } => {
                    libp2p_manager.get_transfer_manager().set_crash_trigger(transfer_id, offset);
                }
                IpcMessage::ResumeFileTransfer { transfer_id } => {
                    let store_clone = Arc::clone(&store);
                    let libp2p_manager_clone = Arc::clone(&libp2p_manager);
                    if let Ok(Some(record)) = store_clone.get_transfer(&transfer_id) {
                        if record.direction == "outgoing" {
                            thread::spawn(move || {
                                if let Ok(peer_id) = record.peer_node_id.parse::<libp2p::PeerId>() {
                                    match libp2p_manager_clone.open_file_stream(peer_id) {
                                        Ok(wrapped_stream) => {
                                            let session_key = crate::file_transfer::SessionKey([0u8; 32]);
                                            if let Err(e) = crate::file_transfer::handle_outgoing_transfer(
                                                Box::new(wrapped_stream),
                                                store_clone,
                                                transfer_id,
                                                session_key,
                                                libp2p_manager_clone.get_transfer_manager(),
                                            ) {
                                                error!("Resumed transfer failed: {}", e);
                                            }
                                        }
                                        Err(e) => error!("Failed to open stream for resume: {}", e),
                                    }
                                }
                            });
                        } else {
                            error!("Cannot resume incoming transfer from receiver side. Wait for sender to resume.");
                        }
                    } else {
                        error!("Failed to find transfer {} to resume", transfer_id);
                    }
                }
                IpcMessage::FileTransferProgress { transfer_id, progress } => {
                    broadcast_event(IpcMessage::FileTransferProgress { transfer_id, progress });
                }
                IpcMessage::TestLibp2pRequest { peer_id } => {
                    if let Some(tx) = &libp2p_request_tx {
                        if let Ok(pid) = peer_id.parse::<libp2p::PeerId>() {
                            info!("Manual test: sending libp2p request to {}", pid);
                            let _ = tx.send((
                                pid,
                                SyncMessage::ClipboardUpdate {
                                    content: "MANUAL_TEST_MSG".to_string(),
                                    timestamp: std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap()
                                        .as_millis() as u64,
                                },
                            ));
                        } else {
                            error!("Manual test: invalid PeerId {}", peer_id);
                        }
                    } else {
                        error!("Manual test: libp2p_request_tx not available");
                    }
                }
                IpcMessage::GetActiveNotifications => {
                    let _ = tx.send(IpcMessage::ActiveNotificationsResponse(
                        ACTIVE_NOTIFICATIONS.lock().values().cloned().collect(),
                    ));
                }
                IpcMessage::DismissNotification { key } => {
                    ACTIVE_NOTIFICATIONS.lock().remove(&key);
                    LAST_ALERT_TIMESTAMPS.lock().remove(&key);
                    info!("Dismissing notification: {}", key);
                    sync_manager.broadcast(SyncMessage::NotificationDismiss { key: key.clone() });
                    broadcast_event(IpcMessage::NotificationDismissed { key });
                }
                IpcMessage::NotificationMirrored(payload) => {
                    let should_alert = {
                        if payload.is_ongoing {
                            false
                        } else if payload.only_alert_once && ACTIVE_NOTIFICATIONS.lock().contains_key(&payload.key) {
                            false
                        } else {
                            let mut alert_times = LAST_ALERT_TIMESTAMPS.lock();
                            let now = std::time::Instant::now();
                            if let Some(&last_time) = alert_times.get(&payload.key) {
                                if now.duration_since(last_time) < Duration::from_secs(5) {
                                    false
                                } else {
                                    alert_times.insert(payload.key.clone(), now);
                                    true
                                }
                            } else {
                                alert_times.insert(payload.key.clone(), now);
                                true
                            }
                        }
                    };

                    if should_alert {
                        info!("Daemon mirroring notification: {:?}", payload);
                    } else {
                        debug!("Daemon mirroring notification (silent update): {:?}", payload);
                    }

                    ACTIVE_NOTIFICATIONS.lock().insert(payload.key.clone(), payload.clone());
                    
                    #[cfg(not(target_os = "android"))]
                    {
                        if should_alert {
                            let title = format!("{} ({})", payload.title, payload.app_name);
                            let _ = notify_rust::Notification::new()
                                .summary(&title)
                                .body(&payload.text)
                                .show();
                        }
                    }

                    #[cfg(target_os = "android")]
                    {
                        sync_manager.broadcast(SyncMessage::NotificationMirror(payload.clone()));
                    }

                    broadcast_event(IpcMessage::NotificationMirrored(payload));
                }
                IpcMessage::NotificationDismissed { key } => {
                    info!("Daemon processing remote dismiss: {}", key);
                    ACTIVE_NOTIFICATIONS.lock().remove(&key);
                    LAST_ALERT_TIMESTAMPS.lock().remove(&key);
                    
                    #[cfg(target_os = "android")]
                    {
                        let _ = tx.send(IpcMessage::DismissNotification { key: key.clone() });
                    }

                    broadcast_event(IpcMessage::NotificationDismissed { key });
                }
                _ => {
                    info!("Daemon: Unhandled message: {:?}", msg);
                }
            }
        }

        thread::sleep(Duration::from_millis(10));
        count += 1;
    }
}

#[cfg(not(target_os = "android"))]
fn decode_png_to_image(png_bytes: &[u8]) -> Result<arboard::ImageData<'static>, anyhow::Error> {
    let img = image::load_from_memory(png_bytes)?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    let raw_pixels = rgba.into_raw();
    Ok(arboard::ImageData {
        width: width as usize,
        height: height as usize,
        bytes: std::borrow::Cow::Owned(raw_pixels),
    })
}
