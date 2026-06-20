use crate::store::Store;
use anyhow::Result;
use cdus_common::{IpcMessage, SyncMessage, TransportType};
use flume::Sender;
use snow::{params::NoiseParams, Builder, HandshakeState, TransportState};
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use parking_lot::Mutex;
use std::thread;
use std::time::Duration;
use tracing::{debug, error, info, warn};
use tungstenite::{accept, client, Message, WebSocket};

use crate::libp2p_manager::Libp2pManager;
use crate::relay::RelayManager;
use crate::turn_manager::{TurnConnection, TurnManager};
use libp2p::Multiaddr;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub enum RelaySignal {
    Noise(Vec<u8>),
    TurnCandidate { relayed_addr: SocketAddr },
    Libp2pCandidate { multiaddr: Multiaddr },
    PairingError { node_id: String, message: String },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum HandshakeIntent {
    Pair,
    Reconnect,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HandshakePayload {
    pub label: String,
    pub node_id: String,
    pub libp2p_addresses: Vec<String>,
    pub oob_secret: Option<String>,
    pub intent: Option<HandshakeIntent>,
}

#[derive(Clone)]
pub struct ActivePairingState {
    pub pin: String,
    pub is_initiator: bool,
    pub remote_id: String,
    pub remote_label: String,
    pub confirmed: Arc<Mutex<Option<bool>>>,
    pub handshake: Arc<Mutex<Option<HandshakeState>>>,
    pub silent: bool,
    pub is_reconnect: bool,
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
        let mut peers = self.peers.lock();
        peers.insert(node_id, (tx, transport));
    }

    #[tracing::instrument(skip(self))]
    pub fn remove_peer(&self, node_id: &str) {
        let mut peers = self.peers.lock();
        peers.remove(node_id);
    }

    #[tracing::instrument(skip(self))]
    pub fn broadcast(&self, msg: SyncMessage) {
        let peers = self.peers.lock();
        for (id, (tx, _)) in peers.iter() {
            if let Err(e) = tx.send(msg.clone()) {
                error!("Failed to send sync message to peer {}: {}", id, e);
            }
        }
    }

    #[tracing::instrument(skip(self))]
    pub fn send_to_peer(&self, node_id: &str, msg: SyncMessage) -> bool {
        let peers = self.peers.lock();
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
        let peers = self.peers.lock();
        peers.contains_key(node_id)
    }

    pub fn get_peer_transport(&self, node_id: &str) -> Option<TransportType> {
        let peers = self.peers.lock();
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
    pub libp2p_manager: Arc<Libp2pManager>,
    pending_turn_sessions: Mutex<HashMap<String, TurnConnection>>,
    active_oob_secret: Arc<Mutex<Option<String>>>,
    target_oob_secret: Arc<Mutex<Option<(String, String)>>>, // (node_id, secret)
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
        libp2p_manager: Arc<Libp2pManager>,
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
            libp2p_manager,
            pending_turn_sessions: Mutex::new(HashMap::new()),
            active_oob_secret: Arc::new(Mutex::new(None)),
            target_oob_secret: Arc::new(Mutex::new(None)),
        }
    }

    pub fn is_device_paired(&self, node_id: &str) -> bool {
        self.store.is_device_paired(node_id).unwrap_or(false)
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
            RelaySignal::PairingError { node_id, message } => {
                if message == "stale" {
                    warn!("Remote device {} reports stale pairing.", node_id);
                    let label = if let Ok(Some(device)) = self.store.get_paired_device(&node_id) {
                        device.label
                    } else {
                        "Unknown Device".to_string()
                    };
                    let _ = self.ipc_tx.send(IpcMessage::StalePairing {
                        node_id,
                        label,
                    });
                }
                let mut ap = self.active_pairing.lock();
                *ap = None;
                return;
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
        let mut ap = self.active_pairing.lock();

        // 1. If no active pairing, this might be a new incoming pairing request (Responder Message 1)
        if ap.is_none() {
            info!(
                "Received potential new pairing request from {} via relay",
                source_uuid
            );

            if payload.is_empty() {
                return Err(anyhow::anyhow!("Received empty Noise message from relay"));
            }

            // Protocol Prefix: 0x00 = XX, 0x01 = IK
            let pattern_byte = payload[0];
            let pattern = if pattern_byte == 0x01 {
                "Noise_IK_25519_ChaChaPoly_BLAKE2s"
            } else {
                "Noise_XX_25519_ChaChaPoly_BLAKE2s"
            };
            let use_ik = pattern.contains("_IK_");
            let noise_data = &payload[1..];

            let params: NoiseParams = pattern.parse().unwrap();
            let mut builder = Builder::new(params);
            builder = builder.local_private_key(&self.private_key);

            let mut remote_static = None;
            if use_ik {
                if let Ok(Some(device)) = self.store.get_paired_device(&source_uuid) {
                    remote_static = device.static_key;
                }
                
                if let Some(ref rs) = remote_static {
                    info!("Using known static key for IK responder via relay: {}", source_uuid);
                    builder = builder.remote_public_key(rs);
                } else {
                    warn!("Received IK handshake from unknown/unpaired device {} via relay. It will likely fail decryption.", source_uuid);
                }
            }

            let mut noise = builder.build_responder().unwrap();

            let mut initiator_payload_buf = [0u8; 1024];
            match noise.read_message(noise_data, &mut initiator_payload_buf) {
                Ok(payload_len) => {
                    info!("Decrypted Noise message 1 from relay using {}", if use_ik { "IK" } else { "XX" });
                    
                    let mut initiator_label = "Unknown Device".to_string();
                    let mut remote_node_id = source_uuid.clone();
                    let mut initiator_payload_opt: Option<HandshakePayload> = None;

                    if use_ik && payload_len > 0 {
                        let payload_slice = &initiator_payload_buf[..payload_len];
                        if let Ok(initiator_payload) = serde_json::from_slice::<HandshakePayload>(payload_slice) {
                            initiator_label = initiator_payload.label.clone();
                            remote_node_id = initiator_payload.node_id.clone();
                            initiator_payload_opt = Some(initiator_payload.clone());

                            // Peer Exchange: Inject libp2p addresses
                            if let Ok(peer_id) = remote_node_id.parse::<libp2p::PeerId>() {
                                for addr_str in initiator_payload.libp2p_addresses {
                                    if let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>() {
                                        self.libp2p_manager.inject_address(peer_id, addr);
                                    }
                                }
                            }
                        }
                    }

                    // Stale pairing check
                    if let Some(HandshakeIntent::Reconnect) = initiator_payload_opt.as_ref().and_then(|p| p.intent.clone()) {
                        if !self.is_device_paired(&remote_node_id) {
                            warn!("Stale pairing attempt from {}. Rejecting.", remote_node_id);
                            let err_sig = RelaySignal::PairingError {
                                node_id: self.node_id.clone(),
                                message: "stale".to_string(),
                            };
                            let _ = self.relay_manager.send_signal(source_uuid, serde_json::to_vec(&err_sig).unwrap());
                            return Ok(());
                        }
                    }

                    let self_label = self
                        .store
                        .get_state("device_name")
                        .unwrap()
                        .unwrap_or_else(|| "Unknown Device".to_string());
                    let self_payload = HandshakePayload {
                        label: self_label,
                        node_id: self.node_id.clone(),
                        libp2p_addresses: self.libp2p_manager.get_listen_addresses().into_iter().map(|a| a.to_string()).collect(),
                        oob_secret: self.active_oob_secret.lock().clone(),
                        intent: Some(if use_ik { HandshakeIntent::Reconnect } else { HandshakeIntent::Pair }),
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
                            let mut pin = String::new();
                            if noise.is_handshake_finished() {
                                let h = noise.get_handshake_hash();
                                pin = derive_pin(h);
                            }

                            let is_paired = self.store.is_device_paired(&source_uuid).unwrap_or(false);
                            let has_active_oob = self.active_oob_secret.lock().is_some();

                            *ap = Some(ActivePairingState {
                                pin,
                                is_initiator: false,
                                remote_id: remote_node_id.clone(),
                                remote_label: initiator_label.clone(),
                                confirmed: Arc::new(Mutex::new(None)),
                                handshake: Arc::new(Mutex::new(Some(noise))),
                                silent: is_paired || use_ik || has_active_oob,
                                is_reconnect: is_paired || use_ik,
                            });

                            if is_paired || use_ik {
                                info!("Auto-confirming relay pairing for known/IK device: {}", remote_node_id);
                                if let Some(ref state) = *ap {
                                    *state.confirmed.lock() = Some(true);
                                }
                            }

                            // Start monitoring for confirmation
                            self.monitor_relay_pairing(remote_node_id);
                        }
                        Err(e) => {
                            error!("Failed to write Noise message (step 2): {}", e);
                        }
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

            let mut handshake_lock = state.handshake.lock();
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

                                // Peer Exchange: Inject libp2p addresses
                                if let Ok(peer_id) = remote_node_id.parse::<libp2p::PeerId>() {
                                    for addr_str in payload.libp2p_addresses {
                                        if let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>() {
                                            self.libp2p_manager.inject_address(peer_id, addr);
                                        }
                                    }
                                }

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

                                // Stale pairing check
                                if let Some(HandshakeIntent::Reconnect) = payload.intent {
                                    if !self.is_device_paired(&remote_node_id) {
                                        warn!("Stale XX pairing attempt from {}. Rejecting.", remote_node_id);
                                        let err_sig = RelaySignal::PairingError {
                                            node_id: self.node_id.clone(),
                                            message: "stale".to_string(),
                                        };
                                        let _ = self.relay_manager.send_signal(source_uuid, serde_json::to_vec(&err_sig).unwrap());
                                        drop(handshake_lock);
                                        *ap = None;
                                        return Ok(());
                                    }
                                }

                                // Peer Exchange: Inject libp2p addresses
                                if let Ok(peer_id) = remote_node_id.parse::<libp2p::PeerId>() {
                                    for addr_str in payload.libp2p_addresses {
                                        if let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>() {
                                            self.libp2p_manager.inject_address(peer_id, addr);
                                        }
                                    }
                                }

                                info!(
                                    "Relay pairing with {} ({}) successful. PIN: {}",
                                    remote_label, remote_node_id, state.pin
                                );
                                state.remote_id = remote_node_id;
                                state.remote_label = remote_label;

                                // Check if OOB secret matches
                                let initiator_oob_secret = payload.oob_secret;
                                {
                                    let mut active = self.active_oob_secret.lock();
                                    if let Some(ref secret) = *active {
                                        if let Some(ref initiator_secret) = initiator_oob_secret {
                                            if secret == initiator_secret {
                                                info!("Relay OOB pairing: secret matched! Auto-confirming.");
                                                let mut conf = state.confirmed.lock();
                                                *conf = Some(true);
                                            } else {
                                                warn!("Relay OOB pairing: secret mismatch! initiator sent {}, expected {}", initiator_secret, secret);
                                            }
                                        }
                                    }
                                    // Clear active secret after connection attempt (one-time use)
                                    *active = None;
                                }

                                // Auto-confirm if already paired
                                if let Ok(true) = self.store.is_device_paired(&state.remote_id) {
                                    info!("Device {} is already paired. Auto-confirming relay pairing.", state.remote_id);
                                    let mut conf = state.confirmed.lock();
                                    *conf = Some(true);
                                }
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

                                // Peer Exchange: Inject libp2p addresses
                                if let Ok(peer_id) = remote_node_id.parse::<libp2p::PeerId>() {
                                    for addr_str in payload.libp2p_addresses {
                                        if let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>() {
                                            self.libp2p_manager.inject_address(peer_id, addr);
                                        }
                                    }
                                }

                                info!(
                                    "Received responder payload via relay: {} ({})",
                                    remote_label, remote_node_id
                                );
                                state.remote_id = remote_node_id.clone();
                                state.remote_label = remote_label;

                                let mut oob_secret = None;
                                {
                                    let target = self.target_oob_secret.lock();
                                    if let Some((ref node_id, ref secret)) = *target {
                                        if node_id == &state.remote_id {
                                            info!("Using OOB secret for relay pairing with {}", state.remote_id);
                                            oob_secret = Some(secret.clone());
                                        }
                                    }
                                }

                                let self_label = self
                                    .store
                                    .get_state("device_name")
                                    .unwrap()
                                    .unwrap_or_else(|| "Unknown Device".to_string());
                                let is_reconnect = self.is_device_paired(&state.remote_id);
                                let self_payload = HandshakePayload {
                                    label: self_label,
                                    node_id: self.node_id.clone(),
                                    libp2p_addresses: self.libp2p_manager.get_listen_addresses().into_iter().map(|a| a.to_string()).collect(),
                                    oob_secret,
                                    intent: Some(if is_reconnect { HandshakeIntent::Reconnect } else { HandshakeIntent::Pair }),
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

                                            // Auto-confirm if already paired
                                            if let Ok(true) = self.store.is_device_paired(&state.remote_id) {
                                                info!("Device {} is already paired. Auto-confirming relay pairing.", state.remote_id);
                                                let mut conf = state.confirmed.lock();
                                                *conf = Some(true);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to write Noise message (step 3): {}", e);
                                        let mut ap = self.active_pairing.lock();
                                        *ap = None;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to read Noise message during relay handshake: {}", e);
                        // Clear active pairing on error to prevent infinite loop in monitor_relay_pairing
                        let mut ap = self.active_pairing.lock();
                        *ap = None;
                    }
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

        let mut ap = self.active_pairing.lock();
        if let Some(ref mut state) = *ap {
            if state.remote_id == source_uuid {
                let confirmed = state.confirmed.lock();
                if let Some(true) = *confirmed {
                    // Start TURN session if we haven't already
                    if !self
                        .pending_turn_sessions
                        .lock()
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
                                    let mut hs = state.handshake.lock();
                                    if let Some(noise) = hs.take() {
                                        let remote_static = noise.get_remote_static().map(|s| s.to_vec());
                                        if let Ok(transport) = noise.into_transport_mode() {
                                            let remote_node_id = state.remote_id.clone();
                                            let remote_label = state.remote_label.clone();
                                            let sync_manager = Arc::clone(&self.sync_manager);
                                            let ipc_tx = self.ipc_tx.clone();
                                            let remote_uuid = source_uuid.clone();

                                            let store = Arc::clone(&self.store);
                                            let libp2p_manager = Arc::clone(&self.libp2p_manager);

                                            thread::spawn(move || {
                                                if let Err(e) = run_turn_sync_session(
                                                    conn,
                                                    transport,
                                                    remote_node_id,
                                                    remote_label,
                                                    sync_manager,
                                                    ipc_tx,
                                                    store,
                                                    libp2p_manager,
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
                                                remote_static.as_deref(),
                                            );
                                            let _ = self.ipc_tx.send(IpcMessage::PairingResult {
                                                success: true,
                                                node_id: source_uuid.clone(),
                                                label: state.remote_label.clone(),
                                                error: None,
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

        // Clear UI state if it matches this source
        {
            let mut ap = self.active_pairing.lock();
            if let Some(ref state) = *ap {
                if state.remote_id == source_uuid {
                    *ap = None;
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
                    let ap = active_pairing.lock();
                    if let Some(ref state) = *ap {
                        if state.remote_id == remote_uuid {
                            let res = state.confirmed.lock();
                            *res
                        } else {
                            warn!("Relay pairing monitor: remote_id mismatch (expected {}, got {}). Terminating.", remote_uuid, state.remote_id);
                            break;
                        }
                    } else {
                        info!("Relay pairing monitor: ActivePairingState cleared. Terminating loop for {}.", remote_uuid);
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
                                    if let Err(e) =
                                        relay_manager.send_signal(remote_uuid.clone(), sig_bytes) {
                                            error!("Failed to send TURN candidate via relay: {}", e);
                                        }

                                    // The session will be started in handle_turn_candidate when peer responds
                                }
                                Err(e) => error!(
                                    "Failed to start TURN session for {}: {}",
                                    remote_uuid, e
                                ),
                            }
                        }
                    } else {
                        info!("Relay pairing for {} was declined locally.", remote_uuid);
                    }
                    break;
                }
                thread::sleep(Duration::from_millis(200));
            }
        });
    }

    pub fn initiate_remote_pairing(&self, target_uuid: String) {
        if self.store.is_device_paired(&target_uuid).unwrap_or(false) {
            info!(
                "Device {} is already paired. Skipping new remote pairing initiation.",
                target_uuid
            );
            return;
        }

        info!("Initiating remote pairing with {} via relay", target_uuid);

        let mut remote_static = None;
        if let Ok(Some(device)) = self.store.get_paired_device(&target_uuid) {
            remote_static = device.static_key;
        }

        let use_ik = remote_static.is_some();
        let pattern = if use_ik {
            "Noise_IK_25519_ChaChaPoly_BLAKE2s"
        } else {
            "Noise_XX_25519_ChaChaPoly_BLAKE2s"
        };

        let params: NoiseParams = pattern.parse().unwrap();
        let mut builder = Builder::new(params);
        builder = builder.local_private_key(&self.private_key);
        if let Some(ref rs) = remote_static {
            builder = builder.remote_public_key(rs);
        }
        let mut noise = builder.build_initiator().unwrap();

        let initiator_payload = HandshakePayload {
            label: self
                .store
                .get_state("device_name")
                .unwrap()
                .unwrap_or_else(|| "Unknown Device".to_string()),
            node_id: self.node_id.clone(),
            libp2p_addresses: self
                .libp2p_manager
                .get_listen_addresses()
                .into_iter()
                .map(|a| a.to_string())
                .collect(),
            oob_secret: None,
            intent: Some(if use_ik {
                HandshakeIntent::Reconnect
            } else {
                HandshakeIntent::Pair
            }),
        };
        let initiator_payload_bytes = serde_json::to_vec(&initiator_payload).unwrap();

        let mut buf = [0u8; 1024];
        match noise.write_message(&initiator_payload_bytes, &mut buf) {
            Ok(n) => {
                info!(
                    "Sending handshake initiation (step 1) to {} via relay using {}",
                    target_uuid, if use_ik { "IK" } else { "XX" }
                );
                // Protocol Prefix: 0x00 = XX, 0x01 = IK
                let prefix = if use_ik { 0x01 } else { 0x00 };
                let mut prefixed_msg = vec![prefix];
                prefixed_msg.extend_from_slice(&buf[..n]);

                let sig = RelaySignal::Noise(prefixed_msg);
                let sig_bytes = serde_json::to_vec(&sig).unwrap();
                if let Err(e) = self
                    .relay_manager
                    .send_signal(target_uuid.clone(), sig_bytes)
                {
                    error!("Failed to send handshake initiation via relay: {}", e);
                    return;
                }

                let mut ap = self.active_pairing.lock();
                let has_oob = self.target_oob_secret.lock().as_ref()
                    .map(|(id, _)| id == &target_uuid).unwrap_or(false);

                *ap = Some(ActivePairingState {
                    pin: String::new(),
                    is_initiator: true,
                    remote_id: target_uuid.clone(),
                    remote_label: "Remote Device (Relay)".to_string(),
                    confirmed: Arc::new(Mutex::new(None)),
                    handshake: Arc::new(Mutex::new(Some(noise))),
                    silent: use_ik || has_oob,
                    is_reconnect: use_ik,
                });

                if use_ik || has_oob {
                    info!("Auto-confirming outgoing relay pairing for device: {}", target_uuid);
                    if let Some(ref state) = *ap {
                        *state.confirmed.lock() = Some(true);
                    }
                }

                self.monitor_relay_pairing(target_uuid);
            }
            Err(e) => error!("Failed to write Noise message (step 1): {}", e),
        }
    }

    pub fn reconnect_known_device(&self, target_uuid: String) {
        if !self.store.is_device_paired(&target_uuid).unwrap_or(false) {
            warn!(
                "reconnect_known_device called for unpaired device {}",
                target_uuid
            );
            return;
        }
        info!("Reconnecting to known device {} via relay", target_uuid);
        self.initiate_remote_pairing_silent(target_uuid);
    }

    fn initiate_remote_pairing_silent(&self, target_uuid: String) {
        let mut remote_static = None;
        if let Ok(Some(device)) = self.store.get_paired_device(&target_uuid) {
            remote_static = device.static_key;
        }

        let use_ik = remote_static.is_some();
        let pattern = if use_ik {
            "Noise_IK_25519_ChaChaPoly_BLAKE2s"
        } else {
            "Noise_XX_25519_ChaChaPoly_BLAKE2s"
        };

        let params: NoiseParams = pattern.parse().unwrap();
        let mut builder = Builder::new(params);
        builder = builder.local_private_key(&self.private_key);
        if let Some(ref rs) = remote_static {
            builder = builder.remote_public_key(rs);
        }
        let mut noise = builder.build_initiator().unwrap();

        let initiator_payload = HandshakePayload {
            label: self
                .store
                .get_state("device_name")
                .unwrap()
                .unwrap_or_else(|| "Unknown Device".to_string()),
            node_id: self.node_id.clone(),
            libp2p_addresses: self
                .libp2p_manager
                .get_listen_addresses()
                .into_iter()
                .map(|a| a.to_string())
                .collect(),
            oob_secret: None,
            intent: Some(HandshakeIntent::Reconnect),
        };
        let initiator_payload_bytes = serde_json::to_vec(&initiator_payload).unwrap();

        let mut buf = [0u8; 1024];
        match noise.write_message(&initiator_payload_bytes, &mut buf) {
            Ok(n) => {
                info!(
                    "Sending silent handshake initiation (step 1) to {} via relay using {}",
                    target_uuid,
                    if use_ik { "IK" } else { "XX" }
                );
                let prefix = if use_ik { 0x01 } else { 0x00 };
                let mut prefixed_msg = vec![prefix];
                prefixed_msg.extend_from_slice(&buf[..n]);

                let sig = RelaySignal::Noise(prefixed_msg);
                let sig_bytes = serde_json::to_vec(&sig).unwrap();
                if let Err(e) = self
                    .relay_manager
                    .send_signal(target_uuid.clone(), sig_bytes)
                {
                    error!("Failed to send silent handshake initiation via relay: {}", e);
                    return;
                }

                let mut ap = self.active_pairing.lock();
                *ap = Some(ActivePairingState {
                    pin: String::new(),
                    is_initiator: true,
                    remote_id: target_uuid.clone(),
                    remote_label: "Known Device".to_string(),
                    confirmed: Arc::new(Mutex::new(Some(true))), // Auto-confirmed
                    handshake: Arc::new(Mutex::new(Some(noise))),
                    silent: true,
                    is_reconnect: true,
                });

                self.monitor_relay_pairing(target_uuid);
            }
            Err(e) => error!("Failed to write silent Noise message (step 1): {}", e),
        }
    }

    pub fn start_auto_reconnect_loop(self: Arc<Self>) {
        info!("Auto-reconnect loop started");
        let mut last_attempts: std::collections::HashMap<String, std::time::Instant> = std::collections::HashMap::new();
        
        loop {
            std::thread::sleep(std::time::Duration::from_secs(15));
            
            let paired_devices = match self.store.get_paired_devices() {
                Ok(devices) => devices,
                Err(e) => {
                    error!("Auto-reconnect loop: failed to retrieve paired devices: {}", e);
                    continue;
                }
            };
            
            for device in paired_devices {
                let is_connected = self.sync_manager.is_connected(&device.node_id);
                if !is_connected {
                    let should_retry = match last_attempts.get(&device.node_id) {
                        Some(last_attempt) => last_attempt.elapsed() >= std::time::Duration::from_secs(30),
                        None => true,
                    };
                    
                    if should_retry {
                        last_attempts.insert(device.node_id.clone(), std::time::Instant::now());
                        info!("Auto-reconnect: initiating retry for offline paired device {}", device.node_id);
                        
                        let pm = Arc::clone(&self);
                        let target_uuid = device.node_id.clone();
                        let ips = device.last_known_ips.clone();
                        let port = device.last_known_port;
                        
                        std::thread::spawn(move || {
                            // Verify the device is still paired before starting connection
                            if !pm.store.is_device_paired(&target_uuid).unwrap_or(false) {
                                debug!("Auto-reconnect: device {} was unpaired, aborting reconnect", target_uuid);
                                return;
                            }
                            
                            let mut success = false;
                            
                            if let (Some(ip_list), Some(p)) = (ips, port) {
                                for ip in ip_list {
                                    if let Ok(ip_addr) = ip.parse() {
                                        let addr = std::net::SocketAddr::new(ip_addr, p);
                                        // Re-check before each connection attempt
                                        if !pm.store.is_device_paired(&target_uuid).unwrap_or(false) {
                                            return;
                                        }
                                        debug!("Auto-reconnect: trying LAN connection for {} at {}", target_uuid, addr);
                                        if pm.initiate_pairing(addr, Some(target_uuid.clone())) {
                                            info!("Auto-reconnect: LAN connection to {} succeeded", target_uuid);
                                            success = true;
                                            break;
                                        }
                                    }
                                }
                            }
                            
                            if !success {
                                if pm.store.is_device_paired(&target_uuid).unwrap_or(false) {
                                    debug!("Auto-reconnect: LAN connection failed or unavailable for {}, falling back to remote relay...", target_uuid);
                                    pm.reconnect_known_device(target_uuid);
                                }
                            }
                        });
                    }
                } else {
                    last_attempts.remove(&device.node_id);
                }
            }
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
                    let node_id = self.node_id.clone();
                    let sync_manager = Arc::clone(&self.sync_manager);
                    let libp2p_manager = Arc::clone(&self.libp2p_manager);
                    let active_oob_secret = Arc::clone(&self.active_oob_secret);
                    let target_oob_secret = Arc::clone(&self.target_oob_secret);

                    thread::spawn(move || {
                        if let Err(e) = handle_incoming_connection(
                            stream,
                            store,
                            ipc_tx,
                            priv_key,
                            active_pairing,
                            node_id,
                            sync_manager,
                            libp2p_manager,
                            active_oob_secret,
                            target_oob_secret,
                        ) {


                            error!("Error in incoming connection: {}", e);
                        }
                    });
                }
                Err(e) => error!("Failed to accept connection: {}", e),
            }
        }
    }

    pub fn generate_qr_payload(&self) -> Result<String> {
        let secret: String = (0..16)
            .map(|_| format!("{:02x}", rand::random::<u8>()))
            .collect();
        *self.active_oob_secret.lock() = Some(secret.clone());

        let label = self
            .store
            .get_state("device_name")?
            .unwrap_or_else(|| "Unknown".to_string());

        let mut ips = Vec::new();
        for addr in self.libp2p_manager.get_listen_addresses() {
            let addr_str = addr.to_string();
            // Extract IP from multiaddr e.g. /ip4/192.168.1.5/tcp/12345
            if let Some(ip) = addr_str.split('/').nth(2) {
                if !ips.contains(&ip.to_string()) && ip != "127.0.0.1" && !ip.starts_with("172.17.") {
                    ips.push(ip.to_string());
                }
            }
        }

        let payload = format!(
            "cdus://pair?v=1&id={}&s={}&l={}&p={}&a={}",
            self.node_id,
            secret,
            urlencoding::encode(&label),
            self.port,
            urlencoding::encode(&ips.join(","))
        );
        Ok(payload)
    }

    pub fn set_target_oob_secret(&self, node_id: String, secret: String) {
        *self.target_oob_secret.lock() = Some((node_id, secret));
    }

    pub fn parse_qr_payload(&self, payload: &str) -> Result<(String, String, String, u16, Vec<String>)> {
        let url = url::Url::parse(payload).map_err(|_| anyhow::anyhow!("Invalid QR format"))?;
        if url.scheme() != "cdus" {
            return Err(anyhow::anyhow!("Not a CDUS pairing QR"));
        }
        
        let is_pair = url.path() == "/pair" || url.host_str() == Some("pair") || url.path() == "pair";
        if !is_pair {
            return Err(anyhow::anyhow!("Invalid CDUS QR path"));
        }

        let mut node_id = String::new();
        let mut secret = String::new();
        let mut label = String::new();
        let mut port = 5200;
        let mut ips = Vec::new();

        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "id" => node_id = value.into_owned(),
                "s" => secret = value.into_owned(),
                "l" => label = value.into_owned(),
                "p" => port = value.parse().unwrap_or(5200),
                "a" => ips = value.split(',').map(|s| s.to_string()).filter(|s| !s.is_empty()).collect(),
                _ => {}
            }
        }

        if node_id.is_empty() || secret.is_empty() {
            return Err(anyhow::anyhow!("Missing required fields in QR"));
        }

        Ok((node_id, secret, label, port, ips))
    }

    pub fn pair_with_qr(&self, payload: String) -> Result<()> {
        info!("Processing scanned QR payload");
        let (node_id, secret, label, _port, _ips) = self.parse_qr_payload(&payload)?;

        if self.store.is_device_paired(&node_id).unwrap_or(false) {
            info!(
                "QR scan for {} ({}) — already paired, nothing to do.",
                label, node_id
            );
            let _ = self.ipc_tx.send(IpcMessage::AlreadyPaired {
                node_id,
                label,
            });
            return Ok(());
        }

        info!("Scanned QR for new device: {} ({})", label, node_id);
        *self.target_oob_secret.lock() = Some((node_id.clone(), secret));

        // Start pairing attempt
        self.initiate_remote_pairing(node_id);
        
        Ok(())
    }

    pub fn initiate_pairing(&self, target_addr: SocketAddr, target_node_id: Option<String>) -> bool {
        let stream = match TcpStream::connect_timeout(&target_addr, Duration::from_secs(5)) {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to connect to target {}: {}", target_addr, e);
                return false;
            }
        };

        let store = Arc::clone(&self.store);
        let ipc_tx = self.ipc_tx.clone();
        let priv_key = self.private_key.clone();
        let active_pairing = Arc::clone(&self.active_pairing);
        let self_node_id = self.node_id.clone();
        let sync_manager = Arc::clone(&self.sync_manager);
        let libp2p_manager = Arc::clone(&self.libp2p_manager);
        let active_oob_secret = Arc::clone(&self.active_oob_secret);
        let target_oob_secret = Arc::clone(&self.target_oob_secret);

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
                        libp2p_manager,
                        active_oob_secret,
                        target_oob_secret,
                        target_node_id,
                    ) {
                        error!("Error in outgoing connection: {}", e);
                    }
                }
                Err(e) => error!("WebSocket client handshake failed: {}", e),
            }
        });
        true
    }
}

