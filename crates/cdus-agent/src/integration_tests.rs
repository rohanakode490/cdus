#[cfg(test)]
mod tests {
    use crate::pairing::{PairingManager, SyncManager};
    use crate::relay::RelayManager;
    use crate::store::Store;
    use crate::turn_manager::TurnManager;
    use crate::ActivePairingState;
    use base64::Engine;
    use cdus_common::{IpcMessage, SyncMessage};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn test_mutual_pairing_and_clipboard_sync() {
        let _ = tracing_subscriber::fmt::try_init();
        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();

        let store1 = Arc::new(Store::init(dir1.path()).unwrap());
        let store2 = Arc::new(Store::init(dir2.path()).unwrap());

        let (tx1, rx1) = flume::unbounded();
        let (tx2, rx2) = flume::unbounded();

        let ap1 = Arc::new(Mutex::new(None::<ActivePairingState>));
        let ap2 = Arc::new(Mutex::new(None::<ActivePairingState>));

        let sm1 = Arc::new(SyncManager::new());
        let sm2 = Arc::new(SyncManager::new());

        let tm1 = Arc::new(TurnManager::new().unwrap());
        let tm2 = Arc::new(TurnManager::new().unwrap());

        let (id1, priv1) = store1.get_or_create_identity(dir1.path()).unwrap();
        let (id2, priv2) = store2.get_or_create_identity(dir2.path()).unwrap();

        let (relay1, _) =
            RelayManager::new(id1.clone(), "http://localhost".to_string(), tx1.clone());
        let (relay2, _) =
            RelayManager::new(id2.clone(), "http://localhost".to_string(), tx2.clone());

        let pm1 = PairingManager::new(
            Arc::clone(&store1),
            tx1.clone(),
            id1.clone(),
            priv1,
            5201,
            Arc::clone(&ap1),
            Arc::clone(&sm1),
            Arc::new(relay1),
            tm1,
        );
        let pm2 = PairingManager::new(
            Arc::clone(&store2),
            tx2.clone(),
            id2.clone(),
            priv2,
            5202,
            Arc::clone(&ap2),
            Arc::clone(&sm2),
            Arc::new(relay2),
            tm2,
        );

        let pm1 = Arc::new(pm1);
        let pm2 = Arc::new(pm2);

        // Start listeners
        let pm1_c = Arc::clone(&pm1);
        thread::spawn(move || pm1_c.start_listener());
        let pm2_c = Arc::clone(&pm2);
        thread::spawn(move || pm2_c.start_listener());

        thread::sleep(Duration::from_millis(100));

        // Initiate pairing from pm1 to pm2
        let pm1_init = Arc::clone(&pm1);
        thread::spawn(move || {
            pm1_init.initiate_pairing("127.0.0.1:5202".parse().unwrap());
        });

        // Wait for both to see active pairing
        let mut attempts = 0;
        while attempts < 50 {
            let p1 = ap1.lock().unwrap().is_some();
            let p2 = ap2.lock().unwrap().is_some();
            if p1 && p2 {
                break;
            }
            thread::sleep(Duration::from_millis(200));
            attempts += 1;
        }

        assert!(
            ap1.lock().unwrap().is_some(),
            "Initiator should have active pairing"
        );
        assert!(
            ap2.lock().unwrap().is_some(),
            "Responder should have active pairing"
        );

        // Confirm on both sides
        {
            let s1 = ap1.lock().unwrap();
            let mut res1 = s1.as_ref().unwrap().confirmed.lock().unwrap();
            *res1 = Some(true);
        }
        {
            let s2 = ap2.lock().unwrap();
            let mut res2 = s2.as_ref().unwrap().confirmed.lock().unwrap();
            *res2 = Some(true);
        }

        // Wait for pairing result messages
        let mut p1_success = false;
        let mut p2_success = false;

        for _ in 0..20 {
            while let Ok(msg) = rx1.try_recv() {
                if let IpcMessage::PairingResult { success, .. } = msg {
                    p1_success = success;
                }
            }
            while let Ok(msg) = rx2.try_recv() {
                if let IpcMessage::PairingResult { success, .. } = msg {
                    p2_success = success;
                }
            }
            if p1_success && p2_success {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        assert!(p1_success, "Initiator pairing failed");
        assert!(p2_success, "Responder pairing failed");

        // Verify they are connected in SyncManager
        assert!(sm1.is_connected(&id2), "SM1 should be connected to Node 2");
        assert!(sm2.is_connected(&id1), "SM2 should be connected to Node 1");

        // Test Clipboard Sync
        let test_content = "Hello from Node 1".to_string();
        let test_ts = 123456789u64;

        sm1.broadcast(SyncMessage::ClipboardUpdate {
            content: test_content.clone(),
            timestamp: test_ts,
        });

        // Wait for rx2 to receive SetClipboard
        let mut received_content = String::new();
        for _ in 0..20 {
            while let Ok(msg) = rx2.try_recv() {
                if let IpcMessage::SetClipboard { content, .. } = msg {
                    received_content = content;
                }
            }
            if !received_content.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        assert_eq!(
            received_content, test_content,
            "Node 2 did not receive the correct clipboard content"
        );
    }

    #[test]
    fn test_lww_conflict_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::init(dir.path()).unwrap());
        let (tx, rx) = flume::unbounded();
        let lw = Arc::new(Mutex::new(None));
        let dd = Arc::new(Mutex::new(Vec::new()));
        let ap = Arc::new(Mutex::new(None));
        let sm = Arc::new(SyncManager::new());
        let tm = Arc::new(TurnManager::new().unwrap());
        let (relay, _) = RelayManager::new(
            "test".to_string(),
            "http://localhost".to_string(),
            tx.clone(),
        );
        let pm = Arc::new(PairingManager::new(
            Arc::clone(&store),
            tx.clone(),
            "test".to_string(),
            vec![],
            0,
            Arc::clone(&ap),
            Arc::clone(&sm),
            Arc::new(relay),
            tm,
        ));
        let lpt = Arc::new(Mutex::new(0u64));

        // Initial state
        let ts1 = 1000u64;
        tx.send(IpcMessage::SetClipboard {
            content: "Initial".to_string(),
            timestamp: ts1,
            source: "Remote".to_string(),
        })
        .unwrap();

        // Run daemon loop for a few iterations
        let tx_daemon = tx.clone();
        let store_daemon = Arc::clone(&store);
        let lw_daemon = Arc::clone(&lw);
        let dd_daemon = Arc::clone(&dd);
        let ap_daemon = Arc::clone(&ap);
        let sm_daemon = Arc::clone(&sm);
        let pm_daemon = Arc::clone(&pm);
        let lpt_daemon = Arc::clone(&lpt);

        let at = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let rm = Arc::new(Mutex::new(std::collections::HashMap::new()));
        // We'll run it manually to control messages
        crate::daemon_loop(
            tx_daemon,
            rx,
            Some(5),
            store_daemon,
            lw_daemon,
            dd_daemon,
            ap_daemon,
            sm_daemon,
            pm_daemon,
            lpt_daemon,
            None,
            at.clone(),
            rm.clone(),
        );

        // 1. Send older message (should be ignored)
        let (tx2, rx2) = flume::unbounded();
        tx2.send(IpcMessage::SetClipboard {
            content: "Older".to_string(),
            timestamp: ts1 - 10,
            source: "Remote".to_string(),
        })
        .unwrap();

        // Reset lw for check
        *lw.lock().unwrap() = None;
        crate::daemon_loop(
            tx.clone(),
            rx2,
            Some(5),
            Arc::clone(&store),
            Arc::clone(&lw),
            Arc::clone(&dd),
            Arc::clone(&ap),
            Arc::clone(&sm),
            Arc::clone(&pm),
            Arc::clone(&lpt),
            None,
            at.clone(),
            rm.clone(),
        );
        assert_eq!(
            *lw.lock().unwrap(),
            None,
            "Older message should not have been written to clipboard"
        );

        // 2. Send newer message (should be accepted)
        let (tx3, rx3) = flume::unbounded();
        let ts2 = ts1 + 100;
        tx3.send(IpcMessage::SetClipboard {
            content: "Newer".to_string(),
            timestamp: ts2,
            source: "Remote".to_string(),
        })
        .unwrap();

        crate::daemon_loop(
            tx.clone(),
            rx3,
            Some(5),
            Arc::clone(&store),
            Arc::clone(&lw),
            Arc::clone(&dd),
            Arc::clone(&ap),
            Arc::clone(&sm),
            Arc::clone(&pm),
            Arc::clone(&lpt),
            None,
            at.clone(),
            rm.clone(),
        );
        assert_eq!(
            *lw.lock().unwrap(),
            Some("Newer".to_string()),
            "Newer message should have been written to clipboard"
        );
    }

    #[test]
    fn test_pairing_rejection() {
        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();
        let store1 = Arc::new(Store::init(dir1.path()).unwrap());
        let store2 = Arc::new(Store::init(dir2.path()).unwrap());
        let (tx1, _) = flume::unbounded();
        let (tx2, rx2) = flume::unbounded();
        let ap1 = Arc::new(Mutex::new(None));
        let ap2 = Arc::new(Mutex::new(None));
        let sm1 = Arc::new(SyncManager::new());
        let sm2 = Arc::new(SyncManager::new());
        let (id1, priv1) = store1.get_or_create_identity(dir1.path()).unwrap();
        let (id2, priv2) = store2.get_or_create_identity(dir2.path()).unwrap();

        let tm1 = Arc::new(TurnManager::new().unwrap());
        let tm2 = Arc::new(TurnManager::new().unwrap());

        let (relay1, _) =
            RelayManager::new(id1.clone(), "http://localhost".to_string(), tx1.clone());
        let (relay2, _) =
            RelayManager::new(id2.clone(), "http://localhost".to_string(), tx2.clone());

        let pm1 = Arc::new(PairingManager::new(
            Arc::clone(&store1),
            tx1,
            id1,
            priv1,
            5301,
            Arc::clone(&ap1),
            Arc::clone(&sm1),
            Arc::new(relay1),
            tm1,
        ));
        let pm2 = Arc::new(PairingManager::new(
            Arc::clone(&store2),
            tx2,
            id2,
            priv2,
            5302,
            Arc::clone(&ap2),
            Arc::clone(&sm2),
            Arc::new(relay2),
            tm2,
        ));

        let pm2_c = Arc::clone(&pm2);
        thread::spawn(move || pm2_c.start_listener());
        thread::sleep(Duration::from_millis(50));

        thread::spawn(move || pm1.initiate_pairing("127.0.0.1:5302".parse().unwrap()));

        // Wait for responder to see pairing
        let mut attempts = 0;
        while attempts < 10 && ap2.lock().unwrap().is_none() {
            thread::sleep(Duration::from_millis(100));
            attempts += 1;
        }

        // Responder rejects
        {
            let s2 = ap2.lock().unwrap();
            let mut res2 = s2.as_ref().unwrap().confirmed.lock().unwrap();
            *res2 = Some(false);
        }

        // Wait for result
        let mut p2_success = true;
        for _ in 0..10 {
            while let Ok(msg) = rx2.try_recv() {
                if let IpcMessage::PairingResult { success, .. } = msg {
                    p2_success = success;
                }
            }
            if !p2_success {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }

        assert!(!p2_success, "Pairing should have failed after rejection");
    }

    #[test]
    fn test_self_pairing_prevention() {
        let dir = tempdir().unwrap();
        let store = Arc::new(Store::init(dir.path()).unwrap());
        let (tx, _rx) = flume::unbounded();
        let (id, priv_key) = store.get_or_create_identity(dir.path()).unwrap();
        let ap = Arc::new(Mutex::new(None));
        let sm = Arc::new(SyncManager::new());
        let tm = Arc::new(TurnManager::new().unwrap());
        let (relay, _) = RelayManager::new(id.clone(), "http://localhost".to_string(), tx.clone());
        let pm = Arc::new(PairingManager::new(
            Arc::clone(&store),
            tx,
            id.clone(),
            priv_key,
            5401,
            Arc::clone(&ap),
            Arc::clone(&sm),
            Arc::new(relay),
            tm,
        ));

        let pm_c = Arc::clone(&pm);
        thread::spawn(move || pm_c.start_listener());
        thread::sleep(Duration::from_millis(50));

        // Attempt to connect to self
        let addr = "127.0.0.1:5401".parse().unwrap();
        pm.initiate_pairing(addr);

        thread::sleep(Duration::from_millis(200));
        // We expect it to fail gracefully and not set an active pairing forever
        assert!(
            ap.lock().unwrap().is_none(),
            "Active pairing should be None after self-connection attempt"
        );
    }

    #[test]
    fn test_malformed_handshake() {
        let dir = tempdir().unwrap();
        let store = Arc::new(Store::init(dir.path()).unwrap());
        let (tx, _rx) = flume::unbounded();
        let (id, priv_key) = store.get_or_create_identity(dir.path()).unwrap();
        let ap = Arc::new(Mutex::new(None));
        let sm = Arc::new(SyncManager::new());
        let tm = Arc::new(TurnManager::new().unwrap());
        let (relay, _) = RelayManager::new(id.clone(), "http://localhost".to_string(), tx.clone());
        let pm = PairingManager::new(
            Arc::clone(&store),
            tx,
            id,
            priv_key,
            5501,
            Arc::clone(&ap),
            Arc::clone(&sm),
            Arc::new(relay),
            tm,
        );

        thread::spawn(move || pm.start_listener());
        thread::sleep(Duration::from_millis(50));

        // Send garbage data
        use std::io::Write;
        let mut stream = std::net::TcpStream::connect("127.0.0.1:5501").unwrap();
        stream.write_all(b"NOT A NOISE MESSAGE").unwrap();

        thread::sleep(Duration::from_millis(100));
        assert!(
            ap.lock().unwrap().is_none(),
            "Should not crash or hang on malformed data"
        );
    }

    #[test]
    fn test_relay_remote_pairing() {
        use crate::relay::RelayManager;

        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();

        let store1 = Arc::new(Store::init(dir1.path()).unwrap());
        let store2 = Arc::new(Store::init(dir2.path()).unwrap());

        let (tx1, _) = flume::unbounded();
        let (tx2, _) = flume::unbounded();

        let ap1 = Arc::new(Mutex::new(None));
        let ap2 = Arc::new(Mutex::new(None));

        let sm1 = Arc::new(SyncManager::new());
        let sm2 = Arc::new(SyncManager::new());

        let (id1, priv1) = store1.get_or_create_identity(dir1.path()).unwrap();
        let (id2, priv2) = store2.get_or_create_identity(dir2.path()).unwrap();

        let tm1 = Arc::new(TurnManager::new().unwrap());
        let tm2 = Arc::new(TurnManager::new().unwrap());

        let (relay1, relay_rx1) =
            RelayManager::new(id1.clone(), "http://localhost".to_string(), tx1.clone());
        let (relay2, relay_rx2) =
            RelayManager::new(id2.clone(), "http://localhost".to_string(), tx2.clone());

        let relay1 = Arc::new(relay1);
        let relay2 = Arc::new(relay2);

        let pm1 = Arc::new(PairingManager::new(
            Arc::clone(&store1),
            tx1.clone(),
            id1.clone(),
            priv1,
            5601,
            Arc::clone(&ap1),
            Arc::clone(&sm1),
            Arc::clone(&relay1),
            tm1,
        ));

        let pm2 = Arc::new(PairingManager::new(
            Arc::clone(&store2),
            tx2.clone(),
            id2.clone(),
            priv2,
            5602,
            Arc::clone(&ap2),
            Arc::clone(&sm2),
            Arc::clone(&relay2),
            tm2,
        ));

        // Mock the relay server by cross-connecting the relay channels
        let pm1_c = Arc::clone(&pm1);
        let pm2_c = Arc::clone(&pm2);

        thread::spawn(move || {
            while let Ok(msg) = relay_rx1.recv() {
                pm2_c.handle_relay_message(
                    msg.source_uuid,
                    base64::engine::general_purpose::STANDARD
                        .decode(&msg.payload)
                        .unwrap(),
                );
            }
        });

        thread::spawn(move || {
            while let Ok(msg) = relay_rx2.recv() {
                pm1_c.handle_relay_message(
                    msg.source_uuid,
                    base64::engine::general_purpose::STANDARD
                        .decode(&msg.payload)
                        .unwrap(),
                );
            }
        });

        // Step 1: Initiate remote pairing from PM1 to PM2
        pm1.initiate_remote_pairing(id2.clone());

        // Wait for handshake to complete and PIN to be derived
        let mut attempts = 0;
        let mut pin1 = String::new();
        let mut pin2 = String::new();

        while attempts < 20 {
            {
                let s1 = ap1.lock().unwrap();
                if let Some(ref st) = *s1 {
                    pin1 = st.pin.clone();
                }
            }
            {
                let s2 = ap2.lock().unwrap();
                if let Some(ref st) = *s2 {
                    pin2 = st.pin.clone();
                }
            }
            if !pin1.is_empty() && !pin2.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(100));
            attempts += 1;
        }

        assert_eq!(pin1, pin2, "Pairing PINs should match");
        assert_eq!(pin1.len(), 4, "PIN should be 4 digits");

        // Verify labels were exchanged
        let label1 = store1.get_state("device_name").unwrap().unwrap();
        let label2 = store2.get_state("device_name").unwrap().unwrap();

        assert_eq!(ap1.lock().unwrap().as_ref().unwrap().remote_label, label2);
        assert_eq!(ap2.lock().unwrap().as_ref().unwrap().remote_label, label1);
    }

    #[test]
    fn test_file_transfer_integration() {
        let _ = tracing_subscriber::fmt::try_init();
        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();

        // Create a dummy file to transfer
        let file_path = dir1.path().join("test_file.bin");
        let mut file_content = vec![0u8; 1024 * 1024]; // 1MB for faster test
        for i in 0..file_content.len() {
            file_content[i] = (i % 256) as u8;
        }
        std::fs::write(&file_path, &file_content).unwrap();

        let store1 = Arc::new(Store::init(dir1.path()).unwrap());
        let store2 = Arc::new(Store::init(dir2.path()).unwrap());

        let (tx1, rx1) = flume::unbounded();
        let (tx2, rx2) = flume::unbounded();

        let (id1, _priv1) = store1.get_or_create_identity(dir1.path()).unwrap();
        let (id2, _priv2) = store2.get_or_create_identity(dir2.path()).unwrap();

        let at1 = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let rm1 = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let at2 = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let rm2 = Arc::new(Mutex::new(std::collections::HashMap::new()));

        let sm1 = Arc::new(SyncManager::new());
        let sm2 = Arc::new(SyncManager::new());

        // Mock peer connection: sm1 <-> sm2
        let (sync_tx1, sync_rx1) = flume::unbounded();
        let (sync_tx2, sync_rx2) = flume::unbounded();

        // sm1 sends to sync_tx2, which goes to Agent 2
        sm1.add_peer(id2.clone(), sync_tx2, cdus_common::TransportType::Lan);
        // sm2 sends to sync_tx1, which goes to Agent 1
        sm2.add_peer(id1.clone(), sync_tx1, cdus_common::TransportType::Lan);

        // Mock libp2p_request_tx
        let (req_tx1, req_rx1) = flume::unbounded();
        let (req_tx2, req_rx2) = flume::unbounded();

        // Start daemon loops
        let tx1_c = tx1.clone();
        let tx2_c = tx2.clone();
        let store1_c = Arc::clone(&store1);
        let sm1_c = Arc::clone(&sm1);
        let at1_c = Arc::clone(&at1);
        let rm1_c = Arc::clone(&rm1);
        let req_tx1_c = req_tx1.clone();
        let id1_c = id1.clone();

        thread::spawn(move || {
            loop {
                crate::daemon_loop(
                    tx1_c.clone(),
                    rx1.clone(),
                    Some(10), // More iterations
                    Arc::clone(&store1_c),
                    Arc::new(Mutex::new(None)),
                    Arc::new(Mutex::new(Vec::new())),
                    Arc::new(Mutex::new(None)),
                    Arc::clone(&sm1_c),
                    Arc::new(crate::pairing::PairingManager::new(
                        Arc::clone(&store1_c),
                        tx1_c.clone(),
                        id1_c.clone(),
                        vec![],
                        0,
                        Arc::new(Mutex::new(None)),
                        Arc::clone(&sm1_c),
                        Arc::new(
                            crate::relay::RelayManager::new(
                                id1_c.clone(),
                                "".to_string(),
                                tx1_c.clone(),
                            )
                            .0,
                        ),
                        Arc::new(crate::turn_manager::TurnManager::new().unwrap()),
                    )),
                    Arc::new(Mutex::new(0)),
                    Some(req_tx1_c.clone()),
                    Arc::clone(&at1_c),
                    Arc::clone(&rm1_c),
                );
                thread::sleep(Duration::from_millis(10));
            }
        });

        let store2_c = Arc::clone(&store2);
        let sm2_c = Arc::clone(&sm2);
        let at2_c = Arc::clone(&at2);
        let rm2_c = Arc::clone(&rm2);
        let req_tx2_c = req_tx2.clone();
        let id2_c = id2.clone();

        thread::spawn(move || loop {
            crate::daemon_loop(
                tx2_c.clone(),
                rx2.clone(),
                Some(10),
                Arc::clone(&store2_c),
                Arc::new(Mutex::new(None)),
                Arc::new(Mutex::new(Vec::new())),
                Arc::new(Mutex::new(None)),
                Arc::clone(&sm2_c),
                Arc::new(crate::pairing::PairingManager::new(
                    Arc::clone(&store2_c),
                    tx2_c.clone(),
                    id2_c.clone(),
                    vec![],
                    0,
                    Arc::new(Mutex::new(None)),
                    Arc::clone(&sm2_c),
                    Arc::new(
                        crate::relay::RelayManager::new(
                            id2_c.clone(),
                            "".to_string(),
                            tx2_c.clone(),
                        )
                        .0,
                    ),
                    Arc::new(crate::turn_manager::TurnManager::new().unwrap()),
                )),
                Arc::new(Mutex::new(0)),
                Some(req_tx2_c.clone()),
                Arc::clone(&at2_c),
                Arc::clone(&rm2_c),
            );
            thread::sleep(Duration::from_millis(10));
        });

        // Router thread to forward SyncMessages and ChunkRequests
        let id1_r = id1.clone();
        let tx1_r = tx1.clone();
        let tx2_r = tx2.clone();
        let at1_r = Arc::clone(&at1);
        thread::spawn(move || {
            loop {
                // Agent 1 -> Agent 2
                if let Ok(msg) = sync_rx2.try_recv() {
                    match msg {
                        SyncMessage::FileTransferRequest(m) => {
                            tx2_r
                                .send(IpcMessage::IncomingFileRequest {
                                    node_id: id1_r.clone(),
                                    manifest: m,
                                })
                                .unwrap();
                        }
                        _ => {}
                    }
                }
                // Agent 2 -> Agent 1
                if let Ok(msg) = sync_rx1.try_recv() {
                    match msg {
                        SyncMessage::FileTransferAccepted { file_hash } => {
                            tx1_r
                                .send(IpcMessage::AcceptFileTransfer { file_hash })
                                .unwrap();
                        }
                        _ => {}
                    }
                }
                // Chunk requests from Agent 2 to Agent 1
                if let Ok((_peer, msg)) = req_rx2.try_recv() {
                    match msg {
                        SyncMessage::ChunkRequest {
                            file_hash,
                            chunk_hash,
                        } => {
                            let info = at1_r.lock().unwrap().get(&file_hash).cloned();
                            if let Some((path, manifest)) = info {
                                if let Some(chunk) =
                                    manifest.chunks.iter().find(|c| c.hash == chunk_hash)
                                {
                                    let data = crate::file_transfer::get_chunk(
                                        &path,
                                        chunk.offset,
                                        chunk.size,
                                    )
                                    .unwrap();
                                    tx2_r
                                        .send(IpcMessage::ChunkReceived {
                                            file_hash,
                                            chunk_hash,
                                            data,
                                        })
                                        .unwrap();
                                }
                            }
                        }
                        _ => {}
                    }
                }
                thread::sleep(Duration::from_millis(5));
            }
        });

        // Step 1: Initiate transfer from Agent 1
        tx1.send(IpcMessage::SendFile {
            node_id: id2.clone(),
            path: file_path.to_str().unwrap().to_string(),
        })
        .unwrap();

        // Step 2: Wait for Agent 2 to receive IncomingFileRequest
        let mut file_hash = String::new();
        let mut attempts = 0;
        while attempts < 100 {
            let rm = rm2.lock().unwrap();
            if let Some(p) = rm.values().next() {
                file_hash = p.manifest.file_hash.clone();
                break;
            }
            drop(rm);
            thread::sleep(Duration::from_millis(100));
            attempts += 1;
        }
        assert!(
            !file_hash.is_empty(),
            "Agent 2 should have received manifest"
        );

        // Step 3: Agent 2 accepts transfer
        tx2.send(IpcMessage::AcceptFileTransfer {
            file_hash: file_hash.clone(),
        })
        .unwrap();

        // Step 4: Wait for completion
        attempts = 0;
        let mut completed = false;
        while attempts < 200 {
            let mut final_path = std::env::current_dir().unwrap();
            final_path.push("test_file.bin");
            if final_path.exists() {
                let saved_content = std::fs::read(&final_path).unwrap();
                if saved_content == file_content {
                    completed = true;
                    let _ = std::fs::remove_file(final_path);
                    break;
                }
            }
            thread::sleep(Duration::from_millis(100));
            attempts += 1;
        }

        assert!(
            completed,
            "File transfer should have completed successfully and matched content"
        );
    }
}
