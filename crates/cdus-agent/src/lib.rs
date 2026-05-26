pub mod file_transfer;
pub mod libp2p_manager;
pub mod mdns;
pub mod pairing;
pub mod relay;
pub mod store;
pub mod turn_manager;
pub mod utils;

#[cfg(test)]
mod integration_tests;

use cdus_common::{IpcMessage, SyncMessage};
use flume::{Receiver, Sender};
use libp2p_manager::Libp2pManager;
use once_cell::sync::Lazy;
use pairing::{ActivePairingState, PairingManager, SyncManager};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tracing::{error, info};
use store::Store;

pub static EVENT_BUS: Lazy<Arc<Mutex<Vec<Sender<IpcMessage>>>>> =
    Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

pub fn broadcast_event(msg: IpcMessage) {
    let mut bus = EVENT_BUS.lock().unwrap();
    bus.retain(|tx| tx.send(msg.clone()).is_ok());
}

pub fn daemon_loop(
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
    peer_map: Arc<
        Mutex<
            std::collections::HashMap<
                String,
                (String, String, String, u16, std::time::Instant),
            >,
        >,
    >,
    _libp2p_request_tx: Option<Sender<(libp2p::PeerId, SyncMessage)>>,
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
                    // Update global peer map for connection fallback
                    {
                        let mut map = peer_map.lock().unwrap();
                        map.insert(node_id.clone(), (label.clone(), os.clone(), ip.clone(), port, std::time::Instant::now()));
                    }

                    let already_paired = store.is_device_paired(&node_id).unwrap_or(false);

                    if !already_paired {
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
                    
                    if !already_paired {
                        broadcast_event(IpcMessage::DeviceDiscovered {
                            node_id: node_id.clone(),
                            label: label.clone(),
                            os: os.clone(),
                            ip: ip.clone(),
                            port,
                        });
                    }

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
                    broadcast_event(IpcMessage::DeviceLost { node_id });
                }
                IpcMessage::PeerDisconnected { node_id } => {
                    broadcast_event(IpcMessage::PeerDisconnected { node_id });
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
                               Ok(stream) => {
                                   let wrapped_stream = crate::file_transfer::Libp2pFileStream { 
                                       stream, 
                                       runtime: libp2p_manager_clone.runtime_handle() 
                                   };

                                    let session_key = crate::file_transfer::SessionKey([0u8; 32]); // TODO: Real session key
                                    if let Err(e) = crate::file_transfer::handle_outgoing_transfer(
                                        Box::new(wrapped_stream),
                                        store_clone,
                                        transfer_id,
                                        session_key,
                                        transfer_manager,
                                    ) {
                                        error!("Outgoing transfer failed: {}", e);
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to open file stream to {}: {}", peer_id, e);
                                }
                             }
                        } else {
                            error!("Invalid PeerId: {}", node_id);
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
                IpcMessage::FileTransferProgress { transfer_id, progress } => {
                    broadcast_event(IpcMessage::FileTransferProgress { transfer_id, progress });
                }
                IpcMessage::FileTransferComplete { transfer_id } => {
                    broadcast_event(IpcMessage::FileTransferComplete { transfer_id });
                }
                IpcMessage::FileTransferError { transfer_id, error } => {
                    broadcast_event(IpcMessage::FileTransferError { transfer_id, error });
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
