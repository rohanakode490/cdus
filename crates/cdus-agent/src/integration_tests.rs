#[cfg(test)]
mod tests {
    use crate::pairing::{PairingManager, SyncManager};
    use crate::store::Store;
    use crate::ActivePairingState;
    use cdus_common::{IpcMessage, SyncMessage};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use std::thread;
    use tempfile::tempdir;

    #[test]
    fn test_mutual_pairing_and_clipboard_sync() {
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

        let (id1, priv1) = store1.get_or_create_identity(dir1.path()).unwrap();
        let (id2, priv2) = store2.get_or_create_identity(dir2.path()).unwrap();

        let pm1 = PairingManager::new(
            Arc::clone(&store1),
            tx1.clone(),
            id1.clone(),
            priv1,
            5201,
            Arc::clone(&ap1),
            Arc::clone(&sm1),
        );
        let pm2 = PairingManager::new(
            Arc::clone(&store2),
            tx2.clone(),
            id2.clone(),
            priv2,
            5202,
            Arc::clone(&ap2),
            Arc::clone(&sm2),
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
        while attempts < 20 {
            let p1 = ap1.lock().unwrap().is_some();
            let p2 = ap2.lock().unwrap().is_some();
            if p1 && p2 {
                break;
            }
            thread::sleep(Duration::from_millis(100));
            attempts += 1;
        }

        assert!(ap1.lock().unwrap().is_some(), "Initiator should have active pairing");
        assert!(ap2.lock().unwrap().is_some(), "Responder should have active pairing");

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
            if p1_success && p2_success { break; }
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
            timestamp: test_ts 
        });

        // Wait for rx2 to receive SetClipboard
        let mut received_content = String::new();
        for _ in 0..20 {
            while let Ok(msg) = rx2.try_recv() {
                if let IpcMessage::SetClipboard { content, .. } = msg {
                    received_content = content;
                }
            }
            if !received_content.is_empty() { break; }
            thread::sleep(Duration::from_millis(100));
        }

        assert_eq!(received_content, test_content, "Node 2 did not receive the correct clipboard content");
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
        let pm = Arc::new(PairingManager::new(Arc::clone(&store), tx.clone(), "test".to_string(), vec![], 0, Arc::clone(&ap), Arc::clone(&sm)));
        let lpt = Arc::new(Mutex::new(0u64));

        // Initial state
        let ts1 = 1000u64;
        tx.send(IpcMessage::SetClipboard { content: "Initial".to_string(), timestamp: ts1, source: "Remote".to_string() }).unwrap();
        
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
        crate::daemon_loop(tx_daemon, rx, Some(5), store_daemon, lw_daemon, dd_daemon, ap_daemon, sm_daemon, pm_daemon, lpt_daemon);

        // 1. Send older message (should be ignored)
        let (tx2, rx2) = flume::unbounded();
        tx2.send(IpcMessage::SetClipboard { content: "Older".to_string(), timestamp: ts1 - 10, source: "Remote".to_string() }).unwrap();
        
        // Reset lw for check
        *lw.lock().unwrap() = None;
        crate::daemon_loop(tx.clone(), rx2, Some(5), Arc::clone(&store), Arc::clone(&lw), Arc::clone(&dd), Arc::clone(&ap), Arc::clone(&sm), Arc::clone(&pm), Arc::clone(&lpt));
        assert_eq!(*lw.lock().unwrap(), None, "Older message should not have been written to clipboard");

        // 2. Send newer message (should be accepted)
        let (tx3, rx3) = flume::unbounded();
        let ts2 = ts1 + 100;
        tx3.send(IpcMessage::SetClipboard { content: "Newer".to_string(), timestamp: ts2, source: "Remote".to_string() }).unwrap();
        
        crate::daemon_loop(tx.clone(), rx3, Some(5), Arc::clone(&store), Arc::clone(&lw), Arc::clone(&dd), Arc::clone(&ap), Arc::clone(&sm), Arc::clone(&pm), Arc::clone(&lpt));
        assert_eq!(*lw.lock().unwrap(), Some("Newer".to_string()), "Newer message should have been written to clipboard");
    }

    #[test]
    fn test_pairing_rejection() {
        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();
        let store1 = Arc::new(Store::init(dir1.path()).unwrap());
        let store2 = Arc::new(Store::init(dir2.path()).unwrap());
        let (tx1, rx1) = flume::unbounded();
        let (tx2, rx2) = flume::unbounded();
        let ap1 = Arc::new(Mutex::new(None));
        let ap2 = Arc::new(Mutex::new(None));
        let sm1 = Arc::new(SyncManager::new());
        let sm2 = Arc::new(SyncManager::new());
        let (id1, priv1) = store1.get_or_create_identity(dir1.path()).unwrap();
        let (id2, priv2) = store2.get_or_create_identity(dir2.path()).unwrap();

        let pm1 = Arc::new(PairingManager::new(Arc::clone(&store1), tx1, id1, priv1, 5301, Arc::clone(&ap1), Arc::clone(&sm1)));
        let pm2 = Arc::new(PairingManager::new(Arc::clone(&store2), tx2, id2, priv2, 5302, Arc::clone(&ap2), Arc::clone(&sm2)));

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
            if !p2_success { break; }
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
        let pm = Arc::new(PairingManager::new(Arc::clone(&store), tx, id.clone(), priv_key, 5401, Arc::clone(&ap), Arc::clone(&sm)));

        let pm_c = Arc::clone(&pm);
        thread::spawn(move || pm_c.start_listener());
        thread::sleep(Duration::from_millis(50));

        // Attempt to connect to self
        let addr = "127.0.0.1:5401".parse().unwrap();
        pm.initiate_pairing(addr);

        thread::sleep(Duration::from_millis(200));
        // We expect it to fail gracefully and not set an active pairing forever
        assert!(ap.lock().unwrap().is_none(), "Active pairing should be None after self-connection attempt");
    }

    #[test]
    fn test_malformed_handshake() {
        let dir = tempdir().unwrap();
        let store = Arc::new(Store::init(dir.path()).unwrap());
        let (tx, _rx) = flume::unbounded();
        let (id, priv_key) = store.get_or_create_identity(dir.path()).unwrap();
        let ap = Arc::new(Mutex::new(None));
        let sm = Arc::new(SyncManager::new());
        let pm = PairingManager::new(Arc::clone(&store), tx, id, priv_key, 5501, Arc::clone(&ap), Arc::clone(&sm));

        thread::spawn(move || pm.start_listener());
        thread::sleep(Duration::from_millis(50));

        // Send garbage data
        use std::io::Write;
        let mut stream = std::net::TcpStream::connect("127.0.0.1:5501").unwrap();
        stream.write_all(b"NOT A NOISE MESSAGE").unwrap();
        
        thread::sleep(Duration::from_millis(100));
        assert!(ap.lock().unwrap().is_none(), "Should not crash or hang on malformed data");
    }
}
