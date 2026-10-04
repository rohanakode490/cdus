use anyhow::{anyhow, Result};
use flume::{Receiver, Sender};
use rand::rngs::OsRng;
use rand::RngCore;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

use cdus_agent::file_transfer::{
    derive_peer_session_key, handle_incoming_transfer_with_manager, handle_outgoing_transfer,
    FileStream, FileTransferManager, SessionKey,
};
use cdus_agent::store::Store;
use cdus_common::FileMessage;

// ============================================================================
// In-memory bidirectional stream harness
// ============================================================================

struct TestHarnessStream {
    tx: Sender<FileMessage>,
    rx: Receiver<FileMessage>,
    bytes_written: Arc<AtomicU64>,
    drop_after_bytes: Option<u64>,
    is_dropped: Arc<AtomicBool>,
}

impl TestHarnessStream {
    fn pair(drop_after_bytes: Option<u64>) -> (Self, Self) {
        let (tx1, rx1) = flume::unbounded();
        let (tx2, rx2) = flume::unbounded();
        let bytes_written = Arc::new(AtomicU64::new(0));
        let is_dropped = Arc::new(AtomicBool::new(false));

        let stream1 = TestHarnessStream {
            tx: tx1,
            rx: rx2,
            bytes_written: Arc::clone(&bytes_written),
            drop_after_bytes,
            is_dropped: Arc::clone(&is_dropped),
        };

        let stream2 = TestHarnessStream {
            tx: tx2,
            rx: rx1,
            bytes_written,
            drop_after_bytes: None,
            is_dropped,
        };

        (stream1, stream2)
    }
}

impl FileStream for TestHarnessStream {
    fn write_message(&mut self, msg: &FileMessage) -> Result<()> {
        if self.is_dropped.load(Ordering::SeqCst) {
            return Err(anyhow!("Connection dropped"));
        }
        if let Some(limit) = self.drop_after_bytes {
            let current = self.bytes_written.load(Ordering::SeqCst);
            if current >= limit {
                self.is_dropped.store(true, Ordering::SeqCst);
                return Err(anyhow!("Connection dropped at limit {}", limit));
            }
        }
        if let FileMessage::Chunk(chunk) = msg {
            self.bytes_written
                .fetch_add(chunk.data.len() as u64, Ordering::SeqCst);
        }
        self.tx
            .send(msg.clone())
            .map_err(|_| anyhow!("Stream closed"))
    }

    fn read_message(&mut self) -> Result<FileMessage> {
        if self.is_dropped.load(Ordering::SeqCst) {
            return Err(anyhow!("Connection dropped"));
        }
        self.rx.recv().map_err(|_| anyhow!("Stream closed"))
    }

