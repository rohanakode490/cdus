use crate::store::Store;
use anyhow::Result;
use cdus_common::{IpcMessage, SyncMessage, TransferProgress};
use flume::{Receiver, Sender};
use libp2p::{
    futures::StreamExt,
    gossipsub, identity, noise, request_response,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, PeerId,
};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tokio::runtime::Runtime;
use tracing::{error, info, warn};

#[derive(NetworkBehaviour)]
pub struct CdusBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: libp2p::mdns::tokio::Behaviour,
    pub identify: libp2p::identify::Behaviour,
    pub dcutr: libp2p::dcutr::Behaviour,
    pub relay_client: libp2p::relay::client::Behaviour,
    pub request_response: request_response::json::Behaviour<SyncMessage, SyncMessage>,
}

pub struct Libp2pManager {
    peer_id: PeerId,
    keypair: identity::Keypair,
    runtime: Arc<Runtime>,
    tx: Sender<IpcMessage>,
    sync_tx: Sender<SyncMessage>,
    sync_rx: Receiver<SyncMessage>,
    request_tx: Sender<(PeerId, SyncMessage)>,
    request_rx: Receiver<(PeerId, SyncMessage)>,
    store: Arc<Store>,
    active_transfers: Arc<
        Mutex<std::collections::HashMap<String, (std::path::PathBuf, cdus_common::FileManifest)>>,
    >,
    received_manifests: Arc<Mutex<std::collections::HashMap<String, TransferProgress>>>,
}

impl Libp2pManager {
    pub fn new(
        priv_bytes: Vec<u8>,
        tx: Sender<IpcMessage>,
        store: Arc<Store>,
        active_transfers: Arc<
            Mutex<
                std::collections::HashMap<String, (std::path::PathBuf, cdus_common::FileManifest)>,
            >,
        >,
        received_manifests: Arc<Mutex<std::collections::HashMap<String, TransferProgress>>>,
    ) -> Result<Self> {
        let mut key_bytes = priv_bytes;
        let keypair = identity::Keypair::ed25519_from_bytes(&mut key_bytes)?;
        let peer_id = PeerId::from(keypair.public());

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        let (sync_tx, sync_rx) = flume::unbounded();
        let (request_tx, request_rx) = flume::unbounded();

        Ok(Self {
            peer_id,
            keypair,
            runtime: Arc::new(runtime),
            tx,
            sync_tx,
            sync_rx,
            request_tx,
            request_rx,
            store,
            active_transfers,
            received_manifests,
        })
    }

    pub fn get_sync_tx(&self) -> Sender<SyncMessage> {
        self.sync_tx.clone()
    }

    pub fn get_request_tx(&self) -> Sender<(PeerId, SyncMessage)> {
        self.request_tx.clone()
    }

