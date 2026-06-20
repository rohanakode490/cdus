#[cfg(test)]
mod tests {
    use crate::pairing::{ActivePairingState, PairingManager, SyncManager};
    use crate::relay::RelayManager;
    use crate::store::Store;
    use crate::turn_manager::TurnManager;
    use crate::{daemon_loop, EVENT_BUS};
    use cdus_common::{IpcMessage, SyncMessage, TransportType, ProgressEvent};
    use std::collections::HashMap;
    use std::sync::Arc; use parking_lot::Mutex;
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;
    use crate::libp2p_manager::Libp2pManager;
    use crate::file_transfer::FileTransferManager;
    use std::net::SocketAddr;

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

        let ftm1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), flume::unbounded().0));
        let lm1 = Arc::new(Libp2pManager::new(vec![0u8; 32], tx1.clone(), Arc::clone(&store1), ftm1).unwrap());
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
            lm1,
        );
        let ftm2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), flume::unbounded().0));
        let lm2 = Arc::new(Libp2pManager::new(vec![0u8; 32], tx2.clone(), Arc::clone(&store2), ftm2).unwrap());
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
            lm2,
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
            pm1_init.initiate_pairing("127.0.0.1:5202".parse().unwrap(), None);
        });

        // Wait for both to see active pairing
        let mut attempts = 0;
        while attempts < 50 {
            let p1 = ap1.lock().is_some();
            let p2 = ap2.lock().is_some();
            if p1 && p2 {
                break;
            }
            thread::sleep(Duration::from_millis(200));
            attempts += 1;
        }

        assert!(
            ap1.lock().is_some(),
            "Initiator should have active pairing"
        );
        assert!(
            ap2.lock().is_some(),
            "Responder should have active pairing"
        );

        // Confirm on both sides
        {
            let s1 = ap1.lock();
            let mut res1 = s1.as_ref().unwrap().confirmed.lock();
            *res1 = Some(true);
        }
        {
            let s2 = ap2.lock();
            let mut res2 = s2.as_ref().unwrap().confirmed.lock();
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
        let peer_map = Arc::new(Mutex::new(HashMap::new()));

        let (p_tx, _p_rx) = flume::unbounded();
        let ftm = Arc::new(FileTransferManager::new(Arc::clone(&store), p_tx));
        let lm = Arc::new(Libp2pManager::new(vec![0u8; 32], tx.clone(), Arc::clone(&store), ftm).unwrap());

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
            lm.clone(),
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

        // We'll run it manually to control messages
        daemon_loop(
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
            peer_map.clone(),
            None,
            lm.clone(),
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
        *lw.lock() = None;
        daemon_loop(
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
            peer_map.clone(),
            None,
            lm.clone(),
        );
        assert_eq!(
            *lw.lock(),
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

        daemon_loop(
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
            peer_map.clone(),
            None,
            lm.clone(),
        );
        assert_eq!(
            *lw.lock(),
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

        let ftm1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), flume::unbounded().0));
        let lm1 = Arc::new(Libp2pManager::new(vec![0u8; 32], tx1.clone(), Arc::clone(&store1), ftm1).unwrap());
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
            lm1,
        ));
        let ftm2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), flume::unbounded().0));
        let lm2 = Arc::new(Libp2pManager::new(vec![0u8; 32], tx2.clone(), Arc::clone(&store2), ftm2).unwrap());
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
            lm2,
        ));

        let pm2_c = Arc::clone(&pm2);
        thread::spawn(move || pm2_c.start_listener());
        thread::sleep(Duration::from_millis(50));

        thread::spawn(move || pm1.initiate_pairing("127.0.0.1:5302".parse().unwrap(), None));

        // Wait for responder to see pairing
        let mut attempts = 0;
        while attempts < 10 && ap2.lock().is_none() {
            thread::sleep(Duration::from_millis(100));
            attempts += 1;
        }

        // Responder rejects
        {
            let s2 = ap2.lock();
            let mut res2 = s2.as_ref().unwrap().confirmed.lock();
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
        let ftm = Arc::new(FileTransferManager::new(Arc::clone(&store), flume::unbounded().0));
        let lm = Arc::new(Libp2pManager::new(vec![0u8; 32], tx.clone(), Arc::clone(&store), ftm).unwrap());
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
            lm.clone(),
        ));

        let pm_c = Arc::clone(&pm);
        thread::spawn(move || pm_c.start_listener());
        thread::sleep(Duration::from_millis(50));

        // Attempt to connect to self
        let addr = "127.0.0.1:5401".parse().unwrap();
        pm.initiate_pairing(addr, None);

        thread::sleep(Duration::from_millis(200));
        // We expect it to fail gracefully and not set an active pairing forever
        assert!(
            ap.lock().is_none(),
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
        let ftm = Arc::new(FileTransferManager::new(Arc::clone(&store), flume::unbounded().0));
        let lm = Arc::new(Libp2pManager::new(vec![0u8; 32], tx.clone(), Arc::clone(&store), ftm).unwrap());
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
            lm.clone(),
        );

        thread::spawn(move || pm.start_listener());
        thread::sleep(Duration::from_millis(50));

        // Send garbage data
        use std::io::Write;
        let mut stream = std::net::TcpStream::connect("127.0.0.1:5501").unwrap();
        stream.write_all(b"NOT A NOISE MESSAGE").unwrap();

        thread::sleep(Duration::from_millis(100));
        assert!(
            ap.lock().is_none(),
            "Should not crash or hang on malformed data"
        );
    }

    #[test]
    fn test_relay_remote_pairing() {
        use crate::relay::RelayManager;
        use base64::Engine;

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

        let ftm1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), flume::unbounded().0));
        let lm1 = Arc::new(Libp2pManager::new(vec![0u8; 32], tx1.clone(), Arc::clone(&store1), ftm1).unwrap());
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
            lm1,
        ));

        let ftm2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), flume::unbounded().0));
        let lm2 = Arc::new(Libp2pManager::new(vec![0u8; 32], tx2.clone(), Arc::clone(&store2), ftm2).unwrap());
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
            lm2,
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
                let s1 = ap1.lock();
                if let Some(ref st) = *s1 {
                    pin1 = st.pin.clone();
                }
            }
            {
                let s2 = ap2.lock();
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

        assert_eq!(ap1.lock().as_ref().unwrap().remote_label, label2);
        assert_eq!(ap2.lock().as_ref().unwrap().remote_label, label1);
    }

    #[test]
    fn test_paired_device_discovery_filtering() {
        let dir = tempdir().unwrap();
        let store = Arc::new(Store::init(dir.path()).unwrap());
        let (tx, rx) = flume::unbounded();
        let (tx_event, rx_event) = flume::unbounded();

        // Subscribe to events
        {
            let mut bus = EVENT_BUS.lock();
            bus.push(tx_event);
        }

        let id = "test_node_id".to_string();
        let label = "Test Device".to_string();

        // 1. Add device as paired
        store.add_paired_device(&id, &label, None).unwrap();

        let lw = Arc::new(Mutex::new(None));
        let dd = Arc::new(Mutex::new(Vec::new()));
        let ap = Arc::new(Mutex::new(None));
        let sm = Arc::new(SyncManager::new());
        let tm = Arc::new(TurnManager::new().unwrap());
        let (relay, _) = RelayManager::new(
            "local".to_string(),
            "http://localhost".to_string(),
            tx.clone(),
        );

        let (p_tx, _p_rx) = flume::unbounded();
        let ftm = Arc::new(FileTransferManager::new(Arc::clone(&store), p_tx));
        let lm = Arc::new(Libp2pManager::new(vec![0u8; 32], tx.clone(), Arc::clone(&store), ftm).unwrap());

        let pm = Arc::new(PairingManager::new(
            Arc::clone(&store),
            tx.clone(),
            "local".to_string(),
            vec![],
            0,
            Arc::clone(&ap),
            Arc::clone(&sm),
            Arc::new(relay),
            tm,
            lm.clone(),
        ));
        let lpt = Arc::new(Mutex::new(0u64));
        let peer_map = Arc::new(Mutex::new(HashMap::new()));

        // 2. Simulate discovery of the ALREADY PAIRED device
        tx.send(IpcMessage::DeviceDiscovered {
            node_id: id.clone(),
            label: label.clone(),
            os: "Linux".to_string(),
            ips: vec!["127.0.0.1".to_string()],
            port: 5200,
        })
        .unwrap();

        // Run daemon loop
        daemon_loop(
            tx.clone(),
            rx,
            Some(1),
            Arc::clone(&store),
            lw,
            Arc::clone(&dd),
            ap,
            sm,
            pm,
            lpt,
            peer_map,
            None,
            lm.clone(),
        );

        // 3. Verify it's NOT in discovered_devices list
        let discovered = dd.lock();
        assert!(
            !discovered.iter().any(|(node_id, _, _, _, _)| node_id == &id),
            "Paired device should not be added to discovered list"
        );

        // 4. Verify no DeviceDiscovered event was broadcasted
        while let Ok(msg) = rx_event.try_recv() {
            if let IpcMessage::DeviceDiscovered { node_id, .. } = msg {
                assert_ne!(node_id, id, "Paired device discovery should not be broadcasted");
            }
        }
    }

    #[test]
    fn test_file_transfer_does_not_block_clipboard() -> Result<(), Box<dyn std::error::Error>> {
        let _ = tracing_subscriber::fmt::try_init();
        let dir1 = tempdir()?;
        let dir2 = tempdir()?;

        let store1 = Arc::new(Store::init(dir1.path())?);
        let store2 = Arc::new(Store::init(dir2.path())?);

        let (tx1, rx1) = flume::unbounded();
        let (tx2, rx2) = flume::unbounded();

        let ap1 = Arc::new(Mutex::new(None));
        let ap2 = Arc::new(Mutex::new(None));

        let sm1 = Arc::new(SyncManager::new());
        let sm2 = Arc::new(SyncManager::new());

        let tm1 = Arc::new(TurnManager::new().unwrap());
        let tm2 = Arc::new(TurnManager::new().unwrap());

        let (id1, priv1) = store1.get_or_create_identity(dir1.path()).unwrap();
        let (id2, priv2) = store2.get_or_create_identity(dir2.path()).unwrap();

        // 1. Setup Pairing
        let (relay1, _) = RelayManager::new(id1.clone(), "http://localhost".to_string(), tx1.clone());
        let (relay2, _) = RelayManager::new(id2.clone(), "http://localhost".to_string(), tx2.clone());

        let ftm1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), flume::unbounded().0));
        let lm1 = Arc::new(Libp2pManager::new(vec![0u8; 32], tx1.clone(), Arc::clone(&store1), ftm1.clone()).unwrap());
        let ftm2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), flume::unbounded().0));
        let lm2 = Arc::new(Libp2pManager::new(vec![0u8; 32], tx2.clone(), Arc::clone(&store2), ftm2.clone()).unwrap());

        let pm1 = Arc::new(PairingManager::new(Arc::clone(&store1), tx1.clone(), id1.clone(), priv1.clone(), 5401, Arc::clone(&ap1), Arc::clone(&sm1), Arc::new(relay1), tm1, lm1));
        let pm2 = Arc::new(PairingManager::new(Arc::clone(&store2), tx2.clone(), id2.clone(), priv2.clone(), 5402, Arc::clone(&ap2), Arc::clone(&sm2), Arc::new(relay2), tm2, lm2));

        let pm2_c = Arc::clone(&pm2);
        thread::spawn(move || pm2_c.start_listener());
        thread::sleep(Duration::from_millis(100));

        let pm1_init = Arc::clone(&pm1);
        thread::spawn(move || { pm1_init.initiate_pairing("127.0.0.1:5402".parse().unwrap(), None); });

        // Auto-confirm for test
        let mut attempts = 0;
        while attempts < 20 && (ap1.lock().is_none() || ap2.lock().is_none()) {
            thread::sleep(Duration::from_millis(100));
            attempts += 1;
        }
        { *ap1.lock().as_ref().unwrap().confirmed.lock() = Some(true); }
        { *ap2.lock().as_ref().unwrap().confirmed.lock() = Some(true); }

        thread::sleep(Duration::from_millis(500));
        assert!(sm1.is_connected(&id2));

        // 2. Start File Transfer (Mock Network)
        let (prog_tx1, _) = flume::unbounded();
        let (prog_tx2, prog_rx2) = flume::unbounded();
        let ftm1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), prog_tx1));
        let ftm2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), prog_tx2));

        let large_file_path = dir1.path().join("large.bin");
        let large_content = vec![0u8; 10 * 1024 * 1024]; // 10MB
        std::fs::write(&large_file_path, &large_content)?;
        let file_hash = blake3::hash(&large_content).to_hex().to_string();
        let transfer_id = uuid::Uuid::new_v4().to_string();

        store1.create_transfer(&transfer_id, "outgoing", &id2, &large_file_path.to_string_lossy(), "large.bin", 10 * 1024 * 1024, 262144, &file_hash)?;

        let (ft_tx1, ft_rx1) = flume::unbounded::<cdus_common::FileMessage>(); // S -> R
        let (ft_tx2, ft_rx2) = flume::unbounded::<cdus_common::FileMessage>(); // R -> S
        
        struct IntegrationMockStream {
            tx: flume::Sender<cdus_common::FileMessage>,
            rx: flume::Receiver<cdus_common::FileMessage>,
        }
        impl crate::file_transfer::FileStream for IntegrationMockStream {
            fn write_message(&mut self, msg: &cdus_common::FileMessage) -> Result<(), anyhow::Error> { self.tx.send(msg.clone()).map_err(|e| anyhow::anyhow!(e)) }
            fn read_message(&mut self) -> Result<cdus_common::FileMessage, anyhow::Error> { self.rx.recv().map_err(|e| anyhow::anyhow!(e)) }
            fn read_message_timeout(&mut self, timeout: Duration) -> Result<cdus_common::FileMessage, anyhow::Error> { self.rx.recv_timeout(timeout).map_err(|e| anyhow::anyhow!(e)) }
        }

        let stream1 = Box::new(IntegrationMockStream { tx: ft_tx1, rx: ft_rx2 });
        let stream2 = Box::new(IntegrationMockStream { tx: ft_tx2, rx: ft_rx1 });

        let s_id = transfer_id.clone();
        let s_store = Arc::clone(&store1);
        let s_manager = Arc::clone(&ftm1);
        thread::spawn(move || {
            crate::file_transfer::handle_outgoing_transfer(stream1, s_store, s_id, crate::file_transfer::SessionKey([0u8; 32]), s_manager)
        });

        let r_store = Arc::clone(&store2);
        let r_manager = Arc::clone(&ftm2);
        let r_dir = dir2.path().to_path_buf();
        thread::spawn(move || {
            crate::file_transfer::handle_incoming_transfer_with_manager(stream2, r_store, crate::file_transfer::SessionKey([0u8; 32]), r_dir, flume::unbounded().0, r_manager, id1)
        });

        // Trigger acceptance so it starts "working"
        thread::sleep(Duration::from_millis(200));
        ftm2.handle_decision(&transfer_id, true);

        // 3. Test Clipboard while transfer is running in background
        let start = std::time::Instant::now();
        sm1.broadcast(SyncMessage::ClipboardUpdate { content: "test".to_string(), timestamp: 999 });

        let mut received = false;
        while let Ok(msg) = rx2.recv_timeout(Duration::from_millis(1000)) {
            if let IpcMessage::SetClipboard { content, .. } = msg {
                assert_eq!(content, "test");
                received = true;
                break;
            }
        }
        assert!(received, "Clipboard should arrive even if file transfer is running");
        assert!(start.elapsed() < Duration::from_millis(1000), "Clipboard should be fast even during file transfer");

        Ok(())
    }

    #[test]
    fn test_pair_with_qr_already_paired() {
        let _ = tracing_subscriber::fmt::try_init();
        let dir = tempdir().unwrap();
        let store = Arc::new(Store::init(dir.path()).unwrap());
        let (tx, rx) = flume::unbounded();
        let ap = Arc::new(Mutex::new(None));
        let sm = Arc::new(SyncManager::new());
        let tm = Arc::new(TurnManager::new().unwrap());
        let (node_id, priv_key) = store.get_or_create_identity(dir.path()).unwrap();
        
        let ftm = Arc::new(FileTransferManager::new(Arc::clone(&store), flume::unbounded().0));
        let lm = Arc::new(Libp2pManager::new(vec![0u8; 32], tx.clone(), Arc::clone(&store), ftm).unwrap());
        let pm = PairingManager::new(
            Arc::clone(&store),
            tx,
            node_id.clone(),
            priv_key,
            5601,
            Arc::clone(&ap),
            sm,
            Arc::new(RelayManager::new(node_id.clone(), "http://localhost".to_string(), flume::unbounded().0).0),
            tm,
            lm,
        );

        // 1. Manually mark a device as paired in the store
        let remote_id = "remote_node_id".to_string();
        store.add_paired_device(&remote_id, "Remote Device", None).unwrap();

        // 2. Scan a QR for that device
        let payload = format!("cdus://pair?id={}&s=secret&l=Remote", remote_id);
        let result = pm.pair_with_qr(payload);

        // 3. Verify result is Ok and IPC message is sent
        assert!(result.is_ok());
        
        let msg = rx.recv_timeout(Duration::from_secs(1)).expect("Should receive IPC message");
        match msg {
            IpcMessage::AlreadyPaired { node_id, label } => {
                assert_eq!(node_id, remote_id);
                assert_eq!(label, "Remote");
            }
            _ => panic!("Expected AlreadyPaired message, got {:?}", msg),
        }
    }

    #[test]
    fn test_asymmetrical_pairing_reconnect_rejection() {
        let _ = tracing_subscriber::fmt::try_init();
        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();

        let store1 = Arc::new(Store::init(dir1.path()).unwrap());
        let store2 = Arc::new(Store::init(dir2.path()).unwrap());

        let (tx1, rx1) = flume::unbounded();
        let (tx2, _rx2) = flume::unbounded();

        let (id1, priv1) = store1.get_or_create_identity(dir1.path()).unwrap();
        let (id2, priv2) = store2.get_or_create_identity(dir2.path()).unwrap();

        let ftm1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), flume::unbounded().0));
        let lm1 = Arc::new(Libp2pManager::new(vec![0u8; 32], tx1.clone(), Arc::clone(&store1), ftm1).unwrap());
        let pm1 = Arc::new(PairingManager::new(
            Arc::clone(&store1),
            tx1,
            id1.clone(),
            priv1,
            5701,
            Arc::new(Mutex::new(None)),
            Arc::new(SyncManager::new()),
            Arc::new(RelayManager::new(id1.clone(), "http://localhost".to_string(), flume::unbounded().0).0),
            Arc::new(TurnManager::new().unwrap()),
            lm1,
        ));

        let ftm2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), flume::unbounded().0));
        let lm2 = Arc::new(Libp2pManager::new(vec![0u8; 32], tx2.clone(), Arc::clone(&store2), ftm2).unwrap());
        let pm2 = Arc::new(PairingManager::new(
            Arc::clone(&store2),
            tx2,
            id2.clone(),
            priv2,
            5702,
            Arc::new(Mutex::new(None)),
            Arc::new(SyncManager::new()),
            Arc::new(RelayManager::new(id2.clone(), "http://localhost".to_string(), flume::unbounded().0).0),
            Arc::new(TurnManager::new().unwrap()),
            lm2,
        ));

        // 1. Manually establish pairing on Device 1 ONLY (Asymmetrical)
        store1.add_paired_device(&id2, "Device 2", None).unwrap();
        // Device 2 does NOT have Device 1 in its store.

        // 2. Start listener on Device 2
        let pm2_c = Arc::clone(&pm2);
        thread::spawn(move || pm2_c.start_listener());
        thread::sleep(Duration::from_millis(100));

        // 3. Device 1 attempts reconnection to Device 2
        let pm1_init = Arc::clone(&pm1);
        let addr = "127.0.0.1:5702".parse().unwrap();
        let id2_c = id2.clone();
        thread::spawn(move || {
            pm1_init.initiate_pairing(addr, Some(id2_c.clone()));
        });

        // 4. Verify Device 1 receives StalePairing message
        let msg = rx1.recv_timeout(Duration::from_secs(2)).expect("Should receive IPC message");
        match msg {
            IpcMessage::Log(ref s) if s.contains("Manual pairing initiated") => {
                // Skip the initial log message and wait for the actual result
                let msg2 = rx1.recv_timeout(Duration::from_secs(2)).expect("Should receive second IPC message");
                match msg2 {
                    IpcMessage::StalePairing { ref node_id, .. } => {
                        assert_eq!(node_id, &id2);
                    }
                    _ => panic!("Expected StalePairing, got {:?}", msg2),
                }
            }
            IpcMessage::StalePairing { ref node_id, .. } => {
                assert_eq!(node_id, &id2);
            }
            _ => panic!("Expected StalePairing, got {:?}", msg),
        }
    }

    #[test]
    fn test_local_only_clipboard_filtering() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::init(dir.path()).unwrap());
        let (tx, rx) = flume::unbounded();
        let lw = Arc::new(Mutex::new(None));
        let dd = Arc::new(Mutex::new(Vec::new()));
        let ap = Arc::new(Mutex::new(None));
        let sm = Arc::new(SyncManager::new());
        let tm = Arc::new(TurnManager::new().unwrap());
        let peer_map = Arc::new(Mutex::new(HashMap::new()));

        let (p_tx, _p_rx) = flume::unbounded();
        let ftm = Arc::new(FileTransferManager::new(Arc::clone(&store), p_tx));
        let lm = Arc::new(Libp2pManager::new(vec![0u8; 32], tx.clone(), Arc::clone(&store), ftm).unwrap());

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
            lm.clone(),
        ));
        let lpt = Arc::new(Mutex::new(0u64));

        // 1. Append a clipboard item and toggle it as local_only = true
        let payload = "Private Password".to_string();
        let _ = store.append_event(payload.as_bytes(), "Local").unwrap();
        
        let history = store.get_recent_events(1).unwrap();
        assert_eq!(history.len(), 1);
        let item_id = history[0].id;
        assert_eq!(history[0].content, payload);
        assert!(!history[0].local_only); // initially false

        // Toggle it to local_only
        store.set_local_only(item_id, true).unwrap();
        let history = store.get_recent_events(1).unwrap();
        assert!(history[0].local_only); // now true

        // 2. Test incoming SetClipboard prevention when local is local-only
        let (tx3, rx3) = flume::unbounded();
        tx3.send(IpcMessage::SetClipboard {
            content: "Remote Override".to_string(),
            timestamp: 3000u64,
            source: "Remote".to_string(),
        })
        .unwrap();

        // Run daemon loop
        daemon_loop(
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
            peer_map.clone(),
            None,
            lm.clone(),
        );

        // Verify that since our current item ("Private Password") is local-only,
        // the SetClipboard request from Remote was IGNORED (i.e. not written to clipboard or database).
        let history = store.get_recent_events(5).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "Private Password");
    }

    #[test]
    fn test_notification_mirror_and_dismiss() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::init(dir.path()).unwrap());
        let (tx, rx) = flume::unbounded();
        let lw = Arc::new(Mutex::new(None));
        let dd = Arc::new(Mutex::new(Vec::new()));
        let ap = Arc::new(Mutex::new(None));
        let sm = Arc::new(SyncManager::new());
        let tm = Arc::new(TurnManager::new().unwrap());
        let peer_map = Arc::new(Mutex::new(HashMap::new()));

        let (p_tx, _p_rx) = flume::unbounded();
        let ftm = Arc::new(FileTransferManager::new(Arc::clone(&store), p_tx));
        let lm = Arc::new(Libp2pManager::new(vec![0u8; 32], tx.clone(), Arc::clone(&store), ftm).unwrap());

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
            lm.clone(),
        ));
        let lpt = Arc::new(Mutex::new(0u64));

        // 1. Simulate a NotificationMirrored event sent to the daemon loop
        let payload = cdus_common::NotificationPayload {
            key: "test_key_123".to_string(),
            package_name: "com.example.app".to_string(),
            app_name: "Example App".to_string(),
            title: "Test Title".to_string(),
            text: "Test Notification Body".to_string(),
            timestamp: 1600000000u64,
        };

        let (tx_in, rx_in) = flume::unbounded();
        tx_in.send(IpcMessage::NotificationMirrored(payload.clone())).unwrap();

        // Run daemon loop to process IpcMessage::NotificationMirrored
        daemon_loop(
            tx.clone(),
            rx_in,
            Some(1), // process exactly one message
            Arc::clone(&store),
            Arc::clone(&lw),
            Arc::clone(&dd),
            Arc::clone(&ap),
            Arc::clone(&sm),
            Arc::clone(&pm),
            Arc::clone(&lpt),
            peer_map.clone(),
            None,
            lm.clone(),
        );

        // Verify active notifications contains our notification payload
        {
            let map = crate::ACTIVE_NOTIFICATIONS.lock();
            assert_eq!(map.len(), 1);
            assert_eq!(map.get("test_key_123").unwrap().title, "Test Title");
        }

        // 2. Simulate querying the active notifications from the client
        let (tx_in_2, rx_in_2) = flume::unbounded();
        tx_in_2.send(IpcMessage::GetActiveNotifications).unwrap();

        // Run daemon loop to process GetActiveNotifications
        daemon_loop(
            tx.clone(),
            rx_in_2,
            Some(1),
            Arc::clone(&store),
            Arc::clone(&lw),
            Arc::clone(&dd),
            Arc::clone(&ap),
            Arc::clone(&sm),
            Arc::clone(&pm),
            Arc::clone(&lpt),
            peer_map.clone(),
            None,
            lm.clone(),
        );

        // Verify we got the ActiveNotificationsResponse in FFI/Client rx
        let mut got_response = false;
        while let Ok(msg) = rx.try_recv() {
            if let IpcMessage::ActiveNotificationsResponse(list) = msg {
                assert_eq!(list.len(), 1);
                assert_eq!(list[0].key, "test_key_123");
                got_response = true;
            }
        }
        assert!(got_response, "Should have received ActiveNotificationsResponse");

        // 3. Simulate dismissing a notification (client clicked dismiss in UI)
        let (tx_in_3, rx_in_3) = flume::unbounded();
        tx_in_3.send(IpcMessage::DismissNotification { key: "test_key_123".to_string() }).unwrap();

        // Run daemon loop to process DismissNotification
        daemon_loop(
            tx.clone(),
            rx_in_3,
            Some(1),
            Arc::clone(&store),
            Arc::clone(&lw),
            Arc::clone(&dd),
            Arc::clone(&ap),
            Arc::clone(&sm),
            Arc::clone(&pm),
            Arc::clone(&lpt),
            peer_map.clone(),
            None,
            lm.clone(),
        );

        // Verify it was removed from active notifications map
        {
            let map = crate::ACTIVE_NOTIFICATIONS.lock();
            assert_eq!(map.len(), 0);
        }
    }
}