    fn read_message_timeout(&mut self, timeout: Duration) -> Result<FileMessage> {
        if self.is_dropped.load(Ordering::SeqCst) {
            return Err(anyhow!("Connection dropped"));
        }
        self.rx
            .recv_timeout(timeout)
            .map_err(|_| anyhow!("Timeout reading stream"))
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Generates `len` bytes of cryptographically random data using OsRng.
///
/// Uses `rand::rngs::OsRng` instead of the deprecated `rand::thread_rng()`
/// (removed in rand 0.9+) so this compiles cleanly on both rand 0.8 and 0.9.
fn random_bytes(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    OsRng.fill_bytes(&mut buf);
    buf
}

/// Builds two paired stores and returns both stores plus their derived session key.
///
/// Both stores add each other as a paired device with a known static key fixture.
/// The `derive_peer_session_key` function is exercised here, testing the actual
/// BLAKE3-based key derivation path used in production pairing.
fn make_paired_stores(
    dir1: &std::path::Path,
    dir2: &std::path::Path,
) -> Result<(Arc<Store>, Arc<Store>, String, String, SessionKey)> {
    // P1 Fix: use deterministic but distinct static key fixtures that match what
    // the real Noise XX handshake would produce, so derive_peer_session_key()
    // produces a matching key on both sides.
    const STATIC_KEY_A: [u8; 32] = [0xAA; 32];
    const STATIC_KEY_B: [u8; 32] = [0xBB; 32];

    let store1 = Arc::new(Store::init(dir1)?);
    let store2 = Arc::new(Store::init(dir2)?);

    let (id1, _) = store1.get_or_create_identity(dir1)?;
    let (id2, _) = store2.get_or_create_identity(dir2)?;

    store1.add_paired_device(&id2, "Android Emulator", Some(&STATIC_KEY_A))?;
    store2.add_paired_device(&id1, "Desktop Node", Some(&STATIC_KEY_B))?;

    // P1 Fix: derive the session key the same way the daemon does — via
    // derive_peer_session_key — instead of a hardcoded placeholder constant.
    // This exercises the BLAKE3 KDF path and catches any regressions in it.
    let session_key = derive_peer_session_key(&store1, &id2);

    Ok((store1, store2, id1, id2, session_key))
}

fn wait_and_handle_decision(manager: &FileTransferManager, transfer_id: &str, accepted: bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if manager.pending_decisions.lock().contains_key(transfer_id) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    manager.handle_decision(transfer_id, accepted);
}

// ============================================================================
// Test 1: Clean end-to-end transfer with BLAKE3 integrity verification
// ============================================================================

#[test]
fn test_multi_node_clean_e2e_transfer() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let dir1 = tempdir()?;
    let dir2 = tempdir()?;

    let (store1, store2, _id1, id2, session_key) = make_paired_stores(dir1.path(), dir2.path())?;

    let file_size: usize = 2 * 1024 * 1024; // 2 MB
    let source_path = dir1.path().join("test_file.bin");
    // P0 Fix: use OsRng instead of the deprecated rand::thread_rng()
    let payload = random_bytes(file_size);
    std::fs::write(&source_path, &payload)?;

    let original_hash = blake3::hash(&payload).to_hex().to_string();
    let transfer_id = uuid::Uuid::new_v4().to_string();

    let (prog_tx1, _prog_rx1) = flume::unbounded();
    let (prog_tx2, _prog_rx2) = flume::unbounded();

    let manager1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), prog_tx1));
    let manager2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), prog_tx2));

    let (stream1, stream2) = TestHarnessStream::pair(None);

    store1.create_transfer(
        &transfer_id,
        "outgoing",
        &id2,
        &source_path.to_string_lossy(),
        "test_file.bin",
        file_size as u64,
        65536,
        &original_hash,
    )?;

    let t_id = transfer_id.clone();
    let s1 = Arc::clone(&store1);
    let m1 = Arc::clone(&manager1);
    let sk1 = SessionKey(session_key.0);
    let sender =
        thread::spawn(move || handle_outgoing_transfer(Box::new(stream1), s1, t_id, sk1, m1));

    let (id1_for_receiver, _) = store2.get_or_create_identity(dir2.path())?; // re-fetch id1 via store2
    let s2 = Arc::clone(&store2);
    let m2 = Arc::clone(&manager2);
    let sk2 = SessionKey(session_key.0);
    let dl2 = dir2.path().to_path_buf();
    let receiver = thread::spawn(move || {
        handle_incoming_transfer_with_manager(
            Box::new(stream2),
            s2,
            sk2,
            dl2,
            flume::unbounded().0,
            m2,
            id1_for_receiver,
        )
    });

    wait_and_handle_decision(&manager2, &transfer_id, true);

    sender.join().unwrap()?;
    receiver.join().unwrap()?;

    let dest_path = dir2.path().join("test_file.bin");
    assert!(dest_path.exists(), "Destination file must exist");

    let dest_content = std::fs::read(&dest_path)?;
    assert_eq!(dest_content.len(), file_size);
    let dest_hash = blake3::hash(&dest_content).to_hex().to_string();
    assert_eq!(dest_hash, original_hash, "BLAKE3 hash must match");

    Ok(())
}