    pub fn start(&self) {
        let runtime = Arc::clone(&self.runtime);
        let keypair = self.keypair.clone();
        let tx = self.tx.clone();
        let sync_rx = self.sync_rx.clone();
        let request_rx = self.request_rx.clone();
        let store = Arc::clone(&self.store);
        let active_transfers: Arc<
            Mutex<
                std::collections::HashMap<String, (std::path::PathBuf, cdus_common::FileManifest)>,
            >,
        > = Arc::clone(&self.active_transfers);
        let _received_manifests: Arc<Mutex<std::collections::HashMap<String, TransferProgress>>> =
            Arc::clone(&self.received_manifests);
        let peer_id = self.peer_id;

        thread::spawn(move || {
            runtime.block_on(async {
                let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
                    .with_tokio()
                    .with_tcp(
                        tcp::Config::default(),
                        noise::Config::new,
                        yamux::Config::default,
                    ).expect("TCP config")
                    .with_quic()
                    .with_relay_client(noise::Config::new, yamux::Config::default).expect("Relay client config")
                    .with_behaviour(|key, relay_client| {
                        let gossipsub_config = gossipsub::ConfigBuilder::default()
                            .heartbeat_interval(Duration::from_secs(10))
                            .validation_mode(gossipsub::ValidationMode::Strict)
                            .build()
                            .expect("Valid config");

                        let gossipsub = gossipsub::Behaviour::new(
                            gossipsub::MessageAuthenticity::Signed(key.clone()),
                            gossipsub_config,
                        ).expect("Valid behaviour");

                        let identify = libp2p::identify::Behaviour::new(
                            libp2p::identify::Config::new("/cdus/1.0.0".into(), key.public()),
                        );

                        let request_response = request_response::json::Behaviour::new(
                            [(
                                libp2p::StreamProtocol::new("/cdus/file-transfer/1.0.0"),
                                request_response::ProtocolSupport::Full,
                            )],
                            request_response::Config::default(),
                        );

                        let mdns = libp2p::mdns::tokio::Behaviour::new(
                            libp2p::mdns::Config::default(),
                            key.public().to_peer_id(),
                        ).expect("Valid mdns behaviour");

                        CdusBehaviour {
                            gossipsub,
                            mdns,
                            identify,
                            dcutr: libp2p::dcutr::Behaviour::new(key.public().to_peer_id()),
                            relay_client,
                            request_response,
                        }
                    }).expect("Behaviour config")
                    .build();

                let topic = gossipsub::IdentTopic::new("cdus/sync/v1");
                swarm.behaviour_mut().gossipsub.subscribe(&topic).expect("Gossipsub subscribe");

                swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse().expect("Valid multiaddr")).expect("Listen on TCP");
                swarm.listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse().expect("Valid multiaddr")).expect("Listen on QUIC");

                info!("Libp2p swarm started. Peer ID: {}", peer_id);

                loop {
                    tokio::select! {
                        event = swarm.select_next_some() => {
                            match event {
                                SwarmEvent::NewListenAddr { address, .. } => {
                                    info!("Libp2p listening on {:?}", address);
                                }
                                SwarmEvent::Behaviour(event) => {
                                    match event {
                                        CdusBehaviourEvent::Mdns(libp2p::mdns::Event::Discovered(list)) => {
                                            for (peer_id, multiaddr) in list {
                                                info!("mDNS discovered libp2p peer: {} at {}", peer_id, multiaddr);
                                                let _ = swarm.dial(multiaddr.clone());
                                                swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                                            }
                                        }
                                        CdusBehaviourEvent::Mdns(libp2p::mdns::Event::Expired(list)) => {
                                            for (peer_id, multiaddr) in list {
                                                info!("mDNS peer expired: {} at {}", peer_id, multiaddr);
                                                swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                                            }
                                        }
                                        CdusBehaviourEvent::Gossipsub(gossipsub::Event::Message { propagation_source, message, .. }) => {
                                            if let Ok(sync_msg) = serde_json::from_slice::<SyncMessage>(&message.data) {
                                                // Verify peer is paired
                                                if let Ok(true) = store.is_device_paired(&propagation_source.to_string()) {
                                                    match sync_msg {
                                                        SyncMessage::ClipboardUpdate { content, timestamp } => {
                                                            let _ = tx.send(IpcMessage::SetClipboard {
                                                                content,
                                                                timestamp,
                                                                source: format!("libp2p:{}", propagation_source)
                                                            });
                                                        }
                                                        SyncMessage::FileTransferRequest(manifest) => {
                                                            let _ = tx.send(IpcMessage::IncomingFileRequest {
                                                                node_id: propagation_source.to_string(),
                                                                manifest,
                                                            });
                                                        }
                                                        SyncMessage::FileTransferAccepted { file_hash } => {
                                                            let _ = tx.send(IpcMessage::AcceptFileTransfer { file_hash });
                                                        }
                                                        SyncMessage::FileTransferRejected { file_hash } => {
                                                            let _ = tx.send(IpcMessage::RejectFileTransfer { file_hash });
                                                        }
                                                        _ => {}
                                                    }
                                                } else {
                                                    warn!("Received gossipsub message from unpaired peer {}", propagation_source);
                                                }
                                            }
                                        }
                                        CdusBehaviourEvent::RequestResponse(request_response::Event::Message { peer, message, .. }) => {
                                            match message {
                                                request_response::Message::Request { request, channel, .. } => {
                                                    match request {
                                                        SyncMessage::FileTransferRequest(manifest) => {
                                                            info!("Libp2p: Received direct FileTransferRequest for {}", manifest.file_name);
                                                            let _ = tx.send(IpcMessage::IncomingFileRequest {
                                                                node_id: peer.to_string(),
                                                                manifest,
                                                            });
                                                            let response = SyncMessage::FileTransferAccepted { file_hash: "ack".to_string() };
                                                            let _ = swarm.behaviour_mut().request_response.send_response(channel, response);
                                                        }
                                                        SyncMessage::FileTransferAccepted { file_hash } => {
                                                            info!("Libp2p: Received direct FileTransferAccepted for {}", file_hash);
                                                            let _ = tx.send(IpcMessage::AcceptFileTransfer { file_hash });
                                                            let response = SyncMessage::FileTransferAccepted { file_hash: "ack".to_string() };
                                                            let _ = swarm.behaviour_mut().request_response.send_response(channel, response);
                                                        }
                                                        SyncMessage::FileTransferRejected { file_hash } => {
                                                            info!("Libp2p: Received direct FileTransferRejected for {}", file_hash);
                                                            let _ = tx.send(IpcMessage::RejectFileTransfer { file_hash });
                                                            let response = SyncMessage::FileTransferAccepted { file_hash: "ack".to_string() };
                                                            let _ = swarm.behaviour_mut().request_response.send_response(channel, response);
                                                        }
                                                        SyncMessage::ChunkRequest { file_hash, chunk_hash } => {
                                                            info!("Received ChunkRequest for {} / {}", file_hash, chunk_hash);
                                                            let transfer_info = {
                                                                let at = active_transfers.lock().unwrap();
                                                                at.get(&file_hash).cloned()
                                                            };

                                                            if let Some((path, manifest)) = transfer_info {
                                                                if let Some(chunk) = manifest.chunks.iter().find(|c| c.hash == chunk_hash) {
                                                                    match crate::file_transfer::get_chunk(&path, chunk.offset, chunk.size) {
                                                                        Ok(data) => {
                                                                            let response = SyncMessage::ChunkResponse {
                                                                                file_hash: file_hash.clone(),
                                                                                chunk_hash: chunk_hash.clone(),
                                                                                data
                                                                            };
                                                                            let _ = swarm.behaviour_mut().request_response.send_response(channel, response);
                                                                            let _ = tx.send(IpcMessage::ChunkServed { file_hash, chunk_hash });
                                                                        }
                                                                        Err(e) => {
                                                                            error!("Failed to read chunk: {}", e);
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                                request_response::Message::Response { response, .. } => {
                                                    match response {
                                                        SyncMessage::ChunkResponse { file_hash, chunk_hash, data } => {
                                                            let _ = tx.send(IpcMessage::ChunkReceived { file_hash, chunk_hash, data });
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                _ => {}
                            }
                        }
                        msg = sync_rx.recv_async() => {
                            if let Ok(sync_msg) = msg {
                                if let Ok(data) = serde_json::to_vec(&sync_msg) {
                                    if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), data) {
                                        error!("Gossipsub publish failed: {}", e);
                                        let _ = tx.send(IpcMessage::FileTransferError {
                                            file_hash: "broadcast".to_string(),
                                            error: format!("Network error: {}. Make sure devices are on the same WiFi and paired.", e)
                                        });
                                    }
                                }
                            }
                        }
                        req = request_rx.recv_async() => {
                            if let Ok((peer, sync_msg)) = req {
                                swarm.behaviour_mut().request_response.send_request(&peer, sync_msg);
                            }
                        }
                    }
                }
            });
        });
    }
}