fn run_turn_sync_session(
    conn: TurnConnection,
    mut transport: TransportState,
    node_id: String,
    label: String,
    sync_manager: Arc<SyncManager>,
    ipc_tx: Sender<IpcMessage>,
    store: Arc<Store>,
    libp2p_manager: Arc<Libp2pManager>,
) -> Result<()> {
    let (tx, rx) = flume::unbounded::<SyncMessage>();
    sync_manager.add_peer(node_id.clone(), tx.clone(), TransportType::Relay);

    info!("TURN Sync session started for {} ({})", label, node_id);

    // Initial PEX: Send our known peers to this new peer via TURN
    let mut pex_records = Vec::new();
    pex_records.push(cdus_common::PeerExchangeRecord {
        node_id: libp2p_manager.get_peer_id().to_string(),
        addresses: libp2p_manager
            .get_listen_addresses()
            .into_iter()
            .map(|a| a.to_string())
            .collect(),
    });

    if let Ok(paired) = store.get_paired_devices() {
        for device in paired {
            if device.node_id != node_id {
                let mut addrs = Vec::new();
                if let Some(ips) = device.last_known_ips {
                    if let Some(port) = device.last_known_port {
                        for ip in ips {
                            addrs.push(format!("/ip4/{}/tcp/{}", ip, port));
                        }
                    }
                }
                if !addrs.is_empty() {
                    pex_records.push(cdus_common::PeerExchangeRecord {
                        node_id: device.node_id,
                        addresses: addrs,
                    });
                }
            }
        }
    }

    if !pex_records.is_empty() {
        info!("Sending PEX update ({} peers) to {} via TURN", pex_records.len(), label);
        let pex_msg = SyncMessage::PeerExchange { peers: pex_records };
        if let Ok(data) = pex_msg.to_vec() {
            let mut out = vec![0u8; data.len() + 100];
            if let Ok(n) = transport.write_message(&data, &mut out) {
                let _ = conn.tx.send(out[..n].to_vec());
            }
        }
    }

    // Initial Clipboard Sync: Send our latest clipboard update to the peer via TURN
    if let Ok(Some(ts_str)) = store.get_state("last_sync_timestamp") {
        if let Ok(ts) = ts_str.parse::<u64>() {
            if let Ok(Some(content)) = store.get_state("last_clipboard_content") {
                info!("Sending initial clipboard update on connection to {} via TURN: timestamp={}", label, ts);
                let sync_msg = SyncMessage::ClipboardUpdate { content, timestamp: ts };
                if let Ok(data) = sync_msg.to_vec() {
                    let mut out = vec![0u8; data.len() + 100];
                    if let Ok(n) = transport.write_message(&data, &mut out) {
                        let _ = conn.tx.send(out[..n].to_vec());
                    }
                }
            }
        }
    }

    loop {
        // 1. Check for incoming messages from peer via TURN
        if let Ok(data) = conn.rx.try_recv() {
            let mut out = vec![0u8; data.len()];
            match transport.read_message(&data, &mut out) {
                Ok(n) => {
                    out.truncate(n);
                    if let Ok(msg) = SyncMessage::from_slice(&out) {
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
                            SyncMessage::NotificationMirror(payload) => {
                                let _ = ipc_tx.send(IpcMessage::NotificationMirrored(payload));
                            }
                            SyncMessage::NotificationDismiss { key } => {
                                let _ = ipc_tx.send(IpcMessage::NotificationDismissed { key });
                            }
                            SyncMessage::PeerExchange { peers } => {
                                info!(
                                    "Received PEX via TURN from {} ({} peers)",
                                    label,
                                    peers.len()
                                );
                                for peer in peers {
                                    if let Ok(peer_id) = peer.node_id.parse::<libp2p::PeerId>() {
                                        for addr_str in peer.addresses {
                                            if let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>() {
                                                libp2p_manager.inject_address(peer_id, addr);
                                            }
                                        }
                                    }
                                }
                            }
                            SyncMessage::Disconnect => {
                                info!("Received Disconnect request from peer {} via TURN, closing session", label);
                                break;
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
        match rx.try_recv() {
            Ok(msg) => {
                let is_disconnect = msg == SyncMessage::Disconnect;
                let data = msg.to_vec()?;
                let mut buf = vec![0u8; data.len() + 1024];
                match transport.write_message(&data, &mut buf) {
                    Ok(n) => {
                        if let Err(e) = conn.tx.send(buf[..n].to_vec()) {
                            error!("Failed to send to TURN thread: {}", e);
                            break;
                        }
                        if is_disconnect {
                            info!("Sent Disconnect to peer {} via TURN, closing session", label);
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Noise encryption failed: {}", e);
                        break;
                    }
                }
            }
            Err(flume::TryRecvError::Disconnected) => {
                info!("Outgoing channel disconnected, closing TURN sync session for {}", label);
                break;
            }
            Err(flume::TryRecvError::Empty) => {}
        }

        thread::sleep(Duration::from_millis(100));
    }

    sync_manager.remove_peer(&node_id);
    let _ = ipc_tx.send(IpcMessage::PeerDisconnected { node_id });
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
    libp2p_manager: Arc<Libp2pManager>,
    active_oob_secret: Arc<Mutex<Option<String>>>,
    target_oob_secret: Arc<Mutex<Option<(String, String)>>>,
) -> Result<()> {
    let res = handle_incoming_connection_inner(
        stream,
        Arc::clone(&store),
        ipc_tx.clone(),
        priv_key,
        Arc::clone(&active_pairing),
        self_node_id,
        Arc::clone(&sync_manager),
        libp2p_manager,
        active_oob_secret,
        target_oob_secret,
    );

    if let Err(e) = res {
        error!("Error in incoming connection: {}", e);
        // Ensure UI is notified of failure if it was an active pairing attempt
        let _ = ipc_tx.send(IpcMessage::PairingResult {
            success: false,
            node_id: "unknown".to_string(),
            label: "unknown".to_string(),
            error: Some(e.to_string()),
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
    libp2p_manager: Arc<Libp2pManager>,
    active_oob_secret: Arc<Mutex<Option<String>>>,
    _target_oob_secret: Arc<Mutex<Option<(String, String)>>>,
) -> Result<()> {
    info!("Upgrading incoming connection to WebSocket");
    let mut ws = accept(stream)?;

    info!("Handling incoming Noise connection over WebSocket");

    let self_label = store
        .get_state("device_name")?
        .unwrap_or_else(|| "Unknown Device".to_string());

    let mut buf = [0u8; 2048];

    // 1. Read Prefix + Message 1
    let msg = ws.read()?;
    let (pattern, noise_data, remote_id_hint) = if let Message::Binary(ref data) = msg {
        if data.is_empty() {
            return Err(anyhow::anyhow!("Received empty Noise message"));
        }
        let pattern_byte = data[0];
        if pattern_byte == 0x01 {
            // IK Pattern: [0x01, node_id_len (1), node_id (n), noise_msg]
            if data.len() < 2 {
                return Err(anyhow::anyhow!("IK message too short for length byte"));
            }
            let id_len = data[1] as usize;
            if data.len() < 2 + id_len {
                return Err(anyhow::anyhow!("IK message truncated ID"));
            }
            let node_id = String::from_utf8_lossy(&data[2..2 + id_len]).to_string();
            ("Noise_IK_25519_ChaChaPoly_BLAKE2s", &data[2 + id_len..], Some(node_id))
        } else {
            // XX Pattern: [0x00, noise_msg]
            ("Noise_XX_25519_ChaChaPoly_BLAKE2s", &data[1..], None)
        }
    } else {
        return Err(anyhow::anyhow!("Expected binary Noise message"));
    };

    let use_ik = pattern.contains("_IK_");

    let params: NoiseParams = pattern
        .parse()
        .map_err(|e: snow::Error| anyhow::anyhow!(e))?;
    let mut builder = Builder::new(params);
    builder = builder.local_private_key(&priv_key);

    let mut remote_static = None;
    if use_ik {
        if let Some(ref node_id) = remote_id_hint {
            if let Ok(Some(device)) = store.get_paired_device(node_id) {
                remote_static = device.static_key;
            }
            
            if let Some(ref rs) = remote_static {
                info!("Using known static key for IK responder (direct): {}", node_id);
                builder = builder.remote_public_key(rs);
            } else {
                warn!("Received IK handshake from unknown device hint: {}", node_id);
            }
        }
    }

    let mut noise = builder.build_responder().map_err(|e| anyhow::anyhow!(e))?;

    // Read Message 1
    let mut initiator_payload_buf = [0u8; 2048];
    let payload_len = noise
        .read_message(noise_data, &mut initiator_payload_buf)
        .map_err(|e| anyhow::anyhow!(e))?;

    // 2. Write Message 2
    // For XX: <- e, ee, s, es + payload
    // For IK: <- e, ee, se
    let self_payload = HandshakePayload {
        label: self_label.clone(),
        node_id: self_node_id.clone(),
        libp2p_addresses: libp2p_manager.get_listen_addresses().into_iter().map(|a| a.to_string()).collect(),
        oob_secret: active_oob_secret.lock().clone(),
        intent: None, // Responder doesn't need to send intent
    };
    let self_payload_bytes = serde_json::to_vec(&self_payload).map_err(|e| anyhow::anyhow!(e))?;
    
    // In IK, the responder can send a payload in Message 2
    let n = noise
        .write_message(&self_payload_bytes, &mut buf)
        .map_err(|e| anyhow::anyhow!(e))?;
    ws.send(Message::Binary(buf[..n].to_vec()))?;

    let mut initiator_label = "Unknown Device".to_string();
    let mut remote_node_id = String::new();
    let mut initiator_oob_secret = None;
    let mut initiator_intent = None;

    if !use_ik {
        // 3. Read Message 3 (Only for XX)
        // For XX: -> s, se + payload
        let mut msg3_payload_buf = [0u8; 2048];
        let msg = ws.read()?;
        if let Message::Binary(data) = msg {
            let msg3_len = noise
                .read_message(&data, &mut msg3_payload_buf)
                .map_err(|e| {
                    error!("Noise decryption failed for initiator payload: {}", e);
                    anyhow::anyhow!(e)
                })?;

            if msg3_len == 0 {
                return Err(anyhow::anyhow!("Initiator sent an empty handshake payload"));
            }

            let payload_slice = &msg3_payload_buf[..msg3_len];
            let initiator_payload: HandshakePayload = serde_json::from_slice(payload_slice)
                .map_err(|e| {
                    let raw = String::from_utf8_lossy(payload_slice);
                    error!("Failed to parse initiator handshake payload. Len: {}. Raw data: '{}'. Error: {}", msg3_len, raw, e);
                    anyhow::anyhow!("Invalid handshake payload format")
                })?;

            initiator_label = initiator_payload.label;
            remote_node_id = initiator_payload.node_id;
            initiator_oob_secret = initiator_payload.oob_secret;
            initiator_intent = initiator_payload.intent;

            // Peer Exchange: Inject libp2p addresses
            if let Ok(peer_id) = remote_node_id.parse::<libp2p::PeerId>() {
                for addr_str in initiator_payload.libp2p_addresses {
                    if let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>() {
                        libp2p_manager.inject_address(peer_id, addr);
                    }
                }
            }
        } else {
            return Err(anyhow::anyhow!("Expected binary Noise message (step 3)"));
        }
    } else {
        // For IK, the initiator can send a payload in Message 1
        if payload_len > 0 {
            let payload_slice = &initiator_payload_buf[..payload_len];
            let initiator_payload: HandshakePayload = serde_json::from_slice(payload_slice)
                .map_err(|e| {
                    error!("Failed to parse IK initiator handshake payload: {}", e);
                    anyhow::anyhow!("Invalid IK handshake payload")
                })?;
            initiator_label = initiator_payload.label;
            remote_node_id = initiator_payload.node_id;
            initiator_oob_secret = initiator_payload.oob_secret;
            initiator_intent = initiator_payload.intent;

            // Peer Exchange: Inject libp2p addresses
            if let Ok(peer_id) = remote_node_id.parse::<libp2p::PeerId>() {
                for addr_str in initiator_payload.libp2p_addresses {
                    if let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>() {
                        libp2p_manager.inject_address(peer_id, addr);
                    }
                }
            }
        } else if let Some(rs) = noise.get_remote_static() {
            // Fallback to DB lookup if payload is empty
            if let Ok(Some(node_id)) = store.get_node_id_by_static_key(rs) {
                remote_node_id = node_id;
                if let Ok(Some(device)) = store.get_paired_device(&remote_node_id) {
                    initiator_label = device.label;
                }
            }
        }
        
        if remote_node_id.is_empty() {
             warn!("IK connection from unknown static key and no payload. Aborting.");
             return Err(anyhow::anyhow!("Unknown IK initiator"));
        }
    }

    // Verify Node ID
    if remote_node_id.is_empty() || remote_node_id.parse::<libp2p::PeerId>().is_err() {
        error!("Remote node provided an invalid or missing Peer ID: {}", remote_node_id);
        return Err(anyhow::anyhow!("Invalid Peer ID format"));
    }

    // Handshake finished.
    let h = noise.get_handshake_hash();
    let pin = derive_pin(h);
    let remote_static = noise.get_remote_static().map(|s| s.to_vec());

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

    let is_paired = store.is_device_paired(&remote_node_id)?;

    // Stale pairing check
    if let Some(HandshakeIntent::Reconnect) = initiator_intent {
        if !is_paired {
            warn!("Stale LAN pairing attempt from {}. Rejecting.", remote_node_id);
            // Send error code 0xFF framed
            write_ws_framed(&mut ws, &mut transport, &[0xFF])?;
            return Err(anyhow::anyhow!("Stale pairing"));
        }
    }

    if is_paired {
        info!(
            "Trusted device {} ({}) connected via {}. Auto-confirming sync session.",
            initiator_label, remote_node_id, if use_ik { "IK" } else { "XX" }
        );
        // Even if already paired, update the static key if we just got a new one
        let _ = store.add_paired_device(&remote_node_id, &initiator_label, remote_static.as_deref());
    } else {
            info!(
                "New device {} ({}) requesting pairing. PIN: {}",
                initiator_label, remote_node_id, pin
            );

            // Check if OOB secret matches (but still require manual confirmation)
            let mut _oob_matched = false;
            {
                let mut active = active_oob_secret.lock();
                if let Some(ref secret) = *active {
                    if let Some(ref initiator_secret) = initiator_oob_secret {
                        if secret == initiator_secret {
                            info!("OOB pairing: secret matched!");
                            _oob_matched = true;
                        } else {
                            warn!("OOB pairing: secret mismatch! initiator sent {}, expected {}", initiator_secret, secret);
                            // We could reject here, but letting it proceed to PIN check is also safe
                            // as long as we don't auto-confirm.
                        }
                    }
                }
                // Clear active secret after connection attempt (one-time use)
                *active = None;
            }

            // Update state for UI
            let confirmed = Arc::new(Mutex::new(None));
            {
                let mut ap = active_pairing.lock();
                *ap = Some(ActivePairingState {
                    pin: pin.clone(),
                    is_initiator: false,
                    remote_id: remote_node_id.clone(),
                    remote_label: initiator_label.clone(),
                    confirmed: Arc::clone(&confirmed),
                    handshake: Arc::new(Mutex::new(None)),
                    silent: false, // Always show modal for first-time pairing
                    is_reconnect: false,
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
                    let res = confirmed.lock();
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
                let mut ap = active_pairing.lock();
                *ap = None;
            }

            if !local_confirmed || !remote_confirmed {
                return Err(anyhow::anyhow!("Pairing failed or rejected"));
            }

            info!("Both sides confirmed. Pairing successful.");
            let _ = store.add_paired_device(&remote_node_id, &initiator_label, remote_static.as_deref());
            let _ = ipc_tx.send(IpcMessage::PairingResult {
                success: true,
                node_id: remote_node_id.clone(),
                label: initiator_label.clone(),
                error: None,
            });
        }

        // If we reach here, we are either trusted (skipped loop) or pairing was confirmed (loop finished)
        // Wait! We STILL need to send/receive the confirmation bytes even if trusted, 
        // to keep the protocol consistent!
        if is_paired {
             // Send our confirmation
             write_ws_framed(&mut ws, &mut transport, &[1])?;
             // Wait for remote confirmation
             let _ = ws.get_ref().set_read_timeout(Some(Duration::from_secs(2)));
             match read_ws_framed(&mut ws, &mut transport) {
                 Ok(data) if !data.is_empty() && data[0] == 1 => {
                     info!("Remote side also trusted us. Session established.");
                 }
                 _ => return Err(anyhow::anyhow!("Remote side did not confirm trusted session")),
             }
        }

        run_sync_session(
            ws,
            transport,
            remote_node_id.clone(),
            initiator_label.clone(),
            sync_manager,
            ipc_tx,
            store,
            libp2p_manager,
        )?;

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
    libp2p_manager: Arc<Libp2pManager>,
    active_oob_secret: Arc<Mutex<Option<String>>>,
    target_oob_secret: Arc<Mutex<Option<(String, String)>>>,
    target_node_id: Option<String>,
) -> Result<()> {
    let res = handle_outgoing_connection_inner(
        ws,
        Arc::clone(&store),
        ipc_tx.clone(),
        priv_key,
        Arc::clone(&active_pairing),
        self_node_id,
        Arc::clone(&sync_manager),
        libp2p_manager,
        active_oob_secret,
        target_oob_secret,
        target_node_id,
    );

    if let Err(e) = res {
        error!("Error in outgoing connection: {}", e);
        let _ = ipc_tx.send(IpcMessage::PairingResult {
            success: false,
            node_id: "unknown".to_string(),
            label: "unknown".to_string(),
            error: Some(e.to_string()),
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
    libp2p_manager: Arc<Libp2pManager>,
    _active_oob_secret: Arc<Mutex<Option<String>>>,
    target_oob_secret: Arc<Mutex<Option<(String, String)>>>,
    target_node_id: Option<String>,
) -> Result<()> {
    info!("Initiating outgoing Noise connection over WebSocket");

    let self_label = store
        .get_state("device_name")?
        .unwrap_or_else(|| "Unknown Device".to_string());

    let mut remote_static = None;
    if let Some(ref node_id) = target_node_id {
        if let Ok(Some(device)) = store.get_paired_device(node_id) {
            remote_static = device.static_key;
        }
    }

    let use_ik = remote_static.is_some();
    let pattern = if use_ik {
        "Noise_IK_25519_ChaChaPoly_BLAKE2s"
    } else {
        "Noise_XX_25519_ChaChaPoly_BLAKE2s"
    };

    let params: NoiseParams = pattern
        .parse()
        .map_err(|e: snow::Error| anyhow::anyhow!(e))?;
    let mut builder = Builder::new(params);
    builder = builder.local_private_key(&priv_key);
    if let Some(ref rs) = remote_static {
        builder = builder.remote_public_key(rs);
    }
    let mut noise = builder.build_initiator().map_err(|e| anyhow::anyhow!(e))?;

    let mut buf = [0u8; 2048];

    // Protocol Prefix: 0x00 = XX, 0x01 = IK
    let prefix = if use_ik { 0x01 } else { 0x00 };

    // 1. Write Message 1
    // For XX: -> e
    // For IK: -> e, es, ss
    let is_reconnect = if let Some(ref tid) = target_node_id {
        store.is_device_paired(tid).unwrap_or(false)
    } else {
        false
    };

    let initiator_payload = if use_ik {
        Some(HandshakePayload {
            label: self_label.clone(),
            node_id: self_node_id.clone(),
            libp2p_addresses: libp2p_manager.get_listen_addresses().into_iter().map(|a| a.to_string()).collect(),
            oob_secret: None,
            intent: Some(HandshakeIntent::Reconnect),
        })
    } else {
        None
    };

    let initiator_payload_bytes = if let Some(p) = initiator_payload {
        serde_json::to_vec(&p).map_err(|e| anyhow::anyhow!(e))?
    } else {
        Vec::new()
    };

    let n = noise
        .write_message(&initiator_payload_bytes, &mut buf)
        .map_err(|e| anyhow::anyhow!(e))?;
    
    let mut prefixed_msg = vec![prefix];
    if use_ik {
        let node_id_bytes = self_node_id.as_bytes();
        prefixed_msg.push(node_id_bytes.len() as u8);
        prefixed_msg.extend_from_slice(node_id_bytes);
    }
    prefixed_msg.extend_from_slice(&buf[..n]);
    ws.send(Message::Binary(prefixed_msg))?;

    let mut responder_payload_buf = [0u8; 2048];
    let msg = ws.read()?;
    if let Message::Binary(data) = msg {
        let payload_len = noise
            .read_message(&data, &mut responder_payload_buf)
            .map_err(|e| {
                error!("Noise decryption failed for responder payload: {}", e);
                anyhow::anyhow!(e)
            })?;

        if payload_len == 0 && !use_ik {
            return Err(anyhow::anyhow!("Responder sent an empty handshake payload"));
        }

        let mut responder_label = "Unknown Device".to_string();
        let mut remote_node_id = target_node_id.unwrap_or_default();

        if payload_len > 0 {
            let payload_slice = &responder_payload_buf[..payload_len];
            let responder_payload: HandshakePayload = serde_json::from_slice(payload_slice)
                .map_err(|e| {
                    let raw = String::from_utf8_lossy(payload_slice);
                    error!("Failed to parse responder handshake payload. Len: {}. Raw data: '{}'. Error: {}", payload_len, raw, e);
                    anyhow::anyhow!("Invalid handshake payload format from responder")
                })?;

            responder_label = responder_payload.label;
            remote_node_id = responder_payload.node_id;

            // Peer Exchange: Inject libp2p addresses
            if let Ok(peer_id) = remote_node_id.parse::<libp2p::PeerId>() {
                for addr_str in responder_payload.libp2p_addresses {
                    if let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>() {
                        libp2p_manager.inject_address(peer_id, addr);
                    }
                }
            }

            // Verify Node ID is a valid PeerId
            if let Err(e) = remote_node_id.parse::<libp2p::PeerId>() {
                error!(
                    "Remote responder provided an invalid Peer ID: {}. Error: {}",
                    remote_node_id, e
                );
                return Err(anyhow::anyhow!("Invalid Peer ID format from responder"));
            }
        }

        let mut oob_secret = None;
        {
            let mut target = target_oob_secret.lock();
            if let Some((ref node_id, ref secret)) = *target {
                if node_id == &remote_node_id {
                    info!("Using OOB secret for pairing with {}", remote_node_id);
                    oob_secret = Some(secret.clone());
                }
            }
            // Clear it after use or if mismatch
            *target = None;
        }

        // 3. Write Message 3 (Only for XX)
        // For XX: -> s, se + payload
        // For IK: (Already finished)
        if !use_ik {
            let self_payload = HandshakePayload {
                label: self_label.clone(),
                node_id: self_node_id.clone(),
                libp2p_addresses: libp2p_manager.get_listen_addresses().into_iter().map(|a| a.to_string()).collect(),
                oob_secret: oob_secret.clone(),
                intent: Some(if is_reconnect { HandshakeIntent::Reconnect } else { HandshakeIntent::Pair }),
            };
            let self_payload_bytes =
                serde_json::to_vec(&self_payload).map_err(|e| anyhow::anyhow!(e))?;
            let n = noise
                .write_message(&self_payload_bytes, &mut buf)
                .map_err(|e| anyhow::anyhow!(e))?;
            ws.send(Message::Binary(buf[..n].to_vec()))?;
        }

        // Handshake finished.
        let h = noise.get_handshake_hash();
        let pin = derive_pin(h);
        let remote_static = noise.get_remote_static().map(|s| s.to_vec());

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

        let is_paired = store.is_device_paired(&remote_node_id)?;

        if is_paired {
            info!(
                "Connecting to trusted device {} ({}) for sync.",
                responder_label, remote_node_id
            );
            // Even if already paired, update the static key if we just got a new one
            let _ = store.add_paired_device(&remote_node_id, &responder_label, remote_static.as_deref());
        } else {
            info!(
                "Initiating pairing with {} ({}). PIN: {}",
                responder_label, remote_node_id, pin
            );

            // Even for OOB pairing, we now require manual PIN confirmation to avoid ghost pairing.
            let mut local_confirmed = false;
            let mut remote_confirmed = false;

            // Update state for UI
            let confirmed = Arc::new(Mutex::new(None));
            {
                let mut ap = active_pairing.lock();
                *ap = Some(ActivePairingState {
                    pin: pin.clone(),
                    is_initiator: true,
                    remote_id: remote_node_id.clone(),
                    remote_label: responder_label.clone(),
                    confirmed: Arc::clone(&confirmed),
                    handshake: Arc::new(Mutex::new(None)),
                    silent: false, // Always show modal for first-time pairing
                    is_reconnect: false,
                });
            }

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
                    let res = confirmed.lock();
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
                let mut ap = active_pairing.lock();
                *ap = None;
            }

            if local_confirmed && remote_confirmed {
                info!("Both sides confirmed. Pairing successful.");
                let _ = store.add_paired_device(&remote_node_id, &responder_label, remote_static.as_deref());
                let _ = ipc_tx.send(IpcMessage::PairingResult {
                    success: true,
                    node_id: remote_node_id.clone(),
                    label: responder_label.clone(),
                    error: None,
                });
            } else {
                info!(
                    "Pairing failed: local_confirmed={}, remote_confirmed={}",
                    local_confirmed, remote_confirmed
                );
                let _ = ipc_tx.send(IpcMessage::PairingResult {
                    success: false,
                    node_id: remote_node_id.clone(),
                    label: responder_label.clone(),
                    error: Some("Pairing confirmation failed or was rejected".to_string()),
                });
                return Err(anyhow::anyhow!("Pairing failed or rejected"));
            }
        }

        // If trusted, exchange confirmation bytes
        if is_paired {
             // Send our confirmation
             write_ws_framed(&mut ws, &mut transport, &[1])?;
             // Wait for remote confirmation
             let _ = ws.get_ref().set_read_timeout(Some(Duration::from_secs(2)));
             match read_ws_framed(&mut ws, &mut transport) {
                 Ok(data) if !data.is_empty() && data[0] == 1 => {
                     info!("Remote side also trusted us. Session established.");
                 }
                 Ok(data) if !data.is_empty() && data[0] == 0xFF => {
                     warn!("Remote device rejected reconnection (stale pairing).");
                     let label = if let Ok(Some(device)) = store.get_paired_device(&remote_node_id) {
                         device.label
                     } else {
                         responder_label.clone()
                     };
                     let _ = ipc_tx.send(IpcMessage::StalePairing {
                         node_id: remote_node_id.clone(),
                         label,
                     });
                     return Err(anyhow::anyhow!("Stale pairing"));
                 }
                 _ => return Err(anyhow::anyhow!("Remote side did not confirm trusted session")),
             }
        }

        run_sync_session(
            ws,
            transport,
            remote_node_id.clone(),
            responder_label.clone(),
            sync_manager,
            ipc_tx,
            store,
            libp2p_manager,
        )?;
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
    store: Arc<Store>,
    libp2p_manager: Arc<Libp2pManager>,
) -> Result<()> {
    let (tx, rx) = flume::unbounded::<SyncMessage>();
    sync_manager.add_peer(node_id.clone(), tx.clone(), TransportType::Lan);

    let _ = ipc_tx.send(IpcMessage::PeerConnected {
        node_id: node_id.clone(),
    });

    info!(
        "Sync session started for {} ({}) over WebSocket",
        label, node_id
    );

    // Initial PEX: Send our known peers to this new peer
    let mut pex_records = Vec::new();

    // 1. Add our own listen addresses
    pex_records.push(cdus_common::PeerExchangeRecord {
        node_id: libp2p_manager.get_peer_id().to_string(),
        addresses: libp2p_manager
            .get_listen_addresses()
            .into_iter()
            .map(|a| a.to_string())
            .collect(),
    });

    // 2. Add other paired devices (as potential bridge points)
    if let Ok(paired) = store.get_paired_devices() {
        for device in paired {
            if device.node_id != node_id {
                let mut addrs = Vec::new();
                if let Some(ips) = device.last_known_ips {
                    if let Some(port) = device.last_known_port {
                        for ip in ips {
                            addrs.push(format!("/ip4/{}/tcp/{}", ip, port));
                        }
                    }
                }
                if !addrs.is_empty() {
                    pex_records.push(cdus_common::PeerExchangeRecord {
                        node_id: device.node_id,
                        addresses: addrs,
                    });
                }
            }
        }
    }

    if !pex_records.is_empty() {
        info!("Sending PEX update ({} peers) to {}", pex_records.len(), label);
        let pex_msg = SyncMessage::PeerExchange { peers: pex_records };
        if let Ok(data) = pex_msg.to_vec() {
            let _ = write_ws_framed(&mut ws, &mut transport, &data);
        }
    }

    // Initial Clipboard Sync: Send our latest clipboard update to the peer
    if let Ok(Some(ts_str)) = store.get_state("last_sync_timestamp") {
        if let Ok(ts) = ts_str.parse::<u64>() {
            if let Ok(Some(content)) = store.get_state("last_clipboard_content") {
                info!("Sending initial clipboard update on connection to {}: timestamp={}", label, ts);
                let sync_msg = SyncMessage::ClipboardUpdate { content, timestamp: ts };
                if let Ok(data) = sync_msg.to_vec() {
                    let _ = write_ws_framed(&mut ws, &mut transport, &data);
                }
            }
        }
    }

    // Ensure non-blocking for read-loop if needed, but we'll use can_read or short timeouts
    let _ = ws
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(100)));

    loop {
        // 1. Check for incoming messages from peer
        match read_ws_framed(&mut ws, &mut transport) {
            Ok(data) => {
                if let Ok(msg) = SyncMessage::from_slice(&data) {
                    match msg {
                        SyncMessage::ClipboardUpdate { content, timestamp } => {
                            info!("Received clipboard update from peer {}: {}", label, content);
                            let _ = ipc_tx.send(IpcMessage::SetClipboard {
                                content,
                                timestamp,
                                source: label.clone(),
                            });
                        }
                        SyncMessage::NotificationMirror(payload) => {
                            let _ = ipc_tx.send(IpcMessage::NotificationMirrored(payload));
                        }
                        SyncMessage::NotificationDismiss { key } => {
                            let _ = ipc_tx.send(IpcMessage::NotificationDismissed { key });
                        }
                        SyncMessage::PeerExchange { peers } => {
                            info!("Received PEX update from {} ({} peers)", label, peers.len());
                            for peer in peers {
                                if let Ok(peer_id) = peer.node_id.parse::<libp2p::PeerId>() {
                                    for addr_str in peer.addresses {
                                        if let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>() {
                                            libp2p_manager.inject_address(peer_id, addr);
                                        }
                                    }
                                }
                            }
                        }
                        SyncMessage::Disconnect => {
                            info!("Received Disconnect request from peer {}, closing session", label);
                            break;
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
        match rx.try_recv() {
            Ok(msg) => {
                let is_disconnect = msg == SyncMessage::Disconnect;
                let data = msg.to_vec()?;
                write_ws_framed(&mut ws, &mut transport, &data)?;
                if is_disconnect {
                    info!("Sent Disconnect to peer {}, closing session", label);
                    break;
                }
            }
            Err(flume::TryRecvError::Disconnected) => {
                info!("Outgoing channel disconnected, closing sync session for {}", label);
                break;
            }
            Err(flume::TryRecvError::Empty) => {}
        }

        thread::sleep(Duration::from_millis(100));
    }

    sync_manager.remove_peer(&node_id);
    let _ = ipc_tx.send(IpcMessage::PeerDisconnected { node_id });
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
        Err(e) => Err(std::io::Error::other(e)),
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
