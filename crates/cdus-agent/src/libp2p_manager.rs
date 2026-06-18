use crate::store::Store;
use crate::file_transfer::FileTransferManager;
use anyhow::Result;
use async_trait::async_trait;
use cdus_common::{IpcMessage, SyncMessage};
use flume::{Receiver, Sender};
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, StreamExt};
use libp2p::{
    gossipsub, identity, noise, request_response,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, PeerId,
};
use std::io;
use std::sync::Arc; use parking_lot::Mutex;
use std::thread;
use std::time::Duration;
use tokio::runtime::Runtime;
use tracing::{error, info};

#[derive(Default, Clone)]
pub struct MessagePackCodec;

#[async_trait]
impl request_response::Codec for MessagePackCodec {
    type Protocol = libp2p::StreamProtocol;
    type Request = SyncMessage;
    type Response = SyncMessage;

    async fn read_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let mut len_bytes = [0u8; 4];
        io.read_exact(&mut len_bytes).await?;
        let len = u32::from_be_bytes(len_bytes) as usize;

        let mut data = vec![0u8; len];
        io.read_exact(&mut data).await?;

        SyncMessage::from_slice(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    async fn read_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let mut len_bytes = [0u8; 4];
        io.read_exact(&mut len_bytes).await?;
        let len = u32::from_be_bytes(len_bytes) as usize;

        let mut data = vec![0u8; len];
        io.read_exact(&mut data).await?;

        SyncMessage::from_slice(&data)
            .map_err(|e| {
                error!("MessagePackCodec: failed to decode response: {}", e);
                io::Error::new(io::ErrorKind::InvalidData, e)
            })
    }

    async fn write_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        request: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        info!("MessagePackCodec: writing request: {:?}", request);
        let data = request
            .to_vec()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let len = data.len() as u32;
        io.write_all(&len.to_be_bytes()).await?;
        io.write_all(&data).await?;
        Ok(())
    }

    async fn write_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        response: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        info!("MessagePackCodec: writing response: {:?}", response);
        let data = response
            .to_vec()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let len = data.len() as u32;
        io.write_all(&len.to_be_bytes()).await?;
        io.write_all(&data).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::io::Cursor;
    use libp2p::request_response::Codec as _;

    #[tokio::test]
    async fn test_message_pack_codec_roundtrip() {
        let mut codec = MessagePackCodec::default();
        let protocol = libp2p::StreamProtocol::new("/test");
        let msg = SyncMessage::ClipboardUpdate {
            content: "hello manual test".to_string(),
            timestamp: 1337,
        };

        let mut buf = Vec::new();
        // Test Write
        {
            let mut writer = Cursor::new(&mut buf);
            <MessagePackCodec as libp2p::request_response::Codec>::write_request(&mut codec, &protocol, &mut writer, msg.clone()).await.unwrap();
        }

        // Verify length prefix (4 bytes)
        assert!(buf.len() > 4);
        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        assert_eq!(len, buf.len() - 4);

        // Test Read
        {
            let mut reader = Cursor::new(&mut buf);
            let decoded: SyncMessage = <MessagePackCodec as libp2p::request_response::Codec>::read_request(&mut codec, &protocol, &mut reader).await.unwrap();
            assert_eq!(msg, decoded);
        }
    }
}

#[derive(NetworkBehaviour)]
pub struct CdusBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: libp2p::mdns::tokio::Behaviour,
    pub identify: libp2p::identify::Behaviour,
    pub dcutr: libp2p::dcutr::Behaviour,
    pub relay_client: libp2p::relay::client::Behaviour,
    pub request_response: request_response::Behaviour<MessagePackCodec>,
    pub stream: libp2p_stream::Behaviour,
}

use crate::pairing::SyncManager;

#[derive(Debug, Clone)]
pub enum SwarmCommand {
    AddAddress(PeerId, libp2p::Multiaddr),
    Disconnect(PeerId),
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
    command_tx: Sender<SwarmCommand>,
    command_rx: Receiver<SwarmCommand>,
    store: Arc<Store>,
    file_pool: threadpool::ThreadPool,
    stream_control: Mutex<Option<libp2p_stream::Control>>,
    transfer_manager: Arc<FileTransferManager>,
    download_dir: Option<std::path::PathBuf>,
    listen_addresses: Arc<Mutex<Vec<libp2p::Multiaddr>>>,
    pub sync_manager: parking_lot::RwLock<Option<Arc<SyncManager>>>,
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
        let (command_tx, command_rx) = flume::unbounded();

