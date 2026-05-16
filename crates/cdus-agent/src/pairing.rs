use crate::store::Store;
use anyhow::Result;
use cdus_common::{IpcMessage, SyncMessage, TransportType};
use flume::Sender;
use snow::{params::NoiseParams, Builder, HandshakeState, TransportState};
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use tracing::{error, info, warn};
use tungstenite::{accept, client, Message, WebSocket};

use crate::relay::RelayManager;
use crate::turn_manager::{TurnConnection, TurnManager};
use libp2p::Multiaddr;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub enum RelaySignal {
    Noise(Vec<u8>),
    TurnCandidate { relayed_addr: SocketAddr },
    Libp2pCandidate { multiaddr: Multiaddr },
}

#[derive(Serialize, Deserialize, Debug)]
pub struct HandshakePayload {
    pub label: String,
    pub node_id: String,
}

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
    peers: Mutex<HashMap<String, (Sender<SyncMessage>, TransportType)>>,
}

impl SyncManager {
    pub fn new() -> Self {
        Self {
            peers: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for SyncManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncManager {
    #[tracing::instrument(skip(self, tx))]
    pub fn add_peer(&self, node_id: String, tx: Sender<SyncMessage>, transport: TransportType) {
        let mut peers = self.peers.lock().unwrap();
        peers.insert(node_id, (tx, transport));
    }

    #[tracing::instrument(skip(self))]
    pub fn remove_peer(&self, node_id: &str) {
        let mut peers = self.peers.lock().unwrap();
        peers.remove(node_id);
    }

    #[tracing::instrument(skip(self))]
    pub fn broadcast(&self, msg: SyncMessage) {
        let peers = self.peers.lock().unwrap();
        for (id, (tx, _)) in peers.iter() {
            if let Err(e) = tx.send(msg.clone()) {
                error!("Failed to send sync message to peer {}: {}", id, e);
            }
        }
    }

    #[tracing::instrument(skip(self))]
    pub fn send_to_peer(&self, node_id: &str, msg: SyncMessage) -> bool {
        let peers = self.peers.lock().unwrap();
        if let Some((tx, _)) = peers.get(node_id) {
            if let Err(e) = tx.send(msg) {
                error!(
                    "Failed to send sync message to specific peer {}: {}",
                    node_id, e
                );
                false
            } else {
                true
            }
        } else {
            warn!("Attempted to send message to untracked peer {}", node_id);
            false
        }
    }

    pub fn is_connected(&self, node_id: &str) -> bool {
        let peers = self.peers.lock().unwrap();
        peers.contains_key(node_id)
    }

    pub fn get_peer_transport(&self, node_id: &str) -> Option<TransportType> {
        let peers = self.peers.lock().unwrap();
        peers.get(node_id).map(|(_, t)| t.clone())
    }
}

pub struct PairingManager {
    store: Arc<Store>,
    ipc_tx: Sender<IpcMessage>,
    node_id: String,
    private_key: Vec<u8>,
    port: u16,
    active_pairing: Arc<Mutex<Option<ActivePairingState>>>,
    pub sync_manager: Arc<SyncManager>,
    relay_manager: Arc<RelayManager>,
    turn_manager: Arc<TurnManager>,
    pending_turn_sessions: Mutex<HashMap<String, TurnConnection>>,
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
        turn_manager: Arc<TurnManager>,
    ) -> Self {
        Self {
            store,
            ipc_tx,
            node_id,
            private_key,
            port,
            active_pairing,
            sync_manager,
            relay_manager,
            turn_manager,
            pending_turn_sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn handle_relay_message(&self, source_uuid: String, payload: Vec<u8>) {
        info!(
            "Processing relay signaling message from {} ({} bytes)",
            source_uuid,
            payload.len()
        );

        let signal: RelaySignal = match serde_json::from_slice(&payload) {
            Ok(s) => s,
            Err(_) => {
                // Fallback for legacy raw Noise messages if any
                RelaySignal::Noise(payload)
            }
        };

        match signal {
            RelaySignal::Noise(noise_payload) => {
                if let Err(e) = self.handle_noise_signal(source_uuid, noise_payload) {
                    error!("Relay Noise signaling error: {}", e);
                }
            }
            RelaySignal::TurnCandidate { relayed_addr } => {
                self.handle_turn_candidate(source_uuid, relayed_addr)
            }
            RelaySignal::Libp2pCandidate { multiaddr } => {
                self.handle_libp2p_candidate(source_uuid, multiaddr)
            }
        }
    }

    fn handle_noise_signal(&self, source_uuid: String, payload: Vec<u8>) -> Result<()> {
        let mut ap = self.active_pairing.lock().unwrap();

        // 1. If no active pairing, this might be a new incoming pairing request (Responder Message 1)
        if ap.is_none() {
            info!(
                "Received potential new pairing request from {} via relay",
                source_uuid
            );

            let params: NoiseParams = "Noise_XX_25519_ChaChaPoly_BLAKE2s".parse().unwrap();
            let mut builder = Builder::new(params);
            builder = builder.local_private_key(&self.private_key);
            let mut noise = builder.build_responder().unwrap();

            let mut buf = [0u8; 1024];
            match noise.read_message(&payload, &mut buf) {
                Ok(_) => {
                    // Handshake step 1 successful. Now send step 2.
                    let self_label = self
                        .store
                        .get_state("device_name")
                        .unwrap()
                        .unwrap_or_else(|| "Unknown Device".to_string());
                    let self_payload = HandshakePayload {
                        label: self_label,
                        node_id: self.node_id.clone(),
                    };
                    let self_payload_bytes = serde_json::to_vec(&self_payload).unwrap();
                    let mut out_buf = [0u8; 1024];
                    match noise.write_message(&self_payload_bytes, &mut out_buf) {
                        Ok(n) => {
                            info!(
                                "Sending handshake response (step 2) to {} via relay",
                                source_uuid
                            );
                            let sig = RelaySignal::Noise(out_buf[..n].to_vec());
                            let sig_bytes = serde_json::to_vec(&sig).unwrap();
                            if let Err(e) = self
                                .relay_manager
                                .send_signal(source_uuid.clone(), sig_bytes)
                            {
                                error!("Failed to send handshake response via relay: {}", e);
                                return Ok(());
                            }

                            // Initialize active pairing state
                            *ap = Some(ActivePairingState {
                                pin: String::new(),
                                is_initiator: false,
                                remote_id: source_uuid.clone(),
                                remote_label: "Remote Device (Relay)".to_string(),
                                confirmed: Arc::new(Mutex::new(None)),
                                handshake: Arc::new(Mutex::new(Some(noise))),
                            });

                            // Start monitoring for confirmation
                            self.monitor_relay_pairing(source_uuid);
                        }
                        Err(e) => error!("Failed to write Noise message (step 2): {}", e),
                    }
                }
                Err(e) => error!("Failed to read Noise message (step 1) from relay: {}", e),
            }
            return Ok(());
        }

        // 2. If active pairing exists, check if it's the next step
        if let Some(ref mut state) = *ap {
            if state.remote_id != source_uuid {
                warn!(
                    "Received relay message from {} while busy pairing with {}",
                    source_uuid, state.remote_id
                );
                return Ok(());
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

                            if state.is_initiator {
                                let payload: HandshakePayload = serde_json::from_slice(&buf[..len])
                                    .map_err(|e| {
                                        error!(
                                            "Failed to parse handshake payload from relay: {}",
                                            e
                                        );
                                        anyhow::anyhow!("Invalid handshake payload")
                                    })?;
                                let remote_node_id = payload.node_id;
                                let remote_label = payload.label;
                                info!(
                                    "Relay pairing with {} ({}) successful. PIN: {}",
                                    remote_label, remote_node_id, state.pin
                                );
                                state.remote_id = remote_node_id;
                                state.remote_label = remote_label;
                            } else {
                                let payload: HandshakePayload = serde_json::from_slice(&buf[..len])
                                    .map_err(|e| {
                                        error!(
                                            "Failed to parse handshake payload from relay: {}",
                                            e
                                        );
                                        anyhow::anyhow!("Invalid handshake payload")
                                    })?;
                                let remote_node_id = payload.node_id;
                                let remote_label = payload.label;
                                info!(
                                    "Relay pairing with {} ({}) successful. PIN: {}",
                                    remote_label, remote_node_id, state.pin
                                );
                                state.remote_id = remote_node_id;
                                state.remote_label = remote_label;
                            }
                        } else {
                            if state.is_initiator {
                                let payload: HandshakePayload = serde_json::from_slice(&buf[..len])
                                    .map_err(|e| {
                                        error!(
                                            "Failed to parse handshake payload from relay: {}",
                                            e
                                        );
                                        anyhow::anyhow!("Invalid handshake payload")
                                    })?;
                                let remote_node_id = payload.node_id;
                                let remote_label = payload.label;
                                info!(
                                    "Received responder payload via relay: {} ({})",
                                    remote_label, remote_node_id
                                );
                                state.remote_id = remote_node_id;
                                state.remote_label = remote_label;

                                let self_label = self
                                    .store
                                    .get_state("device_name")
                                    .unwrap()
                                    .unwrap_or_else(|| "Unknown Device".to_string());
                                let self_payload = HandshakePayload {
                                    label: self_label,
                                    node_id: self.node_id.clone(),
                                };
                                let self_payload_bytes = serde_json::to_vec(&self_payload).unwrap();
                                let mut out_buf = [0u8; 1024];
                                match noise.write_message(&self_payload_bytes, &mut out_buf) {
                                    Ok(n) => {
                                        info!(
                                            "Sending handshake step 3 to {} via relay",
                                            source_uuid
                                        );
                                        let sig = RelaySignal::Noise(out_buf[..n].to_vec());
                                        let sig_bytes = serde_json::to_vec(&sig).unwrap();
                                        let _ = self
                                            .relay_manager
                                            .send_signal(source_uuid.clone(), sig_bytes);

                                        if noise.is_handshake_finished() {
                                            info!("Handshake finished via relay with {} (after sending step 3)", source_uuid);
                                            let h = noise.get_handshake_hash();
                                            state.pin = derive_pin(h);
                                            info!(
                                                "Relay pairing with {} ({}) successful. PIN: {}",
                                                state.remote_label, state.remote_id, state.pin
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to write Noise message (step 3): {}", e)
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => error!("Failed to read Noise message during relay handshake: {}", e),
                }
            }
        }
        Ok(())
    }

    fn handle_turn_candidate(&self, source_uuid: String, relayed_addr: SocketAddr) {
        info!(
            "Received TURN candidate from {}: {}",
            source_uuid, relayed_addr
        );

        // 1. If we have a pending session for this UUID, we can now update its remote address
        // (Wait, my start_session doesn't allow updating remote_addr after start)
        // I'll store it in a map for now.

        let mut ap = self.active_pairing.lock().unwrap();
        if let Some(ref mut state) = *ap {
            if state.remote_id == source_uuid {
                let confirmed = state.confirmed.lock().unwrap();
                if let Some(true) = *confirmed {
                    // Start TURN session if we haven't already
                    if !self
                        .pending_turn_sessions
                        .lock()
                        .unwrap()
                        .contains_key(&source_uuid)
                    {
                        if let Ok(creds) = self.relay_manager.get_turn_credentials() {
                            match self.turn_manager.start_session(creds, Some(relayed_addr)) {
                                Ok((conn, _handle)) => {
                                    // Send our candidate back if we haven't
                                    let sig = RelaySignal::TurnCandidate {
                                        relayed_addr: conn.local_relayed_addr,
                                    };
                                    let sig_bytes = serde_json::to_vec(&sig).unwrap();
                                    let _ = self
                                        .relay_manager
                                        .send_signal(source_uuid.clone(), sig_bytes);

                                    // Initiate sync session
                                    let mut hs = state.handshake.lock().unwrap();
                                    if let Some(noise) = hs.take() {
                                        if let Ok(transport) = noise.into_transport_mode() {
                                            let remote_node_id = state.remote_id.clone();
                                            let remote_label = state.remote_label.clone();
                                            let sync_manager = Arc::clone(&self.sync_manager);
                                            let ipc_tx = self.ipc_tx.clone();
                                            let remote_uuid = source_uuid.clone();

                                            thread::spawn(move || {
                                                if let Err(e) = run_turn_sync_session(
                                                    conn,
                                                    transport,
                                                    remote_node_id,
                                                    remote_label,
                                                    sync_manager,
                                                    ipc_tx,
                                                ) {
                                                    error!(
                                                        "TURN sync session error for {}: {}",
                                                        remote_uuid, e
                                                    );
                                                }
                                            });

                                            // Pairing successful
                                            let _ = self.store.add_paired_device(
                                                &source_uuid,
                                                &state.remote_label,
                                            );
                                            let _ = self.ipc_tx.send(IpcMessage::PairingResult {
                                                success: true,
                                                node_id: source_uuid,
                                                label: state.remote_label.clone(),
                                            });
                                        }
                                    }
                                }
                                Err(e) => error!("Failed to start TURN session: {}", e),
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_libp2p_candidate(&self, source_uuid: String, multiaddr: Multiaddr) {
        info!(
            "Received Libp2p candidate from {}: {}",
            source_uuid, multiaddr
        );
        // In the future, we could bridge this to Libp2pManager to dial directly
    }

    fn monitor_relay_pairing(&self, remote_uuid: String) {
        let active_pairing = Arc::clone(&self.active_pairing);
        let relay_manager = Arc::clone(&self.relay_manager);
        let turn_manager = Arc::clone(&self.turn_manager);

        thread::spawn(move || {
            info!("Monitoring relay pairing for {}", remote_uuid);
            loop {
                let status = {
                    let ap = active_pairing.lock().unwrap();
                    if let Some(ref state) = *ap {
                        if state.remote_id == remote_uuid {
                            let res = state.confirmed.lock().unwrap();
                            *res
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                };

                if let Some(accepted) = status {
                    if accepted {
                        info!(
                            "Relay pairing confirmed locally for {}. Fetching TURN credentials.",
                            remote_uuid
                        );
                        if let Ok(creds) = relay_manager.get_turn_credentials() {
                            // Initiator allocates and sends candidate first
                            match turn_manager.start_session(creds, None) {
                                Ok((conn, _handle)) => {
                                    let relayed_addr = conn.local_relayed_addr;
                                    let sig = RelaySignal::TurnCandidate { relayed_addr };
                                    let sig_bytes = serde_json::to_vec(&sig).unwrap();
                                    let _ =
                                        relay_manager.send_signal(remote_uuid.clone(), sig_bytes);

                                    // The session will be started in handle_turn_candidate when peer responds
                                }
                                Err(e) => error!(
                                    "Failed to start TURN session for {}: {}",
                                    remote_uuid, e
                                ),
                            }
                        }
                    }
                    break;
                }
                thread::sleep(Duration::from_millis(500));
            }
        });
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
                info!(
                    "Sending handshake initiation (step 1) to {} via relay",
                    target_uuid
                );
                let sig = RelaySignal::Noise(buf[..n].to_vec());
                let sig_bytes = serde_json::to_vec(&sig).unwrap();
                if let Err(e) = self
                    .relay_manager
                    .send_signal(target_uuid.clone(), sig_bytes)
                {
                    error!("Failed to send handshake initiation via relay: {}", e);
                    return;
                }

                let mut ap = self.active_pairing.lock().unwrap();
                *ap = Some(ActivePairingState {
                    pin: String::new(),
                    is_initiator: true,
                    remote_id: target_uuid.clone(),
                    remote_label: "Remote Device (Relay)".to_string(),
                    confirmed: Arc::new(Mutex::new(None)),
                    handshake: Arc::new(Mutex::new(Some(noise))),
                });

                self.monitor_relay_pairing(target_uuid);
            }
            Err(e) => error!("Failed to write Noise message (step 1): {}", e),
        }
    }

    #[tracing::instrument(skip(self))]
    pub fn start_listener(&self) {
        use socket2::{Domain, Protocol, Socket, Type};

        let addr: SocketAddr = format!("0.0.0.0:{}", self.port).parse().unwrap();

        let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))
            .expect("Failed to create socket");
        socket
            .set_reuse_address(true)
            .expect("Failed to set reuse_address");
        #[cfg(not(windows))]
        socket.set_reuse_port(true).ok(); // Best effort for reuse_port

        socket.bind(&addr.into()).expect("Failed to bind socket");
        socket.listen(128).expect("Failed to listen on socket");

        let listener: TcpListener = socket.into();
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
                        if let Err(e) = handle_incoming_connection(
                            stream,
                            store,
                            ipc_tx,
                            priv_key,
                            active_pairing,
                            self_node_id,
                            sync_manager,
                        ) {
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
                    if let Err(e) = handle_outgoing_connection(
                        ws,
                        store,
                        ipc_tx,
                        priv_key,
                        active_pairing,
                        self_node_id,
                        sync_manager,
                    ) {
                        error!("Error in outgoing connection: {}", e);
                    }
                }
                Err(e) => error!("WebSocket client handshake failed: {}", e),
            }
        });
    }
}

fn run_turn_sync_session(
    conn: TurnConnection,
    mut transport: TransportState,
    node_id: String,
    label: String,
    sync_manager: Arc<SyncManager>,
    ipc_tx: Sender<IpcMessage>,
) -> Result<()> {
    let (tx, rx) = flume::unbounded::<SyncMessage>();
    sync_manager.add_peer(node_id.clone(), tx, TransportType::Relay);

    info!("TURN Sync session started for {} ({})", label, node_id);

    loop {
        // 1. Check for incoming messages from peer via TURN
        if let Ok(data) = conn.rx.try_recv() {
            let mut out = vec![0u8; data.len()];
            match transport.read_message(&data, &mut out) {
                Ok(n) => {
                    out.truncate(n);
                    if let Ok(msg) = serde_json::from_slice::<SyncMessage>(&out) {
                        match msg {
                            SyncMessage::ClipboardUpdate { content, timestamp } => {
                                info!(
                                    "Received clipboard update from peer {} via TURN: {}",
                                    label, content
                                );
                                let _ = ipc_tx.send(IpcMessage::SetClipboard {
                                    content,
                                    timestamp,
                                    source: label.clone(),
                                });
                            }
                            SyncMessage::FileTransferRequest(manifest) => {
                                info!("Received file transfer request from peer {} via TURN: {}", label, manifest.file_name);
                                let _ = ipc_tx.send(IpcMessage::IncomingFileRequest {
                                    node_id: node_id.clone(),
                                    manifest,
                                });
                            }
                            SyncMessage::FileTransferAccepted { file_hash } => {
                                info!("Peer {} accepted file transfer via TURN: {}", label, file_hash);
                                let _ = ipc_tx.send(IpcMessage::AcceptFileTransfer { file_hash });
                            }
                            SyncMessage::FileTransferRejected { file_hash } => {
                                info!("Peer {} rejected file transfer via TURN: {}", label, file_hash);
                                let _ = ipc_tx.send(IpcMessage::RejectFileTransfer { file_hash });
                            }
                            _ => {
                                warn!("Received unhandled sync message from peer {} via TURN: {:?}", label, msg);
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Noise decryption failed via TURN: {}", e);
                    break;
                }
            }
        }

        // 2. Check for outgoing messages
        if let Ok(msg) = rx.try_recv() {
            let data = serde_json::to_vec(&msg)?;
            let mut buf = vec![0u8; data.len() + 1024];
            match transport.write_message(&data, &mut buf) {
                Ok(n) => {
                    if let Err(e) = conn.tx.send(buf[..n].to_vec()) {
                        error!("Failed to send to TURN thread: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    error!("Noise encryption failed: {}", e);
                    break;
                }
            }
        }

        thread::sleep(Duration::from_millis(100));
    }

    sync_manager.remove_peer(&node_id);
    Ok(())
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
    let res = handle_incoming_connection_inner(
        stream,
        Arc::clone(&store),
        ipc_tx.clone(),
        priv_key,
        Arc::clone(&active_pairing),
        self_node_id,
        Arc::clone(&sync_manager),
    );

    if let Err(e) = res {
        error!("Error in incoming connection: {}", e);
        // Ensure UI is notified of failure if it was an active pairing attempt
        let _ = ipc_tx.send(IpcMessage::PairingResult {
            success: false,
            node_id: "unknown".to_string(),
            label: "unknown".to_string(),
        });
        return Err(e);
    }
    Ok(())
}

fn handle_incoming_connection_inner(
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

    let self_label = store
        .get_state("device_name")?
        .unwrap_or_else(|| "Unknown Device".to_string());

    let params: NoiseParams = "Noise_XX_25519_ChaChaPoly_BLAKE2s"
        .parse()
        .map_err(|e: snow::Error| anyhow::anyhow!(e))?;
    let mut builder = Builder::new(params);
    builder = builder.local_private_key(&priv_key);
    let mut noise = builder.build_responder().map_err(|e| anyhow::anyhow!(e))?;

    let mut buf = [0u8; 2048];

    // 1. Read e
    let msg = ws.read()?;
    if let Message::Binary(data) = msg {
        noise
            .read_message(&data, &mut [0u8; 2048])
            .map_err(|e| anyhow::anyhow!(e))?;
    } else {
        return Err(anyhow::anyhow!("Expected binary Noise message"));
    }

    // 2. Write e, ee, s, es + Responder's payload
    let self_payload = HandshakePayload {
        label: self_label.clone(),
        node_id: self_node_id.clone(),
    };
    let self_payload_bytes = serde_json::to_vec(&self_payload).map_err(|e| anyhow::anyhow!(e))?;
    let n = noise
        .write_message(&self_payload_bytes, &mut buf)
        .map_err(|e| anyhow::anyhow!(e))?;
    ws.send(Message::Binary(buf[..n].to_vec()))?;

    let mut initiator_payload_buf = [0u8; 2048];
    let msg = ws.read()?;
    if let Message::Binary(data) = msg {
        let payload_len = noise
            .read_message(&data, &mut initiator_payload_buf)
            .map_err(|e| {
                error!("Noise decryption failed for initiator payload: {}", e);
                anyhow::anyhow!(e)
            })?;

        if payload_len == 0 {
            return Err(anyhow::anyhow!("Initiator sent an empty handshake payload"));
        }

        let payload_slice = &initiator_payload_buf[..payload_len];
        let initiator_payload: HandshakePayload = serde_json::from_slice(payload_slice)
            .map_err(|e| {
                let raw = String::from_utf8_lossy(payload_slice);
                error!("Failed to parse initiator handshake payload. Len: {}. Raw data: '{}'. Error: {}", payload_len, raw, e);
                anyhow::anyhow!("Invalid handshake payload format")
            })?;

        let initiator_label = initiator_payload.label;
        let remote_node_id = initiator_payload.node_id;

        // Verify Node ID is a valid PeerId
        if let Err(e) = remote_node_id.parse::<libp2p::PeerId>() {
            error!(
                "Remote node provided an invalid Peer ID: {}. Error: {}",
                remote_node_id, e
            );
            return Err(anyhow::anyhow!("Invalid Peer ID format"));
        }

        // Handshake finished.
        let h = noise.get_handshake_hash();
        let pin = derive_pin(h);

        info!(
            "Handshake comparison: self={} remote={}",
            self_node_id, remote_node_id
        );

        if remote_node_id == self_node_id {
            error!("Self-connection detected. Aborting.");
            return Err(anyhow::anyhow!("Self-pairing not allowed"));
        }

        let mut transport = noise
            .into_transport_mode()
            .map_err(|e| anyhow::anyhow!(e))?;

        // Check if paired
        if store.is_device_paired(&remote_node_id)? {
            info!(
                "Trusted device {} ({}) connected. Starting sync session.",
                initiator_label, remote_node_id
            );
            run_sync_session(
                ws,
                transport,
                remote_node_id,
                initiator_label,
                sync_manager,
                ipc_tx,
            )?;
        } else {
            info!(
                "New device {} ({}) requesting pairing. PIN: {}",
                initiator_label, remote_node_id, pin
            );

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

            let _ = ws
                .get_ref()
                .set_read_timeout(Some(Duration::from_millis(100)));

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
                                // Give it a moment to send before possibly closing or transitioning
                                thread::sleep(Duration::from_millis(200));
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
                        if e.kind() != std::io::ErrorKind::WouldBlock
                            && e.kind() != std::io::ErrorKind::TimedOut
                        {
                            error!(
                                "Connection lost while waiting for pairing confirmation: {}",
                                e
                            );
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
                let _ = ipc_tx.send(IpcMessage::PairingResult {
                    success: true,
                    node_id: remote_node_id.clone(),
                    label: initiator_label.clone(),
                });

                // Transition to sync session
                run_sync_session(
                    ws,
                    transport,
                    remote_node_id,
                    initiator_label,
                    sync_manager,
                    ipc_tx,
                )?;
            } else {
                info!(
                    "Pairing failed: local_confirmed={}, remote_confirmed={}",
                    local_confirmed, remote_confirmed
                );
                let _ = ipc_tx.send(IpcMessage::PairingResult {
                    success: false,
                    node_id: remote_node_id,
                    label: initiator_label,
                });
            }
        }
    } else {
        return Err(anyhow::anyhow!("Expected binary Noise message"));
    }

    Ok(())
}

fn handle_outgoing_connection(
    ws: WebSocket<TcpStream>,
    store: Arc<Store>,
    ipc_tx: Sender<IpcMessage>,
    priv_key: Vec<u8>,
    active_pairing: Arc<Mutex<Option<ActivePairingState>>>,
    self_node_id: String,
    sync_manager: Arc<SyncManager>,
) -> Result<()> {
    let res = handle_outgoing_connection_inner(
        ws,
        Arc::clone(&store),
        ipc_tx.clone(),
        priv_key,
        Arc::clone(&active_pairing),
        self_node_id,
        Arc::clone(&sync_manager),
    );

    if let Err(e) = res {
        error!("Error in outgoing connection: {}", e);
        let _ = ipc_tx.send(IpcMessage::PairingResult {
            success: false,
            node_id: "unknown".to_string(),
            label: "unknown".to_string(),
        });
        return Err(e);
    }
    Ok(())
}

fn handle_outgoing_connection_inner(
    mut ws: WebSocket<TcpStream>,
    store: Arc<Store>,
    ipc_tx: Sender<IpcMessage>,
    priv_key: Vec<u8>,
    active_pairing: Arc<Mutex<Option<ActivePairingState>>>,
    self_node_id: String,
    sync_manager: Arc<SyncManager>,
) -> Result<()> {
    info!("Initiating outgoing Noise connection over WebSocket");

    let self_label = store
        .get_state("device_name")?
        .unwrap_or_else(|| "Unknown Device".to_string());

    let params: NoiseParams = "Noise_XX_25519_ChaChaPoly_BLAKE2s"
        .parse()
        .map_err(|e: snow::Error| anyhow::anyhow!(e))?;
    let mut builder = Builder::new(params);
    builder = builder.local_private_key(&priv_key);
    let mut noise = builder.build_initiator().map_err(|e| anyhow::anyhow!(e))?;

    let mut buf = [0u8; 2048];

    // 1. Write e
    let n = noise
        .write_message(&[], &mut buf)
        .map_err(|e| anyhow::anyhow!(e))?;
    ws.send(Message::Binary(buf[..n].to_vec()))?;

    let mut responder_payload_buf = [0u8; 2048];
    let msg = ws.read()?;
    if let Message::Binary(data) = msg {
        let payload_len = noise
            .read_message(&data, &mut responder_payload_buf)
            .map_err(|e| {
                error!("Noise decryption failed for responder payload: {}", e);
                anyhow::anyhow!(e)
            })?;

        if payload_len == 0 {
            return Err(anyhow::anyhow!("Responder sent an empty handshake payload"));
        }

        let payload_slice = &responder_payload_buf[..payload_len];
        let responder_payload: HandshakePayload = serde_json::from_slice(payload_slice)
            .map_err(|e| {
                let raw = String::from_utf8_lossy(payload_slice);
                error!("Failed to parse responder handshake payload. Len: {}. Raw data: '{}'. Error: {}", payload_len, raw, e);
                anyhow::anyhow!("Invalid handshake payload format from responder")
            })?;

        let responder_label = responder_payload.label;
        let remote_node_id = responder_payload.node_id;

        // Verify Node ID is a valid PeerId
        if let Err(e) = remote_node_id.parse::<libp2p::PeerId>() {
            error!(
                "Remote responder provided an invalid Peer ID: {}. Error: {}",
                remote_node_id, e
            );
            return Err(anyhow::anyhow!("Invalid Peer ID format from responder"));
        }

        // 3. Write s, se + Initiator's payload
        let self_payload = HandshakePayload {
            label: self_label,
            node_id: self_node_id.clone(),
        };
        let self_payload_bytes =
            serde_json::to_vec(&self_payload).map_err(|e| anyhow::anyhow!(e))?;
        let n = noise
            .write_message(&self_payload_bytes, &mut buf)
            .map_err(|e| anyhow::anyhow!(e))?;
        ws.send(Message::Binary(buf[..n].to_vec()))?;

        // Handshake finished.
        let h = noise.get_handshake_hash();
        let pin = derive_pin(h);

        info!(
            "Handshake comparison: self={} remote={}",
            self_node_id, remote_node_id
        );

        if remote_node_id == self_node_id {
            error!("Self-connection detected. Aborting.");
            return Err(anyhow::anyhow!("Self-pairing not allowed"));
        }

        let mut transport = noise
            .into_transport_mode()
            .map_err(|e| anyhow::anyhow!(e))?;

        // Check if paired
        if store.is_device_paired(&remote_node_id)? {
            info!(
                "Connecting to trusted device {} ({}) for sync.",
                responder_label, remote_node_id
            );
            run_sync_session(
                ws,
                transport,
                remote_node_id,
                responder_label,
                sync_manager,
                ipc_tx,
            )?;
        } else {
            info!(
                "Initiating pairing with {} ({}). PIN: {}",
                responder_label, remote_node_id, pin
            );

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

            let _ = ws
                .get_ref()
                .set_read_timeout(Some(Duration::from_millis(100)));

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
                        if e.kind() != std::io::ErrorKind::WouldBlock
                            && e.kind() != std::io::ErrorKind::TimedOut
                        {
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
                                // Give it a moment to send
                                thread::sleep(Duration::from_millis(200));
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
                let _ = ipc_tx.send(IpcMessage::PairingResult {
                    success: true,
                    node_id: remote_node_id.clone(),
                    label: responder_label.clone(),
                });

                // Transition to sync session
                run_sync_session(
                    ws,
                    transport,
                    remote_node_id,
                    responder_label,
                    sync_manager,
                    ipc_tx,
                )?;
            } else {
                info!(
                    "Pairing failed: local_confirmed={}, remote_confirmed={}",
                    local_confirmed, remote_confirmed
                );
                let _ = ipc_tx.send(IpcMessage::PairingResult {
                    success: false,
                    node_id: remote_node_id,
                    label: responder_label,
                });
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
    sync_manager.add_peer(node_id.clone(), tx, TransportType::Lan);

    info!(
        "Sync session started for {} ({}) over WebSocket",
        label, node_id
    );

    // Ensure non-blocking for read-loop if needed, but we'll use can_read or short timeouts
    let _ = ws
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(100)));

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
                                source: label.clone(),
                            });
                        }
                        SyncMessage::FileTransferRequest(manifest) => {
                            info!("Received file transfer request from peer {}: {}", label, manifest.file_name);
                            let _ = ipc_tx.send(IpcMessage::IncomingFileRequest {
                                node_id: node_id.clone(),
                                manifest,
                            });
                        }
                        SyncMessage::FileTransferAccepted { file_hash } => {
                            info!("Peer {} accepted file transfer: {}", label, file_hash);
                            let _ = ipc_tx.send(IpcMessage::AcceptFileTransfer { file_hash });
                        }
                        SyncMessage::FileTransferRejected { file_hash } => {
                            info!("Peer {} rejected file transfer: {}", label, file_hash);
                            let _ = ipc_tx.send(IpcMessage::RejectFileTransfer { file_hash });
                        }
                        _ => {
                            warn!("Received unhandled sync message from peer {}: {:?}", label, msg);
                        }
                    }
                }
            }
            Err(e) => {
                if e.kind() != std::io::ErrorKind::WouldBlock
                    && e.kind() != std::io::ErrorKind::TimedOut
                {
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

fn write_ws_framed(
    ws: &mut WebSocket<TcpStream>,
    transport: &mut TransportState,
    data: &[u8],
) -> Result<()> {
    let mut buf = vec![0u8; data.len() + 1024];
    let n = transport
        .write_message(data, &mut buf)
        .map_err(|e| anyhow::anyhow!(e))?;
    ws.send(Message::Binary(buf[..n].to_vec()))?;
    Ok(())
}

fn read_ws_framed(
    ws: &mut WebSocket<TcpStream>,
    transport: &mut TransportState,
) -> Result<Vec<u8>, std::io::Error> {
    match ws.read() {
        Ok(Message::Binary(data)) => {
            let mut out = vec![0u8; data.len() + 1024]; // Ensure enough space
            match transport.read_message(&data, &mut out) {
                Ok(n) => {
                    out.truncate(n);
                    Ok(out)
                }
                Err(e) => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            }
        }
        Ok(Message::Close(_)) => Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            "Connection closed by peer",
        )),
        Ok(msg) => {
            info!("Received non-binary message: {:?}", msg);
            Ok(Vec::new()) // Ignore other types or return error
        }
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

        sm.add_peer("node1".to_string(), tx, TransportType::Lan);
        assert!(sm.is_connected("node1"));
        assert_eq!(sm.get_peer_transport("node1"), Some(TransportType::Lan));

        sm.broadcast(SyncMessage::ClipboardUpdate {
            content: "test".to_string(),
            timestamp: 123,
        });

        let received = rx.recv().unwrap();
        match received {
            SyncMessage::ClipboardUpdate { content, .. } => assert_eq!(content, "test"),
            _ => panic!("Expected ClipboardUpdate"),
        }

        sm.remove_peer("node1");
        assert!(!sm.is_connected("node1"));
    }
}
