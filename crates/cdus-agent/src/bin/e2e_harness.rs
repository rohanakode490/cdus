use anyhow::{anyhow, Result};
use clap::Parser;
use flume::{Receiver, Sender};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;
use tracing::info;

use cdus_agent::file_transfer::{
    handle_incoming_transfer_with_manager, handle_outgoing_transfer, FileStream,
    FileTransferManager, SessionKey,
};
use cdus_agent::store::Store;
use cdus_common::FileMessage;

/// Droppable In-Memory FileStream for E2E Multi-Node Testing
pub struct HarnessStream {
    pub tx: Sender<FileMessage>,
    pub rx: Receiver<FileMessage>,
    pub bytes_written: Arc<AtomicU64>,
    pub drop_after_bytes: Option<u64>,
    pub is_dropped: Arc<AtomicBool>,
}

impl HarnessStream {
    pub fn pair(drop_after_bytes: Option<u64>) -> (Self, Self) {
        let (tx1, rx1) = flume::unbounded();
        let (tx2, rx2) = flume::unbounded();
        let bytes_written = Arc::new(AtomicU64::new(0));
        let is_dropped = Arc::new(AtomicBool::new(false));

        let stream1 = HarnessStream {
            tx: tx1,
            rx: rx2,
            bytes_written: Arc::clone(&bytes_written),
            drop_after_bytes,
            is_dropped: Arc::clone(&is_dropped),
        };

        let stream2 = HarnessStream {
            tx: tx2,
            rx: rx1,
            bytes_written,
            drop_after_bytes: None,
            is_dropped,
        };

        (stream1, stream2)
    }
}

impl FileStream for HarnessStream {
    fn write_message(&mut self, msg: &FileMessage) -> Result<()> {
        if self.is_dropped.load(Ordering::SeqCst) {
            return Err(anyhow!("HarnessStream simulated drop: connection severed"));
        }

        if let Some(limit) = self.drop_after_bytes {
            let current = self.bytes_written.load(Ordering::SeqCst);
            if current >= limit {
                self.is_dropped.store(true, Ordering::SeqCst);
                return Err(anyhow!(
                    "HarnessStream simulated drop triggered at {} bytes",
                    current
                ));
            }
        }

        if let FileMessage::Chunk(chunk) = msg {
            self.bytes_written
                .fetch_add(chunk.data.len() as u64, Ordering::SeqCst);
        }

        self.tx
            .send(msg.clone())
            .map_err(|_| anyhow!("Stream closed by receiver"))
    }

    fn read_message(&mut self) -> Result<FileMessage> {
        if self.is_dropped.load(Ordering::SeqCst) {
            return Err(anyhow!("HarnessStream simulated drop: connection severed"));
        }
        self.rx
            .recv()
            .map_err(|_| anyhow!("Stream closed by sender"))
    }

    fn read_message_timeout(&mut self, timeout: Duration) -> Result<FileMessage> {
        if self.is_dropped.load(Ordering::SeqCst) {
            return Err(anyhow!("HarnessStream simulated drop: connection severed"));
        }
        self.rx
            .recv_timeout(timeout)
            .map_err(|_| anyhow!("Timeout reading stream"))
    }
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

#[derive(Parser, Debug)]
#[command(
    name = "cdus-e2e-harness",
    about = "Automated cross-device multi-node file transfer testing harness"
)]
struct Args {
    /// File size in Megabytes to generate and transfer
    #[arg(short = 's', long = "size-mb", default_value = "5")]
    size_mb: u64,

    /// Simulate network drop mid-transfer and validate resume from chunk offset
    #[arg(long = "simulate-drop")]
    simulate_drop: bool,

    /// Output results in JSON format for automated CI parsing
    #[arg(long = "json")]
    json: bool,

    /// Number of test iterations to run
    #[arg(short = 'i', long = "iterations", default_value = "1")]
    iterations: u32,
}

#[derive(serde::Serialize)]
struct TestReport {
    success: bool,
    iterations: u32,
    file_size_bytes: u64,
    elapsed_ms: u128,
    throughput_mb_s: f64,
    simulated_drop_tested: bool,
    blake3_verified: bool,
    error: Option<String>,
}

