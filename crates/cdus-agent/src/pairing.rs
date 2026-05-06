use snow::{Builder, params::NoiseParams};
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, error};
use std::sync::Arc;
use flume::Sender;
use cdus_common::IpcMessage;
use crate::store::Store;
use crate::ActivePairingState;
use std::sync::Mutex;
use std::time::Duration;

pub struct PairingManager {
    store: Arc<Store>,
    ipc_tx: Sender<IpcMessage>,
    node_id: String,
    private_key: Vec<u8>,
    port: u16,
    active_pairing: Arc<Mutex<Option<ActivePairingState>>>,
}

impl PairingManager {
    pub fn new(store: Arc<Store>, ipc_tx: Sender<IpcMessage>, node_id: String, private_key: Vec<u8>, port: u16, active_pairing: Arc<Mutex<Option<ActivePairingState>>>) -> Self {
        Self { store, ipc_tx, node_id, private_key, port, active_pairing }
    }

    pub async fn start_listener(&self) {
        let addr = format!("0.0.0.0:{}", self.port);
        let listener = match TcpListener::bind(&addr).await {
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
                    let active_pairing = Arc::clone(&self.active_pairing);
                    tokio::spawn(async move {
                        if let Err(e) = handle_incoming_pairing(stream, store, ipc_tx, priv_key, active_pairing).await {
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
        let active_pairing = Arc::clone(&self.active_pairing);
        
        tokio::spawn(async move {
            if let Err(e) = handle_outgoing_pairing(stream, store, ipc_tx, priv_key, active_pairing).await {
                error!("Error in outgoing pairing: {}", e);
            }
        });
    }
}

async fn handle_incoming_pairing(mut stream: TcpStream, store: Arc<Store>, ipc_tx: Sender<IpcMessage>, priv_key: Vec<u8>, active_pairing: Arc<Mutex<Option<ActivePairingState>>>) -> Result<(), Box<dyn std::error::Error>> {
    info!("Handling incoming pairing request");
    
    let self_label = store.get_state("device_name")?.unwrap_or_else(|| "Unknown Device".to_string());
    
    let params: NoiseParams = "Noise_XX_25519_ChaChaPoly_BLAKE2s".parse()?;
    let mut builder = Builder::new(params);
    builder = builder.local_private_key(&priv_key);
    let mut noise = builder.build_responder()?;

    let mut buf = [0u8; 1024];

    // 1. Read e
    let n = stream.read(&mut buf).await?;
    noise.read_message(&buf[..n], &mut [0u8; 1024])?;

    // 2. Write e, ee, s, es + Responder's label
    let n = noise.write_message(self_label.as_bytes(), &mut buf)?;
    stream.write_all(&buf[..n]).await?;

    // 3. Read s, se + Initiator's label
    let mut initiator_label_buf = [0u8; 1024];
    let n = stream.read(&mut buf).await?;
    let label_len = noise.read_message(&buf[..n], &mut initiator_label_buf)?;
    let initiator_label = String::from_utf8_lossy(&initiator_label_buf[..label_len]).to_string();

    // Handshake finished. Derive PIN.
    let h = noise.get_handshake_hash();
    let pin = derive_pin(h);
    let remote_node_id = hex::encode(noise.get_remote_static().unwrap());
    
    info!("Incoming pairing PIN: {} from {} ({})", pin, initiator_label, remote_node_id);
    
    // Update state
    let confirmed = Arc::new(Mutex::new(None::<bool>));
    {
        let mut ap = active_pairing.lock().unwrap();
        *ap = Some(ActivePairingState {
            pin: pin.clone(),
            is_initiator: false,
            remote_id: remote_node_id.clone(),
            remote_label: initiator_label.clone(),
            confirmed: Arc::clone(&confirmed),
        });
    }

    // Wait for local confirmation
    let mut success = false;
    let mut incoming_buf = [0u8; 1];
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                let res = confirmed.lock().unwrap();
                if let Some(accepted) = *res {
                    success = accepted;
                    break;
                }
            }
            res = stream.read(&mut incoming_buf) => {
                match res {
                    Ok(0) | Err(_) => {
                        error!("Connection lost while waiting for user confirmation");
                        break;
                    }
                    _ => {} // Ignore unexpected data
                }
            }
        }
    }

    if success {
        info!("User confirmed pairing. Sending acceptance to initiator.");
        stream.write_all(&[1]).await?;
        let _ = store.add_paired_device(&remote_node_id, &initiator_label);
        let _ = ipc_tx.send(IpcMessage::PairingResult { success: true, node_id: remote_node_id, label: initiator_label });
    } else {
        info!("User rejected pairing. Sending rejection to initiator.");
        let _ = stream.write_all(&[0]).await;
    }

    // Clear state
    {
        let mut ap = active_pairing.lock().unwrap();
        *ap = None;
    }

    Ok(())
}

async fn handle_outgoing_pairing(mut stream: TcpStream, store: Arc<Store>, ipc_tx: Sender<IpcMessage>, priv_key: Vec<u8>, active_pairing: Arc<Mutex<Option<ActivePairingState>>>) -> Result<(), Box<dyn std::error::Error>> {
    info!("Initiating outgoing pairing request");

    let self_label = store.get_state("device_name")?.unwrap_or_else(|| "Unknown Device".to_string());

    let params: NoiseParams = "Noise_XX_25519_ChaChaPoly_BLAKE2s".parse()?;
    let mut builder = Builder::new(params);
    builder = builder.local_private_key(&priv_key);
    let mut noise = builder.build_initiator()?;

    let mut buf = [0u8; 1024];

    // 1. Write e
    let n = noise.write_message(&[], &mut buf)?;
    stream.write_all(&buf[..n]).await?;

    // 2. Read e, ee, s, es + Responder's label
    let mut responder_label_buf = [0u8; 1024];
    let n = stream.read(&mut buf).await?;
    let label_len = noise.read_message(&buf[..n], &mut responder_label_buf)?;
    let responder_label = String::from_utf8_lossy(&responder_label_buf[..label_len]).to_string();

    // 3. Write s, se + Initiator's label
    let n = noise.write_message(self_label.as_bytes(), &mut buf)?;
    stream.write_all(&buf[..n]).await?;

    // Handshake finished. Derive PIN.
    let h = noise.get_handshake_hash();
    let pin = derive_pin(h);
    let remote_node_id = hex::encode(noise.get_remote_static().unwrap());
    
    info!("Outgoing pairing PIN: {} for {} ({})", pin, responder_label, remote_node_id);

    // Update state
    let confirmed = Arc::new(Mutex::new(None::<bool>));
    {
        let mut ap = active_pairing.lock().unwrap();
        *ap = Some(ActivePairingState {
            pin: pin.clone(),
            is_initiator: true,
            remote_id: remote_node_id.clone(),
            remote_label: responder_label.clone(),
            confirmed: Arc::clone(&confirmed),
        });
    }

    // Initiator waits for remote to confirm or local to cancel
    let mut success = false;
    let mut result_buf = [0u8; 1];
    
    // We use tokio::select! to wait for either the stream result or a local cancel
    loop {
        tokio::select! {
            res = stream.read_exact(&mut result_buf) => {
                match res {
                    Ok(_) => {
                        success = result_buf[0] == 1;
                        break;
                    }
                    Err(_) => {
                        error!("Connection lost while waiting for pairing confirmation");
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                let res = confirmed.lock().unwrap();
                if let Some(accepted) = *res {
                    if !accepted {
                        info!("User cancelled pairing locally.");
                        break;
                    }
                }
            }
        }
    }

    if success {
        info!("Pairing confirmed by remote.");
        let _ = store.add_paired_device(&remote_node_id, &responder_label);
        let _ = ipc_tx.send(IpcMessage::PairingResult { success: true, node_id: remote_node_id, label: responder_label });
    } else {
        info!("Pairing failed or rejected.");
        let _ = ipc_tx.send(IpcMessage::PairingResult { success: false, node_id: remote_node_id, label: responder_label });
    }

    // Clear state
    {
        let mut ap = active_pairing.lock().unwrap();
        *ap = None;
    }

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
