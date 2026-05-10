use snow::{Builder, params::NoiseParams, TransportState, HandshakeState};
use std::net::{TcpListener, TcpStream, SocketAddr};
use tracing::{info, error, warn};
use std::sync::Arc;
use flume::Sender;
use cdus_common::{IpcMessage, SyncMessage};
use crate::store::Store;
use std::sync::Mutex;
use std::time::Duration;
use std::collections::HashMap;
use std::thread;
use anyhow::Result;
use tungstenite::{accept, client, Message, WebSocket};

use crate::relay::RelayManager;

#[derive(Clone)]
pub struct ActivePairingState {
    pub pin: String,
    pub is_initiator: bool,
    pub remote_id: String,
    pub remote_label: String,
    pub confirmed: Arc<Mutex<Option<bool>>>,
    pub handshake: Arc<Mutex<Option<HandshakeState>>>,
}

pub struct SyncManager {
    peers: Mutex<HashMap<String, Sender<SyncMessage>>>,
}

impl SyncManager {
    pub fn new() -> Self {
        Self {
            peers: Mutex::new(HashMap::new()),
        }
    }

    pub fn add_peer(&self, node_id: String, tx: Sender<SyncMessage>) {
        let mut peers = self.peers.lock().unwrap();
        peers.insert(node_id, tx);
    }

    pub fn remove_peer(&self, node_id: &str) {
        let mut peers = self.peers.lock().unwrap();
        peers.remove(node_id);
    }

    pub fn broadcast(&self, msg: SyncMessage) {
        let peers = self.peers.lock().unwrap();
        for (id, tx) in peers.iter() {
            if let Err(e) = tx.send(msg.clone()) {
                error!("Failed to send sync message to peer {}: {}", id, e);
            }
        }
    }

    pub fn is_connected(&self, node_id: &str) -> bool {
        let peers = self.peers.lock().unwrap();
        peers.contains_key(node_id)
    }
}

pub struct PairingManager {
    store: Arc<Store>,
    ipc_tx: Sender<IpcMessage>,
    node_id: String,
    private_key: Vec<u8>,
    port: u16,
    active_pairing: Arc<Mutex<Option<ActivePairingState>>>,
    sync_manager: Arc<SyncManager>,
    relay_manager: Arc<RelayManager>,
}

impl PairingManager {
    pub fn new(
        store: Arc<Store>, 
        ipc_tx: Sender<IpcMessage>, 
        node_id: String, 
        private_key: Vec<u8>, 
        port: u16, 
        active_pairing: Arc<Mutex<Option<ActivePairingState>>>,
        sync_manager: Arc<SyncManager>,
        relay_manager: Arc<RelayManager>,
    ) -> Self {
        Self { store, ipc_tx, node_id, private_key, port, active_pairing, sync_manager, relay_manager }
    }