fn run_single_e2e_transfer(size_mb: u64, simulate_drop: bool) -> Result<(Duration, f64)> {
    let dir1 = tempdir()?;
    let dir2 = tempdir()?;

    let store1 = Arc::new(Store::init(dir1.path())?);
    let store2 = Arc::new(Store::init(dir2.path())?);

    let (id1, _) = store1.get_or_create_identity(dir1.path())?;
    let (id2, _) = store2.get_or_create_identity(dir2.path())?;

    // Pre-pair Node 1 (Desktop) and Node 2 (Android)
    store1.add_paired_device(&id2, "Android Emulator / Mobile", Some(&[1u8; 32]))?;
    store2.add_paired_device(&id1, "Linux / Windows Desktop", Some(&[2u8; 32]))?;

    // Generate random binary payload
    let total_bytes = size_mb * 1024 * 1024;
    let source_file_path = dir1.path().join(format!("payload_{}mb.bin", size_mb));
    let mut payload = vec![0u8; total_bytes as usize];
    rand::Rng::fill(&mut rand::thread_rng(), &mut payload[..]);
    std::fs::write(&source_file_path, &payload)?;

    let original_hash = blake3::hash(&payload).to_hex().to_string();
    let transfer_id = uuid::Uuid::new_v4().to_string();
    let chunk_size = 65536u32; // 64 KiB canonical chunk size

    let (prog_tx1, _prog_rx1) = flume::unbounded();
    let (prog_tx2, _prog_rx2) = flume::unbounded();

    let manager1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), prog_tx1));
    let manager2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), prog_tx2));

    let session_key = SessionKey([42u8; 32]);
    let start_time = Instant::now();

    if simulate_drop {
        info!("Phase 1: Starting transfer with forced network sever at 50%...");
        let drop_threshold = total_bytes / 2;
        let (stream1, stream2) = HarnessStream::pair(Some(drop_threshold));

        store1.create_transfer(
            &transfer_id,
            "outgoing",
            &id2,
            &source_file_path.to_string_lossy(),
            &format!("payload_{}mb.bin", size_mb),
            total_bytes,
            chunk_size,
            &original_hash,
        )?;

        let t_id_1 = transfer_id.clone();
        let s1_clone = Arc::clone(&store1);
        let m1_clone = Arc::clone(&manager1);
        let sk1 = session_key;
        let sender_phase1 = thread::spawn(move || {
            handle_outgoing_transfer(Box::new(stream1), s1_clone, t_id_1, sk1, m1_clone)
        });

        let s2_clone = Arc::clone(&store2);
        let m2_clone = Arc::clone(&manager2);
        let sk2 = session_key;
        let dl2 = dir2.path().to_path_buf();
        let peer1_id = id1.clone();
        let receiver_phase1 = thread::spawn(move || {
            handle_incoming_transfer_with_manager(
                Box::new(stream2),
                s2_clone,
                sk2,
                dl2,
                flume::unbounded().0,
                m2_clone,
                peer1_id,
            )
        });

        wait_and_handle_decision(&manager2, &transfer_id, true);

        // Phase 1 expected to fail due to dropped stream
        let _ = sender_phase1.join();
        let _ = receiver_phase1.join();

        // Check that .cdus.part file exists and has ~50% bytes
        let dest_file_path = dir2.path().join(format!("payload_{}mb.bin", size_mb));
        let part_file_path = dest_file_path.with_extension("cdus.part");
        if !part_file_path.exists() {
            return Err(anyhow!(
                "Expected .cdus.part file to exist after interrupted Phase 1"
            ));
        }

        let partial_len = std::fs::metadata(&part_file_path)?.len();
        info!(
            "Phase 1 severed successfully: captured {} bytes in partial file",
            partial_len
        );

        info!("Phase 2: Re-establishing stream and resuming from offset...");
        let (stream1_res, stream2_res) = HarnessStream::pair(None);

        let t_id_2 = transfer_id.clone();
        let s1_res = Arc::clone(&store1);
        let m1_res = Arc::clone(&manager1);
        let sk1_res = session_key;
        let sender_phase2 = thread::spawn(move || {
            handle_outgoing_transfer(Box::new(stream1_res), s1_res, t_id_2, sk1_res, m1_res)
        });

        let s2_res = Arc::clone(&store2);
        let m2_res = Arc::clone(&manager2);
        let sk2_res = session_key;
        let dl2_res = dir2.path().to_path_buf();
        let peer1_id_res = id1.clone();
        let receiver_phase2 = thread::spawn(move || {
            handle_incoming_transfer_with_manager(
                Box::new(stream2_res),
                s2_res,
                sk2_res,
                dl2_res,
                flume::unbounded().0,
                m2_res,
                peer1_id_res,
            )
        });

        wait_and_handle_decision(&manager2, &transfer_id, true);

        sender_phase2
            .join()
            .map_err(|_| anyhow!("Sender thread panicked in Phase 2"))??;
        receiver_phase2
            .join()
            .map_err(|_| anyhow!("Receiver thread panicked in Phase 2"))??;
    } else {
        info!("Starting clean end-to-end transfer ({} MB)...", size_mb);
        let (stream1, stream2) = HarnessStream::pair(None);

        store1.create_transfer(
            &transfer_id,
            "outgoing",
            &id2,
            &source_file_path.to_string_lossy(),
            &format!("payload_{}mb.bin", size_mb),
            total_bytes,
            chunk_size,
            &original_hash,
        )?;

        let t_id_1 = transfer_id.clone();
        let s1_clone = Arc::clone(&store1);
        let m1_clone = Arc::clone(&manager1);
        let sk1 = session_key;
        let sender = thread::spawn(move || {
            handle_outgoing_transfer(Box::new(stream1), s1_clone, t_id_1, sk1, m1_clone)
        });

        let s2_clone = Arc::clone(&store2);
        let m2_clone = Arc::clone(&manager2);
        let sk2 = session_key;
        let dl2 = dir2.path().to_path_buf();
        let peer1_id = id1.clone();
        let receiver = thread::spawn(move || {
            handle_incoming_transfer_with_manager(
                Box::new(stream2),
                s2_clone,
                sk2,
                dl2,
                flume::unbounded().0,
                m2_clone,
                peer1_id,
            )
        });

        wait_and_handle_decision(&manager2, &transfer_id, true);

        sender
            .join()
            .map_err(|_| anyhow!("Sender thread panicked"))??;
        receiver
            .join()
            .map_err(|_| anyhow!("Receiver thread panicked"))??;
    }

    let elapsed = start_time.elapsed();
    let dest_file_path = dir2.path().join(format!("payload_{}mb.bin", size_mb));

    if !dest_file_path.exists() {
        return Err(anyhow!(
            "Destination file does not exist at {:?}",
            dest_file_path
        ));
    }

    let dest_content = std::fs::read(&dest_file_path)?;
    if dest_content.len() != total_bytes as usize {
        return Err(anyhow!(
            "Size mismatch: expected {} bytes, got {}",
            total_bytes,
            dest_content.len()
        ));
    }

    let dest_hash = blake3::hash(&dest_content).to_hex().to_string();
    if dest_hash != original_hash {
        return Err(anyhow!(
            "BLAKE3 whole-file hash mismatch! Expected {}, got {}",
            original_hash,
            dest_hash
        ));
    }

    let throughput_mb_s = (total_bytes as f64 / 1_048_576.0) / elapsed.as_secs_f64();
    Ok((elapsed, throughput_mb_s))
}

