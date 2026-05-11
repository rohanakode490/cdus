use anyhow::Result;
use flume::{Receiver, Sender};
use libp2p::{
    futures::StreamExt,
    gossipsub,
    identity,
    noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp,
    yamux,
    PeerId,
};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::runtime::Runtime;
use tracing::{error, info, warn};
use cdus_common::{SyncMessage, IpcMessage};
use crate::store::Store;

#[derive(NetworkBehaviour)]
pub struct CdusBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub identify: libp2p::identify::Behaviour,
    pub dcutr: libp2p::dcutr::Behaviour,
    pub relay_client: libp2p::relay::client::Behaviour,
}

pub struct Libp2pManager {
    peer_id: PeerId,
    keypair: identity::Keypair,
    runtime: Arc<Runtime>,
    tx: Sender<IpcMessage>,
    sync_tx: Sender<SyncMessage>,
    sync_rx: Receiver<SyncMessage>,
    store: Arc<Store>,
}

impl Libp2pManager {
    pub fn new(priv_bytes: Vec<u8>, tx: Sender<IpcMessage>, store: Arc<Store>) -> Result<Self> {
        let mut key_bytes = priv_bytes;
        let keypair = identity::Keypair::ed25519_from_bytes(&mut key_bytes)?;
        let peer_id = PeerId::from(keypair.public());
        
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
            
        let (sync_tx, sync_rx) = flume::unbounded();

        Ok(Self {
            peer_id,
            keypair,
            runtime: Arc::new(runtime),
            tx,
            sync_tx,
            sync_rx,
            store,
        })
    }

    pub fn get_sync_tx(&self) -> Sender<SyncMessage> {
        self.sync_tx.clone()
    }

    pub fn start(&self) {
        let runtime = Arc::clone(&self.runtime);
        let keypair = self.keypair.clone();
        let tx = self.tx.clone();
        let sync_rx = self.sync_rx.clone();
        let store = Arc::clone(&self.store);
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

                        CdusBehaviour {
                            gossipsub,
                            identify,
                            dcutr: libp2p::dcutr::Behaviour::new(key.public().to_peer_id()),
                            relay_client,
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
                                        CdusBehaviourEvent::Gossipsub(gossipsub::Event::Message { propagation_source, message_id, message }) => {
                                            info!("Received gossipsub message {} from {}", message_id, propagation_source);
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
                                                    }
                                                } else {
                                                    warn!("Received gossipsub message from unpaired peer {}", propagation_source);
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
                                    }
                                }
                            }
                        }
                    }
                }
            });
        });
    }
}