    pub fn handle_relay_message(&self, source_uuid: String, payload: Vec<u8>) {
        info!("Processing relay signaling message from {} ({} bytes)", source_uuid, payload.len());
        
        let mut ap = self.active_pairing.lock().unwrap();
        
        // 1. If no active pairing, this might be a new incoming pairing request (Responder Message 1)
        if ap.is_none() {
            info!("Received potential new pairing request from {} via relay", source_uuid);
            
            let params: NoiseParams = "Noise_XX_25519_ChaChaPoly_BLAKE2s".parse().unwrap();
            let mut builder = Builder::new(params);
            builder = builder.local_private_key(&self.private_key);
            let mut noise = builder.build_responder().unwrap();
            
            let mut buf = [0u8; 1024];
            match noise.read_message(&payload, &mut buf) {
                Ok(_) => {
                    // Handshake step 1 successful. Now send step 2.
                    let self_label = self.store.get_state("device_name").unwrap().unwrap_or_else(|| "Unknown Device".to_string());
                    let mut out_buf = [0u8; 1024];
                    match noise.write_message(self_label.as_bytes(), &mut out_buf) {
                        Ok(n) => {
                            info!("Sending handshake response (step 2) to {} via relay", source_uuid);
                            if let Err(e) = self.relay_manager.send_signal(source_uuid.clone(), out_buf[..n].to_vec()) {
                                error!("Failed to send handshake response via relay: {}", e);
                                return;
                            }
                            
                            // Initialize active pairing state
                            *ap = Some(ActivePairingState {
                                pin: String::new(), 
                                is_initiator: false,
                                remote_id: source_uuid,
                                remote_label: "Remote Device (Relay)".to_string(), 
                                confirmed: Arc::new(Mutex::new(None)),
                                handshake: Arc::new(Mutex::new(Some(noise))),
                            });
                        }
                        Err(e) => error!("Failed to write Noise message (step 2): {}", e),
                    }
                }
                Err(e) => error!("Failed to read Noise message (step 1) from relay: {}", e),
            }
            return;
        }

        // 2. If active pairing exists, check if it's the next step
        if let Some(ref mut state) = *ap {
            if state.remote_id != source_uuid {
                warn!("Received relay message from {} while busy pairing with {}", source_uuid, state.remote_id);
                return;
            }

            let mut handshake_lock = state.handshake.lock().unwrap();
            if let Some(ref mut noise) = *handshake_lock {
                let mut buf = [0u8; 1024];
                match noise.read_message(&payload, &mut buf) {
                    Ok(len) => {
                        if noise.is_handshake_finished() {
                            info!("Handshake finished via relay with {}", source_uuid);
                            
                            let h = noise.get_handshake_hash();
                            state.pin = derive_pin(h);
                            let remote_node_id = hex::encode(noise.get_remote_static().unwrap());
                            
                            if state.is_initiator {
                                info!("Relay pairing with {} ({}) successful. PIN: {}", state.remote_label, remote_node_id, state.pin);
                            } else {
                                let remote_label = String::from_utf8_lossy(&buf[..len]).to_string();
                                info!("Relay pairing with {} ({}) successful. PIN: {}", remote_label, remote_node_id, state.pin);
                                state.remote_label = remote_label;
                            }
                        } else {
                            if state.is_initiator {
                                let remote_label = String::from_utf8_lossy(&buf[..len]).to_string();
                                info!("Received responder label via relay: {}", remote_label);
                                state.remote_label = remote_label;

                                let self_label = self.store.get_state("device_name").unwrap().unwrap_or_else(|| "Unknown Device".to_string());
                                let mut out_buf = [0u8; 1024];
                                match noise.write_message(self_label.as_bytes(), &mut out_buf) {
                                    Ok(n) => {
                                        info!("Sending handshake step 3 to {} via relay", source_uuid);
                                        let _ = self.relay_manager.send_signal(source_uuid.clone(), out_buf[..n].to_vec());

                                        if noise.is_handshake_finished() {
                                            info!("Handshake finished via relay with {} (after sending step 3)", source_uuid);
                                            let h = noise.get_handshake_hash();
                                            state.pin = derive_pin(h);
                                            let remote_node_id = hex::encode(noise.get_remote_static().unwrap());
                                            info!("Relay pairing with {} ({}) successful. PIN: {}", state.remote_label, remote_node_id, state.pin);
                                        }
                                    }
                                    Err(e) => error!("Failed to write Noise message (step 3): {}", e),
                                }
                            }
                        }
                    }
                    Err(e) => error!("Failed to read Noise message during relay handshake: {}", e),
                }
            }
        }
    }

    pub fn initiate_remote_pairing(&self, target_uuid: String) {
        info!("Initiating remote pairing with {} via relay", target_uuid);
        
        let params: NoiseParams = "Noise_XX_25519_ChaChaPoly_BLAKE2s".parse().unwrap();
        let mut builder = Builder::new(params);
        builder = builder.local_private_key(&self.private_key);
        let mut noise = builder.build_initiator().unwrap();
        
        let mut buf = [0u8; 1024];
        match noise.write_message(&[], &mut buf) {
            Ok(n) => {
                info!("Sending handshake initiation (step 1) to {} via relay", target_uuid);
                if let Err(e) = self.relay_manager.send_signal(target_uuid.clone(), buf[..n].to_vec()) {
                    error!("Failed to send handshake initiation via relay: {}", e);
                    return;
                }
                
                let mut ap = self.active_pairing.lock().unwrap();
                *ap = Some(ActivePairingState {
                    pin: String::new(),
                    is_initiator: true,
                    remote_id: target_uuid,
                    remote_label: "Remote Device (Relay)".to_string(),
                    confirmed: Arc::new(Mutex::new(None)),
                    handshake: Arc::new(Mutex::new(Some(noise))),
                });
            }
            Err(e) => error!("Failed to write Noise message (step 1): {}", e),
        }
    }