        Ok(Self {
            peer_id,
            keypair,
            runtime: Arc::new(runtime),
            tx,
            sync_tx,
            sync_rx,
            request_tx,
            request_rx,
            command_tx,
            command_rx,
            store,
            file_pool: threadpool::ThreadPool::new(4),
            stream_control: Mutex::new(None),
            transfer_manager,
            download_dir,
            listen_addresses: Arc::new(Mutex::new(Vec::new())),
            sync_manager: parking_lot::RwLock::new(None),
        })
    }

    pub fn inject_address(&self, peer_id: PeerId, addr: libp2p::Multiaddr) {
        let _ = self.command_tx.send(SwarmCommand::AddAddress(peer_id, addr));
    }

    pub fn get_peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub fn get_sync_tx(&self) -> Sender<SyncMessage> {
        self.sync_tx.clone()
    }

    pub fn get_listen_addresses(&self) -> Vec<libp2p::Multiaddr> {
        self.listen_addresses.lock().clone()
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

    pub fn start(&self, sync_manager: Arc<SyncManager>) {
        *self.sync_manager.write() = Some(Arc::clone(&sync_manager));
        let sync_manager_clone = Arc::clone(&sync_manager);
        let runtime = Arc::clone(&self.runtime);
        let runtime_for_pool = Arc::clone(&self.runtime);
        let keypair = self.keypair.clone();
        let tx = self.tx.clone();
        let sync_rx = self.sync_rx.clone();
        let request_rx = self.request_rx.clone();
        let command_rx = self.command_rx.clone();
        let store = Arc::clone(&self.store);
        let peer_id = self.peer_id;
        let file_pool = self.file_pool.clone();
        let transfer_manager = Arc::clone(&self.transfer_manager);
        let download_dir_custom = self.download_dir.clone();
        let listen_addresses_clone = Arc::clone(&self.listen_addresses);
        
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

                        let request_response = request_response::Behaviour::with_codec(
                            MessagePackCodec::default(),
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
                                if !sync_manager_clone.is_connected(&peer.to_string()) {
                                    error!("Incoming file stream rejected: Peer {} is disconnected", peer);
                                    continue;
                                }
                                let store_clone = Arc::clone(&store);
                                let tx_clone = tx.clone();
                                let tm_clone = Arc::clone(&transfer_manager);
                                let peer_str = peer.to_string();
                                let download_dir_custom_clone = download_dir_custom.clone();
                                let runtime_handle = runtime_for_pool.handle().clone();
                                file_pool.execute(move || {
                                    let wrapped_stream = crate::file_transfer::Libp2pFileStream::new(
                                        stream, 
                                        &runtime_handle 
                                    );
                                    // TODO: Get real session key
                                    let session_key = crate::file_transfer::SessionKey([0u8; 32]);

                                     let download_dir = if let Some(custom) = download_dir_custom_clone {
                                         custom
                                     } else if let Some(user_dirs) = directories::UserDirs::new() {
                                         user_dirs.download_dir()
                                             .map(|d| d.join("cdus"))
                                             .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join("cdus"))
                                     } else {
                                         std::env::current_dir().unwrap_or_default().join("cdus")
                                     };
                                     if let Err(e) = std::fs::create_dir_all(&download_dir) {
                                         error!("Failed to create download directory {:?}: {}", download_dir, e);
                                     }
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
                                    listen_addresses_clone.lock().push(address);
                                }
                                SwarmEvent::ExpiredListenAddr { address, .. } => {
                                    info!("Libp2p listen address expired: {:?}", address);
                                    listen_addresses_clone.lock().retain(|a| a != &address);
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
                                                // Verify peer is paired and connected
                                                if let Ok(true) = store.is_device_paired(&propagation_source.to_string()) {
                                                    if sync_manager_clone.is_connected(&propagation_source.to_string()) {
                                                        match sync_msg {
                                                            SyncMessage::ClipboardUpdate { content, timestamp } => {
                                                                let _ = tx.send(IpcMessage::SetClipboard {
                                                                    content,
                                                                    timestamp,
                                                                    source: format!("libp2p:{}", propagation_source)
                                                                });
                                                            }
                                                            SyncMessage::PeerExchange { peers } => {
                                                                info!("Received PEX via Gossipsub from {} ({} peers)", propagation_source, peers.len());
                                                                for peer_rec in peers {
                                                                    if let Ok(peer_id) = peer_rec.node_id.parse::<libp2p::PeerId>() {
                                                                        for addr_str in peer_rec.addresses {
                                                                            if let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>() {
                                                                                swarm.add_peer_address(peer_id, addr);
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                            SyncMessage::Disconnect => {
                                                                info!("Received Disconnect request over Gossipsub from {}", propagation_source);
                                                            }
                                                        }
                                                    } else {
                                                        info!("Ignoring Gossipsub message from disconnected peer {}", propagation_source);
                                                    }
                                                }
                                            }
                                        }
                                        CdusBehaviourEvent::RequestResponse(request_response::Event::Message { peer, message, .. }) => {
                                            match message {
                                                request_response::Message::Request { request, channel, .. } => {
                                                    info!("Received libp2p Request from {}: {:?}", peer, request);
                                                    
                                                    // If it's PEX, process it
                                                    if let SyncMessage::PeerExchange { ref peers } = request {
                                                        info!("Processing PEX Request from {} ({} peers)", peer, peers.len());
                                                        for peer_rec in peers {
                                                            if let Ok(peer_id) = peer_rec.node_id.parse::<libp2p::PeerId>() {
                                                                for addr_str in &peer_rec.addresses {
                                                                    if let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>() {
                                                                        swarm.add_peer_address(peer_id, addr);
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }

                                                    // Echo back for testing
                                                    let _ = swarm.behaviour_mut().request_response.send_response(channel, request);
                                                }
                                                request_response::Message::Response { response, .. } => {
                                                    info!("Received libp2p Response from {}: {:?}", peer, response);
                                                    if let SyncMessage::PeerExchange { peers } = response {
                                                        info!("Processing PEX Response from {} ({} peers)", peer, peers.len());
                                                        for peer_rec in peers {
                                                            if let Ok(peer_id) = peer_rec.node_id.parse::<libp2p::PeerId>() {
                                                                for addr_str in peer_rec.addresses {
                                                                    if let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>() {
                                                                        swarm.add_peer_address(peer_id, addr);
                                                                    }
                                                                }
                                                            }
                                                        }
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
                        cmd = command_rx.recv_async() => {
                            if let Ok(cmd) = cmd {
                                match cmd {
                                    SwarmCommand::AddAddress(peer_id, addr) => {
                                        info!("Manually adding address for {}: {}", peer_id, addr);
                                        swarm.add_peer_address(peer_id, addr);
                                        let _ = swarm.dial(peer_id);
                                    }
                                    SwarmCommand::Disconnect(peer_id) => {
                                        info!("Manually disconnecting libp2p peer: {}", peer_id);
                                        let _ = swarm.disconnect_peer_id(peer_id);
                                    }
                                }
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

    pub fn open_file_stream(&self, peer_id: PeerId) -> Result<crate::file_transfer::Libp2pFileStream> {
        let is_connected = self.sync_manager.read().as_ref()
            .map(|sm| sm.is_connected(&peer_id.to_string()))
            .unwrap_or(false);
        if !is_connected {
            return Err(anyhow::anyhow!("Cannot open stream: Peer {} is disconnected", peer_id));
        }

        let control = self.stream_control.lock().clone()
            .ok_or_else(|| anyhow::anyhow!("Stream control not initialized"))?;
        let protocol = libp2p::StreamProtocol::new("/cdus/file/1.0.0");
        
        let stream = self.runtime.block_on(async {
            control.clone().open_stream(peer_id, protocol).await
        })?;
        
        Ok(crate::file_transfer::Libp2pFileStream::new(stream, self.runtime.handle()))
    }

    pub fn disconnect_peer(&self, peer_id: PeerId) {
        let _ = self.command_tx.send(SwarmCommand::Disconnect(peer_id));
    }
}
