use crate::store::Store;
use crate::file_transfer::FileTransferManager;
use anyhow::Result;
use cdus_common::{IpcMessage, SyncMessage};
use flume::{Receiver, Sender};
use libp2p::{
    futures::StreamExt,
    gossipsub, identity, noise, request_response,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, PeerId,
};
use std::sync::Arc; use parking_lot::Mutex;
use std::thread;
use std::time::Duration;
use tokio::runtime::Runtime;
use tracing::{error, info};

#[derive(NetworkBehaviour)]
pub struct CdusBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: libp2p::mdns::tokio::Behaviour,
    pub identify: libp2p::identify::Behaviour,
    pub dcutr: libp2p::dcutr::Behaviour,
    pub relay_client: libp2p::relay::client::Behaviour,
    pub request_response: request_response::cbor::Behaviour<SyncMessage, SyncMessage>,
    pub stream: libp2p_stream::Behaviour,
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
    file_pool: threadpool::ThreadPool,
    stream_control: Mutex<Option<libp2p_stream::Control>>,
    transfer_manager: Arc<FileTransferManager>,
    download_dir: Option<std::path::PathBuf>,
}

impl Libp2pManager {
    pub fn new(
        priv_bytes: Vec<u8>,
        tx: Sender<IpcMessage>,
        store: Arc<Store>,
        transfer_manager: Arc<FileTransferManager>,
    ) -> Result<Self> {
        Self::new_with_download_dir(priv_bytes, tx, store, transfer_manager, None)
    }

    pub fn new_with_download_dir(
        priv_bytes: Vec<u8>,
        tx: Sender<IpcMessage>,
        store: Arc<Store>,
        transfer_manager: Arc<FileTransferManager>,
        download_dir: Option<std::path::PathBuf>,
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
            file_pool: threadpool::ThreadPool::new(4),
            stream_control: Mutex::new(None),
            transfer_manager,
            download_dir,
        })
    }

    pub fn get_sync_tx(&self) -> Sender<SyncMessage> {
        self.sync_tx.clone()
    }

    pub fn get_request_tx(&self) -> Sender<(PeerId, SyncMessage)> {
        self.request_tx.clone()
    }

    pub fn get_transfer_manager(&self) -> Arc<FileTransferManager> {
        Arc::clone(&self.transfer_manager)
    }

    pub fn runtime_handle(&self) -> tokio::runtime::Handle {
        self.runtime.handle().clone()
    }

    pub fn start(&self) {
        let runtime = Arc::clone(&self.runtime);
        let runtime_for_pool = Arc::clone(&self.runtime);
        let keypair = self.keypair.clone();
        let tx = self.tx.clone();
        let sync_rx = self.sync_rx.clone();
        let request_rx = self.request_rx.clone();
        let store = Arc::clone(&self.store);
        let peer_id = self.peer_id;
        let file_pool = self.file_pool.clone();
        let transfer_manager = Arc::clone(&self.transfer_manager);
        let download_dir_custom = self.download_dir.clone();
        
        let (control_tx, control_rx) = flume::bounded(1);

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

                        let mut codec = request_response::cbor::codec::Codec::default();
                        codec = codec.set_request_size_maximum(256 * 1024 * 1024);
                        codec = codec.set_response_size_maximum(256 * 1024 * 1024);

                        let request_response = request_response::cbor::Behaviour::with_codec(
                            codec,
                            [(
                                libp2p::StreamProtocol::new("/cdus/sync/1.0.0"),
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
                            stream: libp2p_stream::Behaviour::new(),
                        }
                    }).expect("Behaviour config")
                    .build();

                let mut stream_control = swarm.behaviour_mut().stream.new_control();
                let protocol = libp2p::StreamProtocol::new("/cdus/file/1.0.0");
                let mut incoming_streams = stream_control.accept(protocol).expect("Accept protocol");
                
                let _ = control_tx.send(stream_control.clone());

                let topic = gossipsub::IdentTopic::new("cdus/sync/v1");
                swarm.behaviour_mut().gossipsub.subscribe(&topic).expect("Gossipsub subscribe");

                swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse().expect("Valid multiaddr")).expect("Listen on TCP");
                swarm.listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse().expect("Valid multiaddr")).expect("Listen on QUIC");

                info!("Libp2p swarm started. Peer ID: {}", peer_id);

                loop {
                    tokio::select! {
                        incoming = incoming_streams.next() => {
                            if let Some((peer, stream)) = incoming {
                                info!("Incoming file stream from {}", peer);
                                let store_clone = Arc::clone(&store);
                                let tx_clone = tx.clone();
                                let tm_clone = Arc::clone(&transfer_manager);
                                let peer_str = peer.to_string();
                                let download_dir_custom_clone = download_dir_custom.clone();
                                let runtime_handle = runtime_for_pool.handle().clone();
                                file_pool.execute(move || {
                                    let wrapped_stream = crate::file_transfer::Libp2pFileStream { 
                                        stream, 
                                        runtime: runtime_handle 
                                    };
                                    // TODO: Get real session key
                                    let session_key = crate::file_transfer::SessionKey([0u8; 32]);

                                    let download_dir = if let Some(custom) = download_dir_custom_clone {
                                        custom
                                    } else if let Some(user_dirs) = directories::UserDirs::new() {
                                        user_dirs.download_dir()
                                            .map(|d| d.to_path_buf())
                                            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
                                    } else {
                                        std::env::current_dir().unwrap_or_default()
                                    };
                                    let (p_tx, p_rx) = flume::unbounded();
                                    let tx_for_progress = tx_clone.clone();
                                    thread::spawn(move || {
                                        while let Ok(event) = p_rx.recv() {
                                            let _ = tx_for_progress.send(cdus_common::IpcMessage::FileProgress(event));
                                        }
                                    });

                                    if let Err(e) = crate::file_transfer::handle_incoming_transfer_with_manager(
                                        Box::new(wrapped_stream),
                                        store_clone,
                                        session_key,
                                        download_dir,
                                        p_tx,
                                        tm_clone,
                                        peer_str,
                                    ) {
                                        error!("Incoming file transfer failed: {}", e);
                                    }
                                });
                            }
                        }
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
                                            if let Ok(sync_msg) = SyncMessage::from_slice(&message.data) {
                                                // Verify peer is paired
                                                if let Ok(true) = store.is_device_paired(&propagation_source.to_string()) {
                                                    let SyncMessage::ClipboardUpdate { content, timestamp } = sync_msg;
                                                    let _ = tx.send(IpcMessage::SetClipboard {
                                                        content,
                                                        timestamp,
                                                        source: format!("libp2p:{}", propagation_source)
                                                    });
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
                                if let Ok(data) = sync_msg.to_vec() {
                                    if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), data) {
                                        error!("Gossipsub publish failed: {}", e);
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

        if let Ok(control) = control_rx.recv() {
            *self.stream_control.lock() = Some(control);
        }
    }

    pub fn open_file_stream(&self, peer_id: PeerId) -> Result<libp2p::Stream> {
        let control = self.stream_control.lock().clone()
            .ok_or_else(|| anyhow::anyhow!("Stream control not initialized"))?;
        let protocol = libp2p::StreamProtocol::new("/cdus/file/1.0.0");
        
        self.runtime.block_on(async {
            let stream = control.clone().open_stream(peer_id, protocol).await?;
            Ok(stream)
        })
    }
}
