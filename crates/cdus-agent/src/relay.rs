use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use std::thread;
use std::time::Duration;
use tracing::{error, info};
use tungstenite::{connect, Message, stream::MaybeTlsStream};
use ureq;
use flume::{Sender, Receiver};
use cdus_common::IpcMessage;

#[derive(Serialize)]
struct RegisterRequest {
    uuid: String,
    public_key: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SignalMessage {
    pub source_uuid: String,
    pub target_uuid: String,
    pub payload: String, // Base64 encoded from Go
}

#[derive(Deserialize, Debug, Clone)]
pub struct TurnCredentials {
    pub username: String,
    pub password: String,
    pub urls: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum RelayIncomingMessage {
    Signal(SignalMessage),
    Revocation(RevocationEvent),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RevocationEvent {
    pub revoked_uuid: String,
}

pub struct RelayManager {
    node_id: String,
    relay_url: String,
    tx: Sender<IpcMessage>,
    outgoing_tx: Sender<SignalMessage>,
}

impl RelayManager {
    pub fn new(node_id: String, relay_url: String, tx: Sender<IpcMessage>) -> (Self, Receiver<SignalMessage>) {
        let (outgoing_tx, outgoing_rx) = flume::unbounded();
        (Self {
            node_id,
            relay_url,
            tx,
            outgoing_tx,
        }, outgoing_rx)
    }

    pub fn get_turn_credentials(&self) -> Result<TurnCredentials> {
        let url = format!("{}/v1/turn?uuid={}", self.relay_url, self.node_id);
        info!("Fetching TURN credentials from relay at {}...", url);

        let resp = ureq::get(&url)
            .call()?;

        if resp.status() == 200 {
            let creds: TurnCredentials = resp.into_json()?;
            Ok(creds)
        } else {
            let err_msg = resp.into_string().unwrap_or_else(|_| "Unknown error".to_string());
            error!("Failed to fetch TURN credentials: {}", err_msg);
            Err(anyhow::anyhow!("Failed to fetch TURN credentials: {}", err_msg))
        }
    }

    pub fn send_signal(&self, target_uuid: String, payload: Vec<u8>) -> Result<()> {
        let b64_payload = general_purpose::STANDARD.encode(payload);
        let msg = SignalMessage {
            source_uuid: self.node_id.clone(),
            target_uuid,
            payload: b64_payload,
        };
        self.outgoing_tx.send(msg).map_err(|e| anyhow::anyhow!("Failed to queue signaling message: {}", e))
    }

    #[tracing::instrument(skip(self))]
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

    #[tracing::instrument(skip(self))]
    pub fn revoke_device(&self, uuid: String) -> Result<()> {
        let url = format!("{}/v1/revoke", self.relay_url);
        let req = serde_json::json!({ "uuid": uuid });

        info!("Revoking device {} with relay at {}...", uuid, url);
        let resp = ureq::post(&url)
            .send_json(&req)?;

        if resp.status() == 200 {
            info!("Device {} revoked successfully.", uuid);
            Ok(())
        } else {
            let err_msg = resp.into_string().unwrap_or_else(|_| "Unknown error".to_string());
            error!("Failed to revoke device: {}", err_msg);
            Err(anyhow::anyhow!("Revocation failed: {}", err_msg))
        }
    }

    pub fn start_signaling_loop(&self, outgoing_rx: Receiver<SignalMessage>) {
        let ws_url = self.relay_url.replace("http", "ws") + "/v1/signaling?uuid=" + &self.node_id;
        let tx = self.tx.clone();

        info!("Starting background relay signaling loop for {}", ws_url);

        thread::spawn(move || {
            loop {
                info!("Relay: Attempting connection to {}...", ws_url);
                match connect(&ws_url) {
                    Ok((mut socket, _response)) => {
                        info!("Relay: Connected successfully.");
                        
                        // Set read timeout
                        if let MaybeTlsStream::Plain(s) = socket.get_mut() {
                            let _ = s.set_read_timeout(Some(Duration::from_millis(100)));
                        }

                        loop {
                            // 1. Check for incoming messages
                            match socket.read() {
                                Ok(msg) => {
                                    let data = match msg {
                                        Message::Binary(data) => Some(data),
                                        Message::Text(text) => Some(text.into_bytes()),
                                        _ => None,
                                    };

                                    if let Some(data) = data {
                                        if let Ok(incoming) = serde_json::from_slice::<RelayIncomingMessage>(&data) {
                                            match incoming {
                                                RelayIncomingMessage::Signal(signal) => {
                                                    match general_purpose::STANDARD.decode(&signal.payload) {
                                                        Ok(decoded_payload) => {
                                                            info!("Relay: Received message from {} ({} bytes)", signal.source_uuid, decoded_payload.len());
                                                            let _ = tx.send(IpcMessage::RelayMessage { 
                                                                source_uuid: signal.source_uuid, 
                                                                payload: decoded_payload 
                                                            });
                                                        }
                                                        Err(e) => error!("Relay: Failed to decode base64 from {}: {}", signal.source_uuid, e),
                                                    }
                                                }
                                                RelayIncomingMessage::Revocation(rev) => {
                                                    info!("Relay: Received revocation for {}", rev.revoked_uuid);
                                                    let _ = tx.send(IpcMessage::RevokeDevice { uuid: rev.revoked_uuid });
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    if let tungstenite::Error::Io(io_err) = &e {
                                        if io_err.kind() == std::io::ErrorKind::WouldBlock || io_err.kind() == std::io::ErrorKind::TimedOut {
                                            // Normal timeout, continue
                                        } else {
                                            error!("Relay: Connection lost (IO error: {})", e);
                                            break;
                                        }
                                    } else {
                                        error!("Relay: WebSocket error: {}", e);
                                        break;
                                    }
                                }
                            }

                            // 2. Check for outgoing messages
                            while let Ok(msg) = outgoing_rx.try_recv() {
                                if let Ok(json_msg) = serde_json::to_string(&msg) {
                                    if let Err(e) = socket.send(Message::Text(json_msg)) {
                                        error!("Relay: Failed to send message: {}", e);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Relay: Connection failed: {}. Retrying in 10s. (Note: Relay is optional for LAN discovery)", e);
                    }
                }
                thread::sleep(Duration::from_secs(10));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flume;
    use httpmock::prelude::*;

    #[test]
    fn test_get_turn_credentials() {
        let server = MockServer::start();
        let node_id = "test-node".to_string();
        let (tx, _) = flume::unbounded();
        
        let relay = RelayManager {
            node_id: node_id.clone(),
            relay_url: server.base_url(),
            tx,
            outgoing_tx: flume::unbounded().0,
        };

        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/v1/turn")
                .query_param("uuid", &node_id);
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"username":"user","password":"pass","urls":["turn:localhost:3478"]}"#);
        });

        let creds = relay.get_turn_credentials().unwrap();
        mock.assert();
        
        assert_eq!(creds.username, "user");
        assert_eq!(creds.password, "pass");
        assert_eq!(creds.urls[0], "turn:localhost:3478");
    }

    #[test]
    fn test_relay_incoming_message_deser() {
        // Test SignalMessage
        let signal_json = r#"{"source_uuid":"src","target_uuid":"dst","payload":"SGVsbG8="}"#;
        let msg: RelayIncomingMessage = serde_json::from_str(signal_json).unwrap();
        match msg {
            RelayIncomingMessage::Signal(s) => {
                assert_eq!(s.source_uuid, "src");
                assert_eq!(s.payload, "SGVsbG8=");
            }
            _ => panic!("Expected Signal"),
        }

        // Test RevocationEvent
        let rev_json = r#"{"revoked_uuid":"bad-node"}"#;
        let msg: RelayIncomingMessage = serde_json::from_str(rev_json).unwrap();
        match msg {
            RelayIncomingMessage::Revocation(r) => {
                assert_eq!(r.revoked_uuid, "bad-node");
            }
            _ => panic!("Expected Revocation"),
        }
    }

    #[test]
    fn test_revoke_device() {
        let server = MockServer::start();
        let (tx, _) = flume::unbounded();
        let relay = RelayManager {
            node_id: "me".to_string(),
            relay_url: server.base_url(),
            tx,
            outgoing_tx: flume::unbounded().0,
        };

        let mock = server.mock(|when, then| {
            when.method(POST)
                .path("/v1/revoke")
                .json_body(serde_json::json!({ "uuid": "target-node" }));
            then.status(200);
        });

        relay.revoke_device("target-node".to_string()).unwrap();
        mock.assert();
    }
}