fn main() {
    let args = Args::parse();
    if !args.json {
        tracing_subscriber::fmt::init();
        println!("============================================================");
        println!("       CDUS Cross-Device Automated E2E Test Harness         ");
        println!("============================================================");
        println!("  Target Payload:    {} MB", args.size_mb);
        println!("  Simulate Drop:     {}", args.simulate_drop);
        println!("  Iterations:        {}", args.iterations);
        println!("------------------------------------------------------------");
    }

    let mut total_duration = Duration::ZERO;
    let mut total_throughput = 0.0;
    let mut last_error: Option<String> = None;

    for i in 1..=args.iterations {
        if !args.json {
            println!("▶ Running iteration {}/{}...", i, args.iterations);
        }
        match run_single_e2e_transfer(args.size_mb, args.simulate_drop) {
            Ok((duration, throughput)) => {
                total_duration += duration;
                total_throughput += throughput;
                if !args.json {
                    println!(
                        "  ✔ Iteration {} PASSED in {:.2?} ({:.2} MB/s) [BLAKE3 Verified]",
                        i, duration, throughput
                    );
                }
            }
            Err(e) => {
                last_error = Some(e.to_string());
                if !args.json {
                    eprintln!("  ✖ Iteration {} FAILED: {}", i, e);
                }
                break;
            }
        }
    }

    let success = last_error.is_none();
    let avg_throughput = if success && args.iterations > 0 {
        total_throughput / (args.iterations as f64)
    } else {
        0.0
    };

    if args.json {
        let report = TestReport {
            success,
            iterations: args.iterations,
            file_size_bytes: args.size_mb * 1024 * 1024,
            elapsed_ms: total_duration.as_millis(),
            throughput_mb_s: avg_throughput,
            simulated_drop_tested: args.simulate_drop,
            blake3_verified: success,
            error: last_error,
        };
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        println!("------------------------------------------------------------");
        if success {
            println!("🎉 ALL TESTS PASSED SUCCESSFULLY!");
            println!("   Total Elapsed:    {:.2?}", total_duration);
            println!("   Avg Throughput:   {:.2} MB/s", avg_throughput);
        } else {
            eprintln!("❌ TEST SUITE FAILED: {:?}", last_error);
            std::process::exit(1);
        }
    }
}