    pub fn start_listener(&self) {
        let addr = format!("0.0.0.0:{}", self.port);
        let listener = match TcpListener::bind(&addr) {
            Ok(l) => l,
            Err(e) => {
                error!("Failed to bind pairing listener on {}: {}", addr, e);
                return;
            }
        };

        info!("Pairing listener active on {}", addr);

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let store = Arc::clone(&self.store);
                    let ipc_tx = self.ipc_tx.clone();
                    let priv_key = self.private_key.clone();
                    let active_pairing = Arc::clone(&self.active_pairing);
                    let self_node_id = self.node_id.clone();
                    let sync_manager = Arc::clone(&self.sync_manager);
                    thread::spawn(move || {
                        if let Err(e) = handle_incoming_connection(stream, store, ipc_tx, priv_key, active_pairing, self_node_id, sync_manager) {
                            error!("Error in incoming connection: {}", e);
                        }
                    });
                }
                Err(e) => error!("Failed to accept connection: {}", e),
            }
        }
    }

    pub fn initiate_pairing(&self, target_addr: SocketAddr) {
        let stream = match TcpStream::connect_timeout(&target_addr, Duration::from_secs(5)) {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to connect to target {}: {}", target_addr, e);
                return;
            }
        };

        let store = Arc::clone(&self.store);
        let ipc_tx = self.ipc_tx.clone();
        let priv_key = self.private_key.clone();
        let active_pairing = Arc::clone(&self.active_pairing);
        let self_node_id = self.node_id.clone();
        let sync_manager = Arc::clone(&self.sync_manager);

        thread::spawn(move || {
            // Upgrade to WebSocket
            let url = format!("ws://{}/pairing", target_addr);
            match client(url, stream) {
                Ok((ws, _)) => {
                    if let Err(e) = handle_outgoing_connection(ws, store, ipc_tx, priv_key, active_pairing, self_node_id, sync_manager) {
                        error!("Error in outgoing connection: {}", e);
                    }
                }
                Err(e) => error!("WebSocket client handshake failed: {}", e),
            }
        });
    }
}