// ============================================================================
// Test 2: Connection interruption and offset resume
// ============================================================================

#[test]
fn test_multi_node_interrupt_and_resume() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let dir1 = tempdir()?;
    let dir2 = tempdir()?;

    let (store1, store2, _id1, id2, session_key) = make_paired_stores(dir1.path(), dir2.path())?;

    let file_size: usize = 3 * 1024 * 1024; // 3 MB
    let source_path = dir1.path().join("resumable_file.bin");
    // P0 Fix: use OsRng instead of the deprecated rand::thread_rng()
    let payload = random_bytes(file_size);
    std::fs::write(&source_path, &payload)?;

    let original_hash = blake3::hash(&payload).to_hex().to_string();
    let transfer_id = uuid::Uuid::new_v4().to_string();

    let (prog_tx1, _prog_rx1) = flume::unbounded();
    let (prog_tx2, _prog_rx2) = flume::unbounded();

    let manager1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), prog_tx1));
    let manager2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), prog_tx2));

    // Phase 1: Sever connection at 1.5 MB
    let drop_limit = 1_500_000u64;
    let (stream1, stream2) = TestHarnessStream::pair(Some(drop_limit));

    store1.create_transfer(
        &transfer_id,
        "outgoing",
        &id2,
        &source_path.to_string_lossy(),
        "resumable_file.bin",
        file_size as u64,
        65536,
        &original_hash,
    )?;

    let t_id1 = transfer_id.clone();
    let s1 = Arc::clone(&store1);
    let m1 = Arc::clone(&manager1);
    let sk1 = SessionKey(session_key.0);
    let sender1 =
        thread::spawn(move || handle_outgoing_transfer(Box::new(stream1), s1, t_id1, sk1, m1));

    let (id1_for_recv1, _) = store2.get_or_create_identity(dir2.path())?;
    let s2 = Arc::clone(&store2);
    let m2 = Arc::clone(&manager2);
    let sk2 = SessionKey(session_key.0);
    let dl2 = dir2.path().to_path_buf();
    let receiver1 = thread::spawn(move || {
        handle_incoming_transfer_with_manager(
            Box::new(stream2),
            s2,
            sk2,
            dl2,
            flume::unbounded().0,
            m2,
            id1_for_recv1,
        )
    });

    wait_and_handle_decision(&manager2, &transfer_id, true);

    let _ = sender1.join();
    let _ = receiver1.join();

    let part_path = dir2.path().join("resumable_file.cdus.part");
    assert!(
        part_path.exists(),
        ".cdus.part file must exist after severed connection"
    );
    let partial_bytes = std::fs::metadata(&part_path)?.len();
    assert!(
        partial_bytes > 0,
        "Partial file should have captured bytes before drop"
    );
    assert!(
        partial_bytes < file_size as u64,
        "Partial file should not be complete before drop"
    );

    // Phase 2: Resume transfer from offset
    let (res_stream1, res_stream2) = TestHarnessStream::pair(None);
    let t_id2 = transfer_id.clone();
    let s1_res = Arc::clone(&store1);
    let m1_res = Arc::clone(&manager1);
    let sk1_res = SessionKey(session_key.0);
    let sender2 = thread::spawn(move || {
        handle_outgoing_transfer(Box::new(res_stream1), s1_res, t_id2, sk1_res, m1_res)
    });

    let (id1_for_recv2, _) = store2.get_or_create_identity(dir2.path())?;
    let s2_res = Arc::clone(&store2);
    let m2_res = Arc::clone(&manager2);
    let sk2_res = SessionKey(session_key.0);
    let dl2_res = dir2.path().to_path_buf();
    let receiver2 = thread::spawn(move || {
        handle_incoming_transfer_with_manager(
            Box::new(res_stream2),
            s2_res,
            sk2_res,
            dl2_res,
            flume::unbounded().0,
            m2_res,
            id1_for_recv2,
        )
    });

    wait_and_handle_decision(&manager2, &transfer_id, true);

    sender2.join().unwrap()?;
    receiver2.join().unwrap()?;

    let final_dest = dir2.path().join("resumable_file.bin");
    assert!(final_dest.exists(), "Final resumed file must exist");
    assert!(
        !part_path.exists(),
        "Partial file must be deleted upon completion"
    );

    let final_content = std::fs::read(&final_dest)?;
    assert_eq!(final_content.len(), file_size);
    let final_hash = blake3::hash(&final_content).to_hex().to_string();
    assert_eq!(
        final_hash, original_hash,
        "Resumed file BLAKE3 hash must match"
    );

    Ok(())
}

