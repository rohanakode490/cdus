use crate::relay::TurnCredentials;
use anyhow::Result;
use flume::{Receiver, Sender};
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread;
use tokio::net::UdpSocket;
use tokio::runtime::Runtime;
use tracing::{error, info, warn};
use turn::client::{Client, ClientConfig};
use webrtc_util::conn::Conn;

pub struct TurnConnection {
    pub local_relayed_addr: SocketAddr,
    pub tx: Sender<Vec<u8>>,   // Send to this to send over TURN
    pub rx: Receiver<Vec<u8>>, // Receive from this to get data from TURN
}

pub struct TurnManager {
    runtime: Arc<Runtime>,
}

impl TurnManager {
    pub fn new() -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        Ok(Self {
            runtime: Arc::new(runtime),
        })
    }

    pub fn start_session(
        &self,
        creds: TurnCredentials,
        remote_addr: Option<SocketAddr>, // Peer's relayed address (if known)
    ) -> Result<(TurnConnection, thread::JoinHandle<()>)> {
        let (to_turn_tx, to_turn_rx) = flume::unbounded::<Vec<u8>>();
        let (from_turn_tx, from_turn_rx) = flume::unbounded::<Vec<u8>>();

        let runtime = Arc::clone(&self.runtime);
        let (ready_tx, ready_rx) = flume::bounded::<SocketAddr>(1);

        let handle = thread::spawn(move || {
            runtime.block_on(async {
                let socket = match UdpSocket::bind("0.0.0.0:0").await {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Failed to bind UDP socket for TURN: {}", e);
                        return;
                    }
                };

                // Simple parsing of turn:host:port
                let server_addr = creds.urls[0].replace("turn:", "");

                let config = ClientConfig {
                    stun_serv_addr: server_addr.clone(),
                    turn_serv_addr: server_addr,
                    username: creds.username,
                    password: creds.password,
                    realm: "".to_string(),
                    software: "".to_string(),
                    rto_in_ms: 100,
                    conn: Arc::new(socket),
                    vnet: None,
                };

                let client = match Client::new(config).await {
                    Ok(c) => c,
                    Err(e) => {
                        error!("Failed to create TURN client: {}", e);
                        return;
                    }
                };

                if let Err(e) = client.listen().await {
                    error!("TURN client listen failed: {}", e);
                    return;
                }

                let allocation = match client.allocate().await {
                    Ok(a) => a,
                    Err(e) => {
                        error!("TURN allocation failed: {}", e);
                        return;
                    }
                };

                let relayed_addr = match allocation.local_addr() {
                    Ok(addr) => addr,
                    Err(e) => {
                        error!("Failed to get local relayed addr: {}", e);
                        return;
                    }
                };
                info!("TURN allocation successful: {}", relayed_addr);

                // Notify caller that we are ready
                let _ = ready_tx.send(relayed_addr);

                let mut buf = [0u8; 2048];
                let mut current_remote_addr = remote_addr;

                loop {
                    tokio::select! {
                        // Incoming from TURN
                        res = allocation.recv_from(&mut buf) => {
                            match res {
                                Ok((n, addr)) => {
                                    if current_remote_addr.is_none() {
                                        info!("Received first TURN message from {}, locking session to this peer", addr);
                                        current_remote_addr = Some(addr);
                                    }
                                    let _ = from_turn_tx.send(buf[..n].to_vec());
                                }
                                Err(e) => {
                                    error!("TURN recv_from error: {}", e);
                                    break;
                                }
                            }
                        }
                        // Outgoing to TURN
                        msg = to_turn_rx.recv_async() => {
                            match msg {
                                Ok(data) => {
                                    if let Some(peer_addr) = current_remote_addr {
                                        // send_to handles permission creation automatically in webrtc-turn
                                        if let Err(e) = allocation.send_to(&data, peer_addr).await {
                                            error!("TURN send_to {} failed: {}", peer_addr, e);
                                        }
                                    } else {
                                        warn!("No remote address set for TURN session, dropping outgoing message");
                                    }
                                }
                                Err(_) => break, // Channel closed
                            }
                        }
                    }
                }
                let _ = client.close().await;
                info!("TURN session task exiting");
            });
        });

        // Wait for allocation to be ready
        match ready_rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(relayed_addr) => Ok((
                TurnConnection {
                    local_relayed_addr: relayed_addr,
                    tx: to_turn_tx,
                    rx: from_turn_rx,
                },
                handle,
            )),
            Err(_) => Err(anyhow::anyhow!("TURN allocation timed out")),
        }
    }
}