fn handle_incoming_connection(
    stream: TcpStream, 
    store: Arc<Store>, 
    ipc_tx: Sender<IpcMessage>, 
    priv_key: Vec<u8>, 
    active_pairing: Arc<Mutex<Option<ActivePairingState>>>, 
    self_node_id: String,
    sync_manager: Arc<SyncManager>,
) -> Result<()> {
    info!("Upgrading incoming connection to WebSocket");
    let mut ws = accept(stream)?;

    info!("Handling incoming Noise connection over WebSocket");

    let self_label = store.get_state("device_name")?.unwrap_or_else(|| "Unknown Device".to_string());

    let params: NoiseParams = "Noise_XX_25519_ChaChaPoly_BLAKE2s".parse().map_err(|e: snow::Error| anyhow::anyhow!(e))?;
    let mut builder = Builder::new(params);
    builder = builder.local_private_key(&priv_key);
    let mut noise = builder.build_responder().map_err(|e| anyhow::anyhow!(e))?;

    let mut buf = [0u8; 1024];

    // 1. Read e
    let msg = ws.read()?;
    if let Message::Binary(data) = msg {
        noise.read_message(&data, &mut [0u8; 1024]).map_err(|e| anyhow::anyhow!(e))?;
    } else {
        return Err(anyhow::anyhow!("Expected binary Noise message"));
    }

    // 2. Write e, ee, s, es + Responder's label
    let n = noise.write_message(self_label.as_bytes(), &mut buf).map_err(|e| anyhow::anyhow!(e))?;
    ws.send(Message::Binary(buf[..n].to_vec()))?;

    // 3. Read s, se + Initiator's label
    let mut initiator_label_buf = [0u8; 1024];
    let msg = ws.read()?;
    if let Message::Binary(data) = msg {
        let label_len = noise.read_message(&data, &mut initiator_label_buf).map_err(|e| anyhow::anyhow!(e))?;
        let initiator_label = String::from_utf8_lossy(&initiator_label_buf[..label_len]).to_string();

        // Handshake finished.
        let h = noise.get_handshake_hash();
        let pin = derive_pin(h);
        let remote_node_id = hex::encode(noise.get_remote_static().unwrap());

        info!("Handshake comparison: self={} remote={}", self_node_id, remote_node_id);

        if remote_node_id == self_node_id {
            error!("Self-connection detected. Aborting.");
            return Err(anyhow::anyhow!("Self-pairing not allowed"));
        }

        let mut transport = noise.into_transport_mode().map_err(|e| anyhow::anyhow!(e))?;

        // Check if paired
        if store.is_device_paired(&remote_node_id)? {
            info!("Trusted device {} ({}) connected. Starting sync session.", initiator_label, remote_node_id);
            run_sync_session(ws, transport, remote_node_id, initiator_label, sync_manager, ipc_tx)?;
        } else {
            info!("New device {} ({}) requesting pairing. PIN: {}", initiator_label, remote_node_id, pin);

            // Update state for UI
            let confirmed = Arc::new(Mutex::new(None::<bool>));
            {
                let mut ap = active_pairing.lock().unwrap();
                *ap = Some(ActivePairingState {
                    pin: pin.clone(),
                    is_initiator: false,
                    remote_id: remote_node_id.clone(),
                    remote_label: initiator_label.clone(),
                    confirmed: Arc::clone(&confirmed),
                    handshake: Arc::new(Mutex::new(None)),
                });
            }

            // Wait for both local and remote confirmation
            let mut local_confirmed = false;
            let mut remote_confirmed = false;

            let _ = ws.get_ref().set_read_timeout(Some(Duration::from_millis(100)));

            loop {
                // Check local confirmation
                {
                    let res = confirmed.lock().unwrap();
                    if let Some(accepted) = *res {
                        if accepted {
                            if !local_confirmed {
                                info!("Local user confirmed. Sending acceptance to initiator.");
                                write_ws_framed(&mut ws, &mut transport, &[1])?;
                                local_confirmed = true;
                            }
                        } else {
                            info!("Local user rejected pairing.");
                            let _ = write_ws_framed(&mut ws, &mut transport, &[0]);
                            break;
                        }
                    }
                }

                // Check remote confirmation
                match read_ws_framed(&mut ws, &mut transport) {
                    Ok(data) => {
                        if !data.is_empty() && data[0] == 1 {
                            info!("Remote initiator confirmed pairing.");
                            remote_confirmed = true;
                        } else {
                            info!("Remote initiator rejected or closed pairing.");
                            break;
                        }
                    }
                    Err(e) => {
                        if e.kind() != std::io::ErrorKind::WouldBlock && e.kind() != std::io::ErrorKind::TimedOut {
                            error!("Connection lost while waiting for pairing confirmation: {}", e);
                            break;
                        }
                    }
                }

                if local_confirmed && remote_confirmed {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }

            // Clear UI state
            {
                let mut ap = active_pairing.lock().unwrap();
                *ap = None;
            }

            if local_confirmed && remote_confirmed {
                info!("Both sides confirmed. Pairing successful.");
                let _ = store.add_paired_device(&remote_node_id, &initiator_label);
                let _ = ipc_tx.send(IpcMessage::PairingResult { success: true, node_id: remote_node_id.clone(), label: initiator_label.clone() });

                // Transition to sync session
                run_sync_session(ws, transport, remote_node_id, initiator_label, sync_manager, ipc_tx)?;
            } else {
                info!("Pairing failed: local_confirmed={}, remote_confirmed={}", local_confirmed, remote_confirmed);
                let _ = ipc_tx.send(IpcMessage::PairingResult { success: false, node_id: remote_node_id, label: initiator_label });
            }
        }
    } else {
        return Err(anyhow::anyhow!("Expected binary Noise message"));
    }

    Ok(())
}

fn handle_outgoing_connection(
    mut ws: WebSocket<TcpStream>, 
    store: Arc<Store>, 
    ipc_tx: Sender<IpcMessage>, 
    priv_key: Vec<u8>, 
    active_pairing: Arc<Mutex<Option<ActivePairingState>>>, 
    self_node_id: String,
    sync_manager: Arc<SyncManager>,
) -> Result<()> {
    info!("Initiating outgoing Noise connection over WebSocket");

    let self_label = store.get_state("device_name")?.unwrap_or_else(|| "Unknown Device".to_string());

    let params: NoiseParams = "Noise_XX_25519_ChaChaPoly_BLAKE2s".parse().map_err(|e: snow::Error| anyhow::anyhow!(e))?;
    let mut builder = Builder::new(params);
    builder = builder.local_private_key(&priv_key);
    let mut noise = builder.build_initiator().map_err(|e| anyhow::anyhow!(e))?;

    let mut buf = [0u8; 1024];

    // 1. Write e
    let n = noise.write_message(&[], &mut buf).map_err(|e| anyhow::anyhow!(e))?;
    ws.send(Message::Binary(buf[..n].to_vec()))?;

    // 2. Read e, ee, s, es + Responder's label
    let mut responder_label_buf = [0u8; 1024];
    let msg = ws.read()?;
    if let Message::Binary(data) = msg {
        let label_len = noise.read_message(&data, &mut responder_label_buf).map_err(|e| anyhow::anyhow!(e))?;
        let responder_label = String::from_utf8_lossy(&responder_label_buf[..label_len]).to_string();

        // 3. Write s, se + Initiator's label
        let n = noise.write_message(self_label.as_bytes(), &mut buf).map_err(|e| anyhow::anyhow!(e))?;
        ws.send(Message::Binary(buf[..n].to_vec()))?;

        // Handshake finished.
        let h = noise.get_handshake_hash();
        let pin = derive_pin(h);
        let remote_node_id = hex::encode(noise.get_remote_static().unwrap());

        info!("Handshake comparison: self={} remote={}", self_node_id, remote_node_id);

        if remote_node_id == self_node_id {
            error!("Self-connection detected. Aborting.");
            return Err(anyhow::anyhow!("Self-pairing not allowed"));
        }

        let mut transport = noise.into_transport_mode().map_err(|e| anyhow::anyhow!(e))?;

        // Check if paired
        if store.is_device_paired(&remote_node_id)? {
            info!("Connecting to trusted device {} ({}) for sync.", responder_label, remote_node_id);
            run_sync_session(ws, transport, remote_node_id, responder_label, sync_manager, ipc_tx)?;
        } else {
            info!("Initiating pairing with {} ({}). PIN: {}", responder_label, remote_node_id, pin);

            // Update state for UI
            let confirmed = Arc::new(Mutex::new(None::<bool>));
            {
                let mut ap = active_pairing.lock().unwrap();
                *ap = Some(ActivePairingState {
                    pin: pin.clone(),
                    is_initiator: true,
                    remote_id: remote_node_id.clone(),
                    remote_label: responder_label.clone(),
                    confirmed: Arc::clone(&confirmed),
                    handshake: Arc::new(Mutex::new(None)),
                });
            }

            // Wait for both local and remote confirmation
            let mut local_confirmed = false;
            let mut remote_confirmed = false;

            let _ = ws.get_ref().set_read_timeout(Some(Duration::from_millis(100)));

            loop {
                // Check remote confirmation
                match read_ws_framed(&mut ws, &mut transport) {
                    Ok(data) => {
                        if !data.is_empty() && data[0] == 1 {
                            info!("Remote responder confirmed pairing.");
                            remote_confirmed = true;
                        } else {
                            info!("Remote responder rejected or closed pairing.");
                            break;
                        }
                    }
                    Err(e) => {
                        if e.kind() != std::io::ErrorKind::WouldBlock && e.kind() != std::io::ErrorKind::TimedOut {
                            error!("Error reading pairing response: {}", e);
                            break;
                        }
                    }
                }

                // Check local confirmation
                {
                    let res = confirmed.lock().unwrap();
                    if let Some(accepted) = *res {
                        if accepted {
                            if !local_confirmed {
                                info!("Local user confirmed. Sending acceptance to responder.");
                                write_ws_framed(&mut ws, &mut transport, &[1])?;
                                local_confirmed = true;
                            }
                        } else {
                            info!("User cancelled pairing locally.");
                            let _ = write_ws_framed(&mut ws, &mut transport, &[0]);
                            break;
                        }
                    }
                }

                if local_confirmed && remote_confirmed {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }

            // Clear UI state
            {
                let mut ap = active_pairing.lock().unwrap();
                *ap = None;
            }

            if local_confirmed && remote_confirmed {
                info!("Both sides confirmed. Pairing successful.");
                let _ = store.add_paired_device(&remote_node_id, &responder_label);
                let _ = ipc_tx.send(IpcMessage::PairingResult { success: true, node_id: remote_node_id.clone(), label: responder_label.clone() });

                // Transition to sync session
                run_sync_session(ws, transport, remote_node_id, responder_label, sync_manager, ipc_tx)?;
            } else {
                info!("Pairing failed: local_confirmed={}, remote_confirmed={}", local_confirmed, remote_confirmed);
                let _ = ipc_tx.send(IpcMessage::PairingResult { success: false, node_id: remote_node_id, label: responder_label });
            }
        }
    } else {
        return Err(anyhow::anyhow!("Expected binary Noise message"));
    }

    Ok(())
}

fn run_sync_session(
    mut ws: WebSocket<TcpStream>,
    mut transport: TransportState,
    node_id: String,
    label: String,
    sync_manager: Arc<SyncManager>,
    ipc_tx: Sender<IpcMessage>,
) -> Result<()> {
    let (tx, rx) = flume::unbounded::<SyncMessage>();
    sync_manager.add_peer(node_id.clone(), tx);

    info!("Sync session started for {} ({}) over WebSocket", label, node_id);

    // Ensure non-blocking for read-loop if needed, but we'll use can_read or short timeouts
    let _ = ws.get_ref().set_read_timeout(Some(Duration::from_millis(100)));

    loop {
        // 1. Check for incoming messages from peer
        match read_ws_framed(&mut ws, &mut transport) {
            Ok(data) => {
                if let Ok(msg) = serde_json::from_slice::<SyncMessage>(&data) {
                    match msg {
                        SyncMessage::ClipboardUpdate { content, timestamp } => {
                            info!("Received clipboard update from peer {}: {}", label, content);
                            let _ = ipc_tx.send(IpcMessage::SetClipboard { 
                                content, 
                                timestamp, 
                                source: label.clone() 
                            });
                        }
                    }
                }
            }
            Err(e) => {
                if e.kind() != std::io::ErrorKind::WouldBlock && e.kind() != std::io::ErrorKind::TimedOut {
                    info!("Peer {} disconnected or error: {}", label, e);
                    break;
                }
            }
        }

        // 2. Check for outgoing messages
        if let Ok(msg) = rx.try_recv() {
            let data = serde_json::to_vec(&msg)?;
            write_ws_framed(&mut ws, &mut transport, &data)?;
        }

        thread::sleep(Duration::from_millis(100));
    }

    sync_manager.remove_peer(&node_id);
    Ok(())
}

fn write_ws_framed(ws: &mut WebSocket<TcpStream>, transport: &mut TransportState, data: &[u8]) -> Result<()> {
    let mut buf = vec![0u8; data.len() + 1024];
    let n = transport.write_message(data, &mut buf).map_err(|e| anyhow::anyhow!(e))?;
    ws.send(Message::Binary(buf[..n].to_vec()))?;
    Ok(())
}

fn read_ws_framed(ws: &mut WebSocket<TcpStream>, transport: &mut TransportState) -> Result<Vec<u8>, std::io::Error> {
    match ws.read() {
        Ok(Message::Binary(data)) => {
            let mut out = vec![0u8; data.len()];
            let n = transport.read_message(&data, &mut out).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            out.truncate(n);
            Ok(out)
        }
        Ok(_) => Err(std::io::Error::new(std::io::ErrorKind::Other, "Expected binary message")),
        Err(tungstenite::Error::Io(e)) => Err(e),
        Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
    }
}

fn derive_pin(h: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(h);
    hasher.update(b"SAS");
    let res = hasher.finalize();
    let bytes = res.as_bytes();
    let val = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    format!("{:04}", val % 10000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flume;

    #[test]
    fn test_derive_pin_consistency() {
        let h = b"test_handshake_hash";
        let pin1 = derive_pin(h);
        let pin2 = derive_pin(h);
        assert_eq!(pin1, pin2);
        assert_eq!(pin1.len(), 4);
    }

    #[test]
    fn test_sync_manager_peers() {
        let sm = SyncManager::new();
        let (tx, rx) = flume::unbounded();
        
        sm.add_peer("node1".to_string(), tx);
        assert!(sm.is_connected("node1"));
        
        sm.broadcast(SyncMessage::ClipboardUpdate { 
            content: "test".to_string(), 
            timestamp: 123 
        });
        
        let received = rx.recv().unwrap();
        match received {
            SyncMessage::ClipboardUpdate { content, .. } => assert_eq!(content, "test"),
        }
        
        sm.remove_peer("node1");
        assert!(!sm.is_connected("node1"));
    }
}