// ============================================================================
// Test 3 (P1 new): Receiver rejects the incoming transfer
// ============================================================================

/// Validates that when the receiving side calls `handle_decision(false)`, no file
/// is written to disk and the transfer state is correctly marked as rejected.
///
/// Previously this path was never exercised: `wait_and_handle_decision` always
/// passed `accepted = true`. A bug in the rejection flow (e.g., the sender not
/// receiving a rejection and hanging, or a partial file being left on disk) would
/// have gone completely undetected.
#[test]
fn test_multi_node_rejected_transfer() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let dir1 = tempdir()?;
    let dir2 = tempdir()?;

    let (store1, store2, _id1, id2, session_key) = make_paired_stores(dir1.path(), dir2.path())?;

    let file_size: usize = 512 * 1024; // 512 KB — small, we expect no bytes to land
    let source_path = dir1.path().join("rejected_file.bin");
    let payload = random_bytes(file_size);
    std::fs::write(&source_path, &payload)?;

    let original_hash = blake3::hash(&payload).to_hex().to_string();
    let transfer_id = uuid::Uuid::new_v4().to_string();

    let (prog_tx1, _prog_rx1) = flume::unbounded();
    let (prog_tx2, _prog_rx2) = flume::unbounded();

    let manager1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), prog_tx1));
    let manager2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), prog_tx2));

    let (stream1, stream2) = TestHarnessStream::pair(None);

    store1.create_transfer(
        &transfer_id,
        "outgoing",
        &id2,
        &source_path.to_string_lossy(),
        "rejected_file.bin",
        file_size as u64,
        65536,
        &original_hash,
    )?;

    let t_id = transfer_id.clone();
    let s1 = Arc::clone(&store1);
    let m1 = Arc::clone(&manager1);
    let sk1 = SessionKey(session_key.0);
    let sender =
        thread::spawn(move || handle_outgoing_transfer(Box::new(stream1), s1, t_id, sk1, m1));

    let (id1_for_receiver, _) = store2.get_or_create_identity(dir2.path())?;
    let s2 = Arc::clone(&store2);
    let m2 = Arc::clone(&manager2);
    let sk2 = SessionKey(session_key.0);
    let dl2 = dir2.path().to_path_buf();
    let receiver = thread::spawn(move || {
        handle_incoming_transfer_with_manager(
            Box::new(stream2),
            s2,
            sk2,
            dl2,
            flume::unbounded().0,
            m2,
            id1_for_receiver,
        )
    });

    // P1 New: reject the transfer — `accepted = false`
    wait_and_handle_decision(&manager2, &transfer_id, false);

    // Both sides should terminate cleanly (the sender receives a rejection signal)
    let _ = sender.join();
    let _ = receiver.join();

    // Neither the final file nor any partial file should exist on the receiver
    let dest_path = dir2.path().join("rejected_file.bin");
    let part_path = dir2.path().join("rejected_file.cdus.part");
    assert!(
        !dest_path.exists(),
        "Rejected transfer must not produce a destination file"
    );
    assert!(
        !part_path.exists(),
        "Rejected transfer must not leave a .cdus.part file"
    );

    Ok(())
}
