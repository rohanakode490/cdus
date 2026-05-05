use snow::{Builder, params::NoiseParams};
use std::net::{SocketAddr, IpAddr};
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, error};
use std::sync::Arc;
use flume::Sender;
use cdus_common::IpcMessage;
use crate::store::Store;

pub struct PairingManager {
    store: Arc<Store>,
    ipc_tx: Sender<IpcMessage>,
    node_id: String,
    private_key: Vec<u8>,
}

impl PairingManager {
    pub fn new(store: Arc<Store>, ipc_tx: Sender<IpcMessage>, node_id: String, private_key: Vec<u8>) -> Self {
        Self { store, ipc_tx, node_id, private_key }
    }

    pub async fn start_listener(&self) {
        let addr = "0.0.0.0:5200";
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                error!("Failed to bind pairing listener on {}: {}", addr, e);
                return;
            }
        };

        info!("Pairing listener active on {}", addr);

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let store = Arc::clone(&self.store);
                    let ipc_tx = self.ipc_tx.clone();
                    let priv_key = self.private_key.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_incoming_pairing(stream, store, ipc_tx, priv_key).await {
                            error!("Error in incoming pairing: {}", e);
                        }
                    });
                }
                Err(e) => error!("Failed to accept connection: {}", e),
            }
        }
    }

    pub async fn initiate_pairing(&self, target_addr: SocketAddr) {
        let stream = match TcpStream::connect(target_addr).await {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to connect to target {}: {}", target_addr, e);
                return;
            }
        };

        let store = Arc::clone(&self.store);
        let ipc_tx = self.ipc_tx.clone();
        let priv_key = self.private_key.clone();
        
        tokio::spawn(async move {
            if let Err(e) = handle_outgoing_pairing(stream, store, ipc_tx, priv_key).await {
                error!("Error in outgoing pairing: {}", e);
            }
        });
    }
}

async fn handle_incoming_pairing(mut stream: TcpStream, _store: Arc<Store>, ipc_tx: Sender<IpcMessage>, priv_key: Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
    info!("Handling incoming pairing request");
    
    let params: NoiseParams = "Noise_XX_25519_ChaChaPoly_BLAKE2s".parse()?;
    let mut builder = Builder::new(params);
    builder = builder.local_private_key(&priv_key);
    let mut noise = builder.build_responder()?;

    let mut buf = [0u8; 1024];

    // 1. Read e
    let n = stream.read(&mut buf).await?;
    noise.read_message(&buf[..n], &mut [0u8; 1024])?;

    // 2. Write e, ee, s, es
    let n = noise.write_message(&[], &mut buf)?;
    stream.write_all(&buf[..n]).await?;

    // 3. Read s, se
    let n = stream.read(&mut buf).await?;
    noise.read_message(&buf[..n], &mut [0u8; 1024])?;

    // Handshake finished. Derive PIN.
    let h = noise.get_handshake_hash();
    let pin = derive_pin(h);
    info!("Incoming pairing PIN: {}", pin);
    
    // Notify UI
    let _ = ipc_tx.send(IpcMessage::PairingPin(pin));

    // Wait for UI confirmation (in a real app we'd need a channel here)
    // For now we'll assume success if the handshake finished
    
    Ok(())
}

async fn handle_outgoing_pairing(mut stream: TcpStream, _store: Arc<Store>, ipc_tx: Sender<IpcMessage>, priv_key: Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
    info!("Initiating outgoing pairing request");

    let params: NoiseParams = "Noise_XX_25519_ChaChaPoly_BLAKE2s".parse()?;
    let mut builder = Builder::new(params);
    builder = builder.local_private_key(&priv_key);
    let mut noise = builder.build_initiator()?;

    let mut buf = [0u8; 1024];

    // 1. Write e
    let n = noise.write_message(&[], &mut buf)?;
    stream.write_all(&buf[..n]).await?;

    // 2. Read e, ee, s, es
    let n = stream.read(&mut buf).await?;
    noise.read_message(&buf[..n], &mut [0u8; 1024])?;

    // 3. Write s, se
    let n = noise.write_message(&[], &mut buf)?;
    stream.write_all(&buf[..n]).await?;

    // Handshake finished. Derive PIN.
    let h = noise.get_handshake_hash();
    let pin = derive_pin(h);
    info!("Outgoing pairing PIN: {}", pin);

    // Notify UI
    let _ = ipc_tx.send(IpcMessage::PairingPin(pin));

    Ok(())
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
