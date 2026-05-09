use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use std::thread;
use std::time::Duration;
use tracing::{error, info};
use tungstenite::{connect, Message};
use ureq;

#[derive(Serialize)]
struct RegisterRequest {
    uuid: String,
    public_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SignalMessage {
    pub source_uuid: String,
    pub target_uuid: String,
    pub payload: String, // Base64 encoded from Go
}

pub struct RelayManager {
    node_id: String,
    relay_url: String,
}

impl RelayManager {
    pub fn new(node_id: String, relay_url: String) -> Self {
        Self {
            node_id,
            relay_url,
        }
    }

    pub fn register(&self) -> Result<()> {
        let url = format!("{}/v1/register", self.relay_url);
        let req = RegisterRequest {
            uuid: self.node_id.clone(),
            public_key: self.node_id.clone(),
        };

        info!("Registering device with relay at {}...", url);
        let resp = ureq::post(&url)
            .send_json(&req)?;

        if resp.status() == 201 || resp.status() == 200 {
            info!("Device registered successfully.");
            Ok(())
        } else {
            let err_msg = resp.into_string().unwrap_or_else(|_| "Unknown error".to_string());
            error!("Failed to register device: {}", err_msg);
            Err(anyhow::anyhow!("Registration failed: {}", err_msg))
        }
    }

    pub fn start_signaling_loop(&self) {
        let ws_url = self.relay_url.replace("http", "ws") + "/v1/signaling?uuid=" + &self.node_id;

        thread::spawn(move || {
            loop {
                info!("Connecting to relay signaling at {}...", ws_url);
                match connect(&ws_url) {
                    Ok((mut socket, _response)) => {
                        info!("Connected to relay signaling.");
                        loop {
                            match socket.read() {
                                Ok(msg) => {
                                    let maybe_signal = match msg {
                                        Message::Binary(data) => serde_json::from_slice::<SignalMessage>(&data).ok(),
                                        Message::Text(text) => serde_json::from_str::<SignalMessage>(&text).ok(),
                                        _ => None,
                                    };

                                    if let Some(signal) = maybe_signal {
                                        match general_purpose::STANDARD.decode(&signal.payload) {
                                            Ok(decoded_payload) => {
                                                info!("Received signaling message from {} ({} bytes)", signal.source_uuid, decoded_payload.len());
                                                // TODO: Route decoded_payload to PairingManager/SyncManager
                                            }
                                            Err(e) => error!("Failed to decode base64 payload from {}: {}", signal.source_uuid, e),
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("Relay signaling read error: {}", e);
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to connect to relay signaling: {}. Retrying in 10s...", e);
                    }
                }
                thread::sleep(Duration::from_secs(10));
            }
        });
    }
}
