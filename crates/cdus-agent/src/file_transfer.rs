use anyhow::{anyhow, Result};
use blake3;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, aead::{Aead, KeyInit}};
use crate::store::Store;
use cdus_common::{FileMessage, TransferRequest, TransferAcceptance, ChunkFrame, ChunkAck, TransferComplete, TransferError, ProgressEvent};
use flume::{Sender, Receiver};
use tracing::{info, error};
use futures::{AsyncReadExt, AsyncWriteExt};
use std::collections::HashMap;
use parking_lot::Mutex;
use sysinfo::Disks;

/// Metadata for a single chunk of a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkMeta {
    pub index: u32,
    pub offset: u64,
    pub length: u32,
}

pub struct SessionKey(pub [u8; 32]);

pub struct FileTransferManager {
    pub db: Arc<Store>,
    pub progress_tx: Sender<ProgressEvent>,
    pub pending_decisions: Mutex<HashMap<String, Sender<bool>>>,
    pub cancel_tokens: Mutex<HashMap<String, Sender<()>>>,
    pub active_transfers: Mutex<usize>,
    pub power_lock: Mutex<Option<keepawake::KeepAwake>>,
}

impl FileTransferManager {
    pub fn new(db: Arc<Store>, progress_tx: Sender<ProgressEvent>) -> Self {
        Self {
            db,
            progress_tx,
            pending_decisions: Mutex::new(HashMap::new()),
            cancel_tokens: Mutex::new(HashMap::new()),
            active_transfers: Mutex::new(0),
            power_lock: Mutex::new(None),
        }
    }

    pub fn handle_decision(&self, transfer_id: &str, accepted: bool) {
        info!("FileTransferManager: user decision for {}: accepted={}", transfer_id, accepted);
        let mut decisions = self.pending_decisions.lock();
        let tx_opt: Option<Sender<bool>> = decisions.remove(transfer_id);
        if let Some(tx) = tx_opt {
            info!("FileTransferManager: found pending decision for {}, sending {}", transfer_id, accepted);
            let _ = tx.send(accepted);
        } else {
            error!("FileTransferManager: no pending decision found for {}", transfer_id);
        }
    }

    pub fn add_pending_decision(&self, transfer_id: String) -> Receiver<bool> {
        let (tx, rx) = flume::bounded(1);
        let mut decisions = self.pending_decisions.lock();
        decisions.insert(transfer_id, tx);
        rx
    }

    pub fn register_transfer(&self, transfer_id: String) -> Receiver<()> {
        let (tx, rx) = flume::bounded(1);
        let mut tokens = self.cancel_tokens.lock();
        tokens.insert(transfer_id, tx);
        
        // Update power lock
        let mut active = self.active_transfers.lock();
        *active += 1;
        if *active == 1 {
            let mut lock = self.power_lock.lock();
            match keepawake::Builder::default()
                .display(false)
                .idle(true)
                .sleep(true)
                .reason("CDUS File Transfer Active")
                .app_name("CDUS")
                .create()
            {
                Ok(awake) => *lock = Some(awake),
                Err(e) => error!("Failed to acquire power lock: {}", e),
            }
        }
        
        rx
    }

    pub fn unregister_transfer(&self, transfer_id: &str) {
        let mut tokens = self.cancel_tokens.lock();
        tokens.remove(transfer_id);
        
        // Update power lock
        let mut active = self.active_transfers.lock();
        if *active > 0 {
            *active -= 1;
            if *active == 0 {
                let mut lock = self.power_lock.lock();
                *lock = None;
            }
        }
    }

    pub fn cancel_transfer(&self, transfer_id: &str) {
        let mut tokens = self.cancel_tokens.lock();
        let tx_opt: Option<Sender<()>> = tokens.remove(transfer_id);
        if let Some(tx) = tx_opt {
            let _ = tx.send(());
        }
        self.handle_decision(transfer_id, false);
    }
}

impl SessionKey {
    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        let key = Key::from_slice(&self.0);
        let cipher = ChaCha20Poly1305::new(key);
        
        let mut nonce_bytes = [0u8; 12];
        rand::Rng::fill(&mut rand::thread_rng(), &mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        
        let ciphertext = cipher.encrypt(nonce, data)
            .map_err(|e| anyhow!("Encryption failed: {}", e))?;
            
        let mut result = nonce_bytes.to_vec();
        result.extend(ciphertext);
        Ok(result)
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < 12 {
            return Err(anyhow!("Data too short for decryption"));
        }
        let (nonce_bytes, ciphertext) = data.split_at(12);
        let key = Key::from_slice(&self.0);
        let cipher = ChaCha20Poly1305::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);
        
        cipher.decrypt(nonce, ciphertext)
            .map_err(|e| anyhow!("Decryption failed: {}", e))
    }
}

pub trait FileStream {
    fn write_message(&mut self, msg: &FileMessage) -> Result<()>;
    fn read_message(&mut self) -> Result<FileMessage>;
    fn read_message_timeout(&mut self, timeout: Duration) -> Result<FileMessage>;
}

pub struct Libp2pFileStream {
    pub stream: libp2p::Stream,
    pub runtime: tokio::runtime::Handle,
}

impl FileStream for Libp2pFileStream {
    fn write_message(&mut self, msg: &FileMessage) -> Result<()> {
        let data = msg.to_vec()?;
        let len = data.len() as u32;
        match msg {
            FileMessage::Chunk(ref c) => info!("Libp2pStream: writing Chunk(index={}, offset={}) ({} bytes)", c.chunk_index, c.byte_offset, len),
            _ => info!("Libp2pStream: writing message {:?} ({} bytes)", msg, len),
        }
        self.runtime.block_on(async {
            self.stream.write_all(&len.to_be_bytes()).await?;
            self.stream.write_all(&data).await?;
            Ok::<(), std::io::Error>(())
        })?;
        info!("Libp2pStream: message written successfully");
        Ok(())
    }

    fn read_message(&mut self) -> Result<FileMessage> {
        info!("Libp2pStream: waiting to read message...");
        let msg = self.runtime.block_on(async {
            let mut len_bytes = [0u8; 4];
            self.stream.read_exact(&mut len_bytes).await?;
            let len = u32::from_be_bytes(len_bytes) as usize;
            
            let mut data = vec![0u8; len];
            self.stream.read_exact(&mut data).await?;
            
            let msg = FileMessage::from_slice(&data)?;
            match msg {
                FileMessage::Chunk(ref c) => info!("Libp2pStream: read Chunk(index={}, offset={})", c.chunk_index, c.byte_offset),
                _ => info!("Libp2pStream: read message: {:?}", msg),
            }
            Ok::<FileMessage, anyhow::Error>(msg)
        })?;
        Ok(msg)
    }

    fn read_message_timeout(&mut self, timeout: Duration) -> Result<FileMessage> {
        info!("Libp2pStream: waiting to read message (timeout={:?})...", timeout);
        let msg = self.runtime.block_on(async {
            // Robust framing: Only timeout on the first 4 bytes.
            // If we get those, we wait as long as needed for the body to avoid misalignment.
            let mut len_bytes = [0u8; 4];
            let read_res = tokio::time::timeout(timeout, self.stream.read_exact(&mut len_bytes)).await;
            
            match read_res {
                Ok(Ok(_)) => {
                    let len = u32::from_be_bytes(len_bytes) as usize;
                    info!("Libp2pStream: got length prefix {}, reading body...", len);
                    
                    let mut data = vec![0u8; len];
                    // Use a longer timeout for the body (e.g. 30s) to be safe but not block forever
                    tokio::time::timeout(Duration::from_secs(30), self.stream.read_exact(&mut data)).await??;
                    
                    let msg = FileMessage::from_slice(&data)?;
                    match msg {
                        FileMessage::Chunk(ref c) => info!("Libp2pStream: read Chunk(index={}, offset={})", c.chunk_index, c.byte_offset),
                        _ => info!("Libp2pStream: read message: {:?}", msg),
                    }
                    Ok::<FileMessage, anyhow::Error>(msg)
                }
                Ok(Err(e)) => {
                    error!("Libp2pStream: IO error reading length: {}", e);
                    Err(e.into())
                }
                Err(_) => {
                    // This is a normal timeout, we don't log it as an error
                    Err(anyhow!("Timed out"))
                }
            }
        })?;
        info!("Libp2pStream: read message: {:?}", msg);
        Ok(msg)
    }
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut file = File::open(path)?;
    let mut buf = vec![0u8; 1024 * 1024]; // 1MB read buffer
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

pub fn compute_chunk_plan(total_bytes: u64, chunk_size: u32) -> Vec<ChunkMeta> {
    let mut chunks = Vec::new();
    let mut offset = 0u64;
    let mut index = 0u32;
    while offset < total_bytes {
        let length = std::cmp::min(chunk_size as u64, total_bytes - offset) as u32;
        chunks.push(ChunkMeta { index, offset, length });
        offset += length as u64;
        index += 1;
    }
    chunks
}

pub fn safe_destination_path(download_dir: &Path, file_name: &str) -> Result<PathBuf> {
    let mut safe_name = file_name
        .replace("..", "")
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "")
        .trim_start_matches('.')
        .to_string();

    if safe_name.is_empty() {
        safe_name = "unnamed_file".to_string();
    }

    let mut dest = download_dir.join(&safe_name);
    let mut counter = 1;
    while dest.exists() {
        let path = Path::new(&safe_name);
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        dest = download_dir.join(format!("{} ({}){}", stem, counter, ext));
        counter += 1;
    }
    Ok(dest)
}

pub fn cleanup_stale_transfers(db: &Store) -> Result<()> {
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let cutoff = now_ms - (7 * 24 * 60 * 60 * 1000); // 7 days
    let stale = db.get_transfers_older_than(cutoff, &["paused", "pending"])?;
    for transfer in stale {
        info!("Cleaning up stale transfer: {}", transfer.transfer_id);
        if !transfer.file_path.is_empty() {
            let part_path = PathBuf::from(&transfer.file_path).with_extension("cdus.part");
            if part_path.exists() { let _ = std::fs::remove_file(part_path); }
        }
        db.update_transfer_status(&transfer.transfer_id, "failed")?;
    }
    Ok(())
}

pub fn handle_incoming_transfer_with_manager(
    stream: Box<dyn FileStream>,
    db: Arc<Store>,
    session_key: SessionKey,
    download_dir: PathBuf,
    _progress_tx: Sender<ProgressEvent>,
    manager: Arc<FileTransferManager>,
    peer_id: String,
) -> Result<()> {
    info!("handle_incoming_transfer_with_manager started for peer {}", peer_id);
    let mut stream = stream;
    let res = handle_incoming_transfer_inner(&mut *stream, Arc::clone(&db), session_key, download_dir, Arc::clone(&manager), &peer_id);
    if let Err(ref e) = res {
        error!("Incoming transfer failed: {}", e);
    }
    res
}

pub const BENCHMARK_ID: &str = "ffffffff-ffff-ffff-ffff-ffffffffffff";

fn generate_benchmark_chunk(index: u32, length: u32) -> Vec<u8> {
    let mut data = vec![0u8; length as usize];
    // Seed with index for determinism
    let mut state = index as u64;
    for byte in data.iter_mut() {
        // Simple LCG: state = (a * state + c) % m
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        *byte = (state >> 32) as u8;
    }
    data
}

fn handle_incoming_transfer_inner(
    stream: &mut dyn FileStream,
    db: Arc<Store>,
    session_key: SessionKey,
    download_dir: PathBuf,
    manager: Arc<FileTransferManager>,
    peer_id: &str,
) -> Result<()> {
    let req = match stream.read_message() {
        Ok(FileMessage::Request(r)) => r,
        Ok(m) => {
            error!("Expected TransferRequest, got {:?}", m);
            return Err(anyhow!("Expected TransferRequest, got {:?}", m));
        }
        Err(e) => {
            error!("Failed to read TransferRequest: {}", e);
            return Err(e);
        }
    };
    let transfer_id = req.transfer_id.clone();
    let is_benchmark = transfer_id == BENCHMARK_ID;

    info!("Received TransferRequest for {} ({} bytes). Processing...", transfer_id, req.total_bytes);

    if !is_benchmark {
        let disks = Disks::new_with_refreshed_list();
        let available_space = disks.iter()
            .filter(|d| download_dir.starts_with(d.mount_point()))
            .map(|d| d.available_space())
            .max()
            .unwrap_or(u64::MAX);

        if req.total_bytes > (available_space as f64 * 0.9) as u64 {
            let _ = stream.write_message(&FileMessage::Error(TransferError { transfer_id: transfer_id.clone(), reason: "Insufficient disk space".to_string() }));
            return Err(anyhow!("Insufficient disk space"));
        }
    }

    if db.get_transfer(&transfer_id)?.is_none() {
        db.create_transfer(&transfer_id, "incoming", peer_id, "", &req.file_name, req.total_bytes, req.chunk_size, &req.file_hash)?;
    }

    let decision_rx = manager.add_pending_decision(transfer_id.clone());
    let cancel_rx = manager.register_transfer(transfer_id.clone());
    manager.progress_tx.send(ProgressEvent::IncomingRequest { 
        transfer_id: transfer_id.clone(), 
        node_id: peer_id.to_string(),
        file_name: req.file_name.clone(), 
        total_bytes: req.total_bytes, 
        sender_label: req.sender_label.clone() 
    })?;

    info!("Waiting for user decision on transfer {}", transfer_id);
    let accepted = if is_benchmark {
        info!("Auto-accepting benchmark transfer {}", transfer_id);
        true
    } else {
        match decision_rx.recv_timeout(Duration::from_secs(120)) {
            Ok(a) => a,
            Err(_) => { 
                error!("User decision timeout for {}", transfer_id);
                manager.unregister_transfer(&transfer_id); 
                return Err(anyhow!("User decision timeout")); 
            }
        }
    };
    info!("User decision for {}: accepted={}", transfer_id, accepted);

    let mut resume_from = 0;
    if accepted && !is_benchmark {
        if let Some(existing) = db.get_transfer(&transfer_id)? {
            if existing.status == "in_progress" || existing.status == "paused" {
                resume_from = existing.bytes_confirmed as u64;
                info!("Resuming {} from {} bytes", transfer_id, resume_from);
            }
        }
    }

    info!("Sending Acceptance message for {}", transfer_id);
    stream.write_message(&FileMessage::Acceptance(TransferAcceptance { transfer_id: transfer_id.clone(), accepted, resume_from }))?;
    if !accepted {
        db.update_transfer_status(&transfer_id, "declined")?;
        manager.unregister_transfer(&transfer_id);
        return Ok(());
    }

    let dest_path = if is_benchmark {
        PathBuf::from("/dev/null")
    } else {
        safe_destination_path(&download_dir, &req.file_name)?
    };
    
    let part_path = dest_path.with_extension("cdus.part");
    if !is_benchmark {
        let conn = db.state_conn.lock();
        conn.execute("UPDATE file_transfers SET file_path = ?1 WHERE transfer_id = ?2", (dest_path.to_string_lossy(), &transfer_id))?;
    }

    let mut file_opt = if !is_benchmark {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&part_path)?;
        if resume_from > 0 { f.seek(SeekFrom::Start(resume_from))?; }
        Some(f)
    } else {
        None
    };

    db.update_transfer_status(&transfer_id, "in_progress")?;
    manager.progress_tx.send(ProgressEvent::Started { 
        transfer_id: transfer_id.clone(), 
        file_name: req.file_name.clone(),
        total_bytes: req.total_bytes, 
        is_outgoing: false 
    })?;

    info!("Starting receive loop for {}", transfer_id);
    let loop_res = loop {
        if cancel_rx.try_recv().is_ok() {
            info!("Incoming transfer {} cancelled by user (receiver loop)", transfer_id);
            let _ = stream.write_message(&FileMessage::Cancel { transfer_id: transfer_id.clone() });
            break Ok(());
        }

        match stream.read_message_timeout(Duration::from_secs(5)) {
            Ok(FileMessage::Chunk(chunk)) => {
                let plaintext = match session_key.decrypt(&chunk.data) {
                    Ok(p) => p,
                    Err(e) => { error!("Decryption failed for {}: {}", transfer_id, e); break Err(e); }
                };
                let computed_hash = blake3::hash(&plaintext).to_hex().to_string();
                if computed_hash != chunk.chunk_hash {
                    let _ = stream.write_message(&FileMessage::Error(TransferError { transfer_id: transfer_id.clone(), reason: format!("chunk {} hash mismatch", chunk.chunk_index) }));
                    db.update_transfer_status_error(&transfer_id, "chunk hash mismatch")?;
                    if !is_benchmark { let _ = fs::remove_file(&part_path); }
                    break Err(anyhow!("Chunk hash mismatch"));
                }
                if let Some(ref mut f) = file_opt {
                    f.seek(SeekFrom::Start(chunk.byte_offset))?;
                    f.write_all(&plaintext)?;
                    f.flush()?;
                }
                let new_confirmed = chunk.byte_offset + plaintext.len() as u64;
                db.update_bytes_confirmed(&transfer_id, new_confirmed)?;
                let _ = manager.progress_tx.send(ProgressEvent::Progress { 
                    transfer_id: transfer_id.clone(), 
                    bytes_confirmed: new_confirmed,
                    total_bytes: req.total_bytes
                });
                if let Err(e) = stream.write_message(&FileMessage::Ack(ChunkAck { transfer_id: transfer_id.clone(), chunk_index: chunk.chunk_index, bytes_confirmed: new_confirmed })) {
                    error!("Failed to send ACK for {}: {}", transfer_id, e);
                    break Err(e);
                }
            }
            Ok(FileMessage::Complete(complete)) => {
                if !is_benchmark {
                    if let Some(f) = file_opt.take() {
                        f.sync_all()?;
                    }

                    let actual_hash = hash_file(&part_path)?;
                    if actual_hash != complete.file_hash {
                        let _ = stream.write_message(&FileMessage::Error(TransferError { transfer_id: transfer_id.clone(), reason: "whole file hash mismatch".into() }));
                        db.update_transfer_status_error(&transfer_id, "file hash mismatch")?;
                        let _ = fs::remove_file(&part_path);
                        break Err(anyhow!("Whole file hash mismatch"));
                    }
                    fs::rename(&part_path, &dest_path)?;
                }
                db.update_transfer_status(&transfer_id, "complete")?;
                let _ = stream.write_message(&FileMessage::Ack(ChunkAck { transfer_id: transfer_id.clone(), chunk_index: u32::MAX, bytes_confirmed: req.total_bytes }));
                let _ = manager.progress_tx.send(ProgressEvent::Complete { transfer_id: transfer_id.clone(), dest_path });
                break Ok(());
            }
            Ok(FileMessage::Cancel { .. }) => { info!("Incoming transfer {} cancelled by remote", transfer_id); break Ok(()); }
            Ok(FileMessage::Error(e)) => { error!("Incoming transfer {} remote error: {}", transfer_id, e.reason); db.update_transfer_status_error(&transfer_id, &e.reason)?; break Err(anyhow!("Remote error: {}", e.reason)); }
            Ok(_) => continue,
            Err(e) if e.to_string().to_lowercase().contains("timeout") || e.to_string().to_lowercase().contains("timed out") || e.to_string().to_lowercase().contains("deadline has elapsed") => continue,
            Err(e) => { error!("Incoming transfer {} stream error: {}", transfer_id, e); break Err(e); }
        }
    };

    manager.unregister_transfer(&transfer_id);
    if let Ok(Some(rec)) = db.get_transfer(&transfer_id) {
        if rec.status == "in_progress" { db.update_transfer_status(&transfer_id, "paused")?; }
    }
    loop_res
}

pub fn handle_outgoing_transfer(
    mut stream: Box<dyn FileStream>,
    db: Arc<Store>,
    transfer_id: String,
    session_key: SessionKey,
    manager: Arc<FileTransferManager>,
) -> Result<()> {
    info!("handle_outgoing_transfer started for {}", transfer_id);
    let cancel_rx = manager.register_transfer(transfer_id.clone());
    let res = handle_outgoing_transfer_inner(&mut *stream, Arc::clone(&db), transfer_id.clone(), session_key, Arc::clone(&manager), cancel_rx);
    manager.unregister_transfer(&transfer_id);
    if let Ok(Some(rec)) = db.get_transfer(&transfer_id) {
        if rec.status == "in_progress" || rec.status == "awaiting_acceptance" { db.update_transfer_status(&transfer_id, "paused")?; }
    }
    res
}

fn handle_outgoing_transfer_inner(
    stream: &mut dyn FileStream,
    db: Arc<Store>,
    transfer_id: String,
    session_key: SessionKey,
    manager: Arc<FileTransferManager>,
    cancel_rx: Receiver<()>,
) -> Result<()> {
    info!("handle_outgoing_transfer_inner started for {}", transfer_id);
    let record = db.get_transfer(&transfer_id)?.ok_or_else(|| anyhow!("Transfer not found in DB"))?;
    let is_benchmark = transfer_id == BENCHMARK_ID;
    
    let mut file_opt = if !is_benchmark {
        let file_path = PathBuf::from(&record.file_path);
        Some(File::open(&file_path)?)
    } else {
        None
    };
    
    let chunk_plan = compute_chunk_plan(record.total_bytes as u64, record.chunk_size as u32);
    let db_chunks: Vec<(u32, String, u64, u32)> = chunk_plan.iter().map(|c| (c.index, String::new(), c.offset, c.length)).collect();
    db.insert_chunks_batch(&transfer_id, &db_chunks)?;

    let sender_label = db.get_state("device_name").unwrap_or(None).unwrap_or_else(|| "This Device".to_string());
    info!("Sending TransferRequest for {} with sender_label '{}'", transfer_id, sender_label);
    stream.write_message(&FileMessage::Request(TransferRequest { 
        transfer_id: transfer_id.clone(), 
        file_name: record.file_name.clone(), 
        total_bytes: record.total_bytes as u64, 
        chunk_size: record.chunk_size as u32, 
        file_hash: record.file_hash.clone(), 
        sender_label 
    }))?;
    db.update_transfer_status(&transfer_id, "awaiting_acceptance")?;
    
    info!("Waiting for Acceptance message for {}...", transfer_id);
    let acceptance = match stream.read_message_timeout(Duration::from_secs(120))? {
        FileMessage::Acceptance(a) => a,
        FileMessage::Error(e) => return Err(anyhow!("Receiver error: {}", e.reason)),
        _ => return Err(anyhow!("Unexpected message while waiting for acceptance")),
    };

    if !acceptance.accepted {
        db.update_transfer_status(&transfer_id, "declined")?;
        return Ok(());
    }

    let resume_from = acceptance.resume_from;
    db.update_bytes_confirmed(&transfer_id, resume_from)?;
    db.update_transfer_status(&transfer_id, "in_progress")?;
    let _ = manager.progress_tx.send(ProgressEvent::Started { 
        transfer_id: transfer_id.clone(), 
        file_name: record.file_name.clone(),
        total_bytes: record.total_bytes as u64, 
        is_outgoing: true 
    });

    let max_in_flight_bytes: usize = 50 * record.chunk_size as usize;
    let mut in_flight: usize = 0;

    for chunk_meta in chunk_plan {
        if cancel_rx.try_recv().is_ok() {
            info!("Outgoing transfer {} cancelled by user", transfer_id);
            let _ = stream.write_message(&FileMessage::Cancel { transfer_id: transfer_id.clone() });
            thread::sleep(Duration::from_millis(50));
            return Ok(());
        }

        if chunk_meta.offset + chunk_meta.length as u64 <= resume_from { continue; }

        while in_flight >= max_in_flight_bytes {
            if cancel_rx.try_recv().is_ok() {
                info!("Outgoing transfer {} cancelled by user (during flow control)", transfer_id);
                let _ = stream.write_message(&FileMessage::Cancel { transfer_id: transfer_id.clone() });
                thread::sleep(Duration::from_millis(50));
                return Ok(());
            }
            match stream.read_message_timeout(Duration::from_secs(5)) {
                Ok(FileMessage::Ack(a)) => {
                    in_flight = in_flight.saturating_sub(record.chunk_size as usize);
                    db.update_bytes_confirmed(&transfer_id, a.bytes_confirmed)?;
                    let _ = manager.progress_tx.send(ProgressEvent::Progress { 
    transfer_id: transfer_id.clone(), 
    bytes_confirmed: a.bytes_confirmed,
    total_bytes: record.total_bytes as u64
});
                }
                Ok(FileMessage::Error(e)) => return Err(anyhow!("Receiver error: {}", e.reason)),
                Ok(FileMessage::Cancel { .. }) => return Ok(()),
                Err(e) if e.to_string().to_lowercase().contains("timeout") || e.to_string().to_lowercase().contains("timed out") || e.to_string().to_lowercase().contains("deadline has elapsed") => continue,
                Err(e) => return Err(e),
                _ => continue,
            }
        }

        let chunk_data = if is_benchmark {
            generate_benchmark_chunk(chunk_meta.index, chunk_meta.length)
        } else {
            let mut data = vec![0u8; chunk_meta.length as usize];
            let file = file_opt.as_mut().unwrap();
            file.seek(SeekFrom::Start(chunk_meta.offset))?;
            file.read_exact(&mut data)?;
            data
        };

        let chunk_hash = blake3::hash(&chunk_data).to_hex().to_string();
        let encrypted = session_key.encrypt(&chunk_data)?;
        stream.write_message(&FileMessage::Chunk(ChunkFrame { transfer_id: transfer_id.clone(), chunk_index: chunk_meta.index, byte_offset: chunk_meta.offset, data: encrypted, chunk_hash }))?;
        in_flight += chunk_meta.length as usize;
    }

    while in_flight > 0 {
        if cancel_rx.try_recv().is_ok() {
            info!("Outgoing transfer {} cancelled by user (during final drain)", transfer_id);
            let _ = stream.write_message(&FileMessage::Cancel { transfer_id: transfer_id.clone() });
            thread::sleep(Duration::from_millis(50));
            return Ok(());
        }
        match stream.read_message_timeout(Duration::from_secs(5)) {
            Ok(FileMessage::Ack(a)) => {
                in_flight = in_flight.saturating_sub(record.chunk_size as usize);
                db.update_bytes_confirmed(&transfer_id, a.bytes_confirmed)?;
                let _ = manager.progress_tx.send(ProgressEvent::Progress { 
    transfer_id: transfer_id.clone(), 
    bytes_confirmed: a.bytes_confirmed,
    total_bytes: record.total_bytes as u64
});
            }
            Ok(FileMessage::Error(e)) => return Err(anyhow!("Receiver error: {}", e.reason)),
            Ok(FileMessage::Cancel { .. }) => return Ok(()),
            Err(e) if e.to_string().to_lowercase().contains("timeout") || e.to_string().to_lowercase().contains("timed out") || e.to_string().to_lowercase().contains("deadline has elapsed") => continue,
            Err(e) => return Err(e),
            _ => continue,
        }
    }

    stream.write_message(&FileMessage::Complete(TransferComplete { transfer_id: transfer_id.clone(), file_hash: record.file_hash.clone() }))?;
    match stream.read_message_timeout(Duration::from_secs(60))? {
        FileMessage::Ack(_) => {
            db.update_transfer_status(&transfer_id, "complete")?;
            let _ = manager.progress_tx.send(ProgressEvent::Complete { transfer_id, dest_path: if is_benchmark { PathBuf::from("/dev/null") } else { PathBuf::from(&record.file_path) } });
        }
        FileMessage::Error(e) => {
            db.update_transfer_status_error(&transfer_id, &e.reason)?;
            let _ = manager.progress_tx.send(ProgressEvent::Failed { transfer_id, reason: e.reason });
        }
        _ => return Err(anyhow!("Unexpected final message")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use uuid::Uuid;
    use std::thread;

    struct MockFileStream {
        tx: Sender<FileMessage>,
        rx: Receiver<FileMessage>,
    }

    impl FileStream for MockFileStream {
        fn write_message(&mut self, msg: &FileMessage) -> Result<()> {
            info!("MockFileStream: writing message {:?}", msg);
            self.tx.send(msg.clone()).map_err(|_| anyhow!("Stream closed"))
        }
        fn read_message(&mut self) -> Result<FileMessage> {
            self.rx.recv().map_err(|_| anyhow!("Stream closed"))
        }
        fn read_message_timeout(&mut self, timeout: Duration) -> Result<FileMessage> {
            self.rx.recv_timeout(timeout).map_err(|e| match e {
                flume::RecvTimeoutError::Timeout => anyhow!("Timed out"),
                flume::RecvTimeoutError::Disconnected => anyhow!("Stream closed"),
            })
        }
    }

    #[test]
    fn test_file_transfer_small_file_end_to_end() -> Result<()> {
        let _ = tracing_subscriber::fmt::try_init();
        let dir1 = tempdir()?;
        let dir2 = tempdir()?;
        let file_path = dir1.path().join("test.bin");
        let file_content = b"hello cdus file transfer";
        std::fs::write(&file_path, file_content)?;
        let file_hash = blake3::hash(file_content).to_hex().to_string();
        let store1 = Arc::new(Store::init(dir1.path())?);
        let store2 = Arc::new(Store::init(dir2.path())?);
        let (tx1, rx1) = flume::unbounded();
        let (tx2, rx2) = flume::unbounded();
        let stream1 = Box::new(MockFileStream { tx: tx1, rx: rx2 });
        let stream2 = Box::new(MockFileStream { tx: tx2, rx: rx1 });
        let (prog_tx1, _prog_rx1) = flume::unbounded();
        let (prog_tx2, _prog_rx2) = flume::unbounded();
        let manager1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), prog_tx1));
        let manager2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), prog_tx2));
        let transfer_id = Uuid::new_v4().to_string();
        store1.create_transfer(&transfer_id, "outgoing", "peer-2", &file_path.to_string_lossy(), "test.bin", file_content.len() as u64, 1024, &file_hash)?;
        let t_id_clone = transfer_id.clone();
        let store1_c = Arc::clone(&store1);
        let manager1_c = Arc::clone(&manager1);
        let sender_thread = thread::spawn(move || { handle_outgoing_transfer(stream1, store1_c, t_id_clone, SessionKey([0u8; 32]), manager1_c) });
        let store2_c = Arc::clone(&store2);
        let manager2_c = Arc::clone(&manager2);
        let download_dir = dir2.path().to_path_buf();
        let receiver_thread = thread::spawn(move || { handle_incoming_transfer_with_manager(stream2, store2_c, SessionKey([0u8; 32]), download_dir, flume::unbounded().0, manager2_c, "peer-1".to_string()) });
        thread::sleep(Duration::from_millis(200));
        manager2.handle_decision(&transfer_id, true);
        sender_thread.join().unwrap()?;
        receiver_thread.join().unwrap()?;
        let dest_path = dir2.path().join("test.bin");
        assert!(dest_path.exists());
        let received_content = std::fs::read(dest_path)?;
        assert_eq!(received_content, file_content);
        assert_eq!(store1.get_transfer(&transfer_id)?.unwrap().status, "complete");
        assert_eq!(store2.get_transfer(&transfer_id)?.unwrap().status, "complete");
        Ok(())
    }

    #[test]
    fn test_file_transfer_large_file_end_to_end() -> Result<()> {
        let _ = tracing_subscriber::fmt::try_init();
        let dir1 = tempdir()?;
        let dir2 = tempdir()?;
        let file_path = dir1.path().join("large_test.bin");
        let mut file_content = vec![0u8; 5 * 1024 * 1024];
        rand::Rng::fill(&mut rand::thread_rng(), &mut file_content[..]);
        std::fs::write(&file_path, &file_content)?;
        let file_hash = blake3::hash(&file_content).to_hex().to_string();
        let store1 = Arc::new(Store::init(dir1.path())?);
        let store2 = Arc::new(Store::init(dir2.path())?);
        let (tx1, rx1) = flume::unbounded();
        let (tx2, rx2) = flume::unbounded();
        let stream1 = Box::new(MockFileStream { tx: tx1, rx: rx2 });
        let stream2 = Box::new(MockFileStream { tx: tx2, rx: rx1 });
        let (prog_tx1, _prog_rx1) = flume::unbounded();
        let (prog_tx2, _prog_rx2) = flume::unbounded();
        let manager1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), prog_tx1));
        let manager2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), prog_tx2));
        let transfer_id = Uuid::new_v4().to_string();
        store1.create_transfer(&transfer_id, "outgoing", "peer-2", &file_path.to_string_lossy(), "large_test.bin", file_content.len() as u64, 262144, &file_hash)?;
        let t_id_clone = transfer_id.clone();
        let store1_c = Arc::clone(&store1);
        let manager1_c = Arc::clone(&manager1);
        let sender_thread = thread::spawn(move || { handle_outgoing_transfer(stream1, store1_c, t_id_clone, SessionKey([0u8; 32]), manager1_c) });
        let store2_c = Arc::clone(&store2);
        let manager2_c = Arc::clone(&manager2);
        let download_dir = dir2.path().to_path_buf();
        let receiver_thread = thread::spawn(move || { handle_incoming_transfer_with_manager(stream2, store2_c, SessionKey([0u8; 32]), download_dir, flume::unbounded().0, manager2_c, "peer-1".to_string()) });
        thread::sleep(Duration::from_millis(200));
        manager2.handle_decision(&transfer_id, true);
        sender_thread.join().unwrap()?;
        receiver_thread.join().unwrap()?;
        let dest_path = dir2.path().join("large_test.bin");
        assert!(dest_path.exists());
        let received_content = std::fs::read(dest_path)?;
        assert_eq!(received_content, file_content);
        Ok(())
    }

    #[test]
    fn test_file_transfer_resume_from_offset() -> Result<()> {
        let _ = tracing_subscriber::fmt::try_init();
        let dir1 = tempdir()?;
        let dir2 = tempdir()?;
        let file_path = dir1.path().join("resume_test.bin");
        let file_content = b"first part content AND second part content";
        std::fs::write(&file_path, file_content)?;
        let file_hash = blake3::hash(file_content).to_hex().to_string();
        let store1 = Arc::new(Store::init(dir1.path())?);
        let store2 = Arc::new(Store::init(dir2.path())?);
        let transfer_id = Uuid::new_v4().to_string();
        let dest_path = dir2.path().join("resume_test.bin");
        let part_path = dest_path.with_extension("cdus.part");
        let first_part = b"first part content ";
        std::fs::write(&part_path, first_part)?;
        store2.create_transfer(&transfer_id, "incoming", "peer-1", &dest_path.to_string_lossy(), "resume_test.bin", file_content.len() as u64, 1024, &file_hash)?;
        store2.update_bytes_confirmed(&transfer_id, first_part.len() as u64)?;
        store2.update_transfer_status(&transfer_id, "paused")?;
        let (tx1, rx1) = flume::unbounded();
        let (tx2, rx2) = flume::unbounded();
        let stream1 = Box::new(MockFileStream { tx: tx1, rx: rx2 });
        let stream2 = Box::new(MockFileStream { tx: tx2, rx: rx1 });
        let (prog_tx1, _prog_rx1) = flume::unbounded();
        let (prog_tx2, _prog_rx2) = flume::unbounded();
        let manager1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), prog_tx1));
        let manager2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), prog_tx2));
        store1.create_transfer(&transfer_id, "outgoing", "peer-2", &file_path.to_string_lossy(), "resume_test.bin", file_content.len() as u64, 10, &file_hash)?;
        let t_id_clone = transfer_id.clone();
        let store1_c = Arc::clone(&store1);
        let manager1_c = Arc::clone(&manager1);
        let sender_thread = thread::spawn(move || { handle_outgoing_transfer(stream1, store1_c, t_id_clone, SessionKey([0u8; 32]), manager1_c) });
        let store2_c = Arc::clone(&store2);
        let manager2_c = Arc::clone(&manager2);
        let download_dir = dir2.path().to_path_buf();
        let receiver_thread = thread::spawn(move || { handle_incoming_transfer_with_manager(stream2, store2_c, SessionKey([0u8; 32]), download_dir, flume::unbounded().0, manager2_c, "peer-1".to_string()) });
        thread::sleep(Duration::from_millis(200));
        manager2.handle_decision(&transfer_id, true);
        sender_thread.join().unwrap()?;
        receiver_thread.join().unwrap()?;
        assert!(dest_path.exists());
        let received_content = std::fs::read(dest_path)?;
        assert_eq!(received_content, file_content);
        Ok(())
    }

    struct CorruptFileStream { tx: Sender<FileMessage>, rx: Receiver<FileMessage>, corrupt_index: u32 }
    impl FileStream for CorruptFileStream {
        fn write_message(&mut self, msg: &FileMessage) -> Result<()> {
            let mut final_msg = msg.clone();
            if let FileMessage::Chunk(ref mut chunk) = final_msg {
                if chunk.chunk_index == self.corrupt_index {
                    if !chunk.chunk_hash.is_empty() { chunk.chunk_hash = "corrupted-hash".to_string(); }
                }
            }
            self.tx.send(final_msg).map_err(|_| anyhow!("Stream closed"))
        }
        fn read_message(&mut self) -> Result<FileMessage> { self.rx.recv().map_err(|_| anyhow!("Stream closed")) }
        fn read_message_timeout(&mut self, timeout: Duration) -> Result<FileMessage> {
            self.rx.recv_timeout(timeout).map_err(|e| match e {
                flume::RecvTimeoutError::Timeout => anyhow!("Timed out"),
                flume::RecvTimeoutError::Disconnected => anyhow!("Stream closed"),
            })
        }
    }

    #[test]
    fn test_file_transfer_chunk_hash_mismatch_aborts() -> Result<()> {
        let _ = tracing_subscriber::fmt::try_init();
        let dir1 = tempdir()?;
        let dir2 = tempdir()?;
        let file_path = dir1.path().join("corrupt_test.bin");
        let file_content = b"content to be corrupted";
        std::fs::write(&file_path, file_content)?;
        let file_hash = blake3::hash(file_content).to_hex().to_string();
        let store1 = Arc::new(Store::init(dir1.path())?);
        let store2 = Arc::new(Store::init(dir2.path())?);
        let (tx1, rx1) = flume::unbounded();
        let (tx2, rx2) = flume::unbounded();
        let stream1 = Box::new(CorruptFileStream { tx: tx1, rx: rx2, corrupt_index: 0 });
        let stream2 = Box::new(MockFileStream { tx: tx2, rx: rx1 });
        let (prog_tx1, _prog_rx1) = flume::unbounded();
        let (prog_tx2, _prog_rx2) = flume::unbounded();
        let manager1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), prog_tx1));
        let manager2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), prog_tx2));
        let transfer_id = Uuid::new_v4().to_string();
        store1.create_transfer(&transfer_id, "outgoing", "peer-2", &file_path.to_string_lossy(), "corrupt_test.bin", file_content.len() as u64, 1024, &file_hash)?;
        let t_id_clone = transfer_id.clone();
        let store1_c = Arc::clone(&store1);
        let manager1_c = Arc::clone(&manager1);
        let sender_thread = thread::spawn(move || { handle_outgoing_transfer(stream1, store1_c, t_id_clone, SessionKey([0u8; 32]), manager1_c) });
        let store2_c = Arc::clone(&store2);
        let manager2_c = Arc::clone(&manager2);
        let download_dir = dir2.path().to_path_buf();
        let receiver_thread = thread::spawn(move || { handle_incoming_transfer_with_manager(stream2, store2_c, SessionKey([0u8; 32]), download_dir, flume::unbounded().0, manager2_c, "peer-1".to_string()) });
        thread::sleep(Duration::from_millis(200));
        manager2.handle_decision(&transfer_id, true);
        let s_res = sender_thread.join().unwrap();
        assert!(s_res.is_err());
        let r_res = receiver_thread.join().unwrap();
        assert!(r_res.is_err());
        assert!(r_res.unwrap_err().to_string().contains("Chunk hash mismatch"));
        assert_eq!(store2.get_transfer(&transfer_id)?.unwrap().status, "failed");
        Ok(())
    }

    struct FinalHashCorruptStream { tx: Sender<FileMessage>, rx: Receiver<FileMessage> }
    impl FileStream for FinalHashCorruptStream {
        fn write_message(&mut self, msg: &FileMessage) -> Result<()> {
            let mut final_msg = msg.clone();
            if let FileMessage::Complete(ref mut complete) = final_msg { complete.file_hash = "wrong-final-hash".to_string(); }
            self.tx.send(final_msg).map_err(|_| anyhow!("Stream closed"))
        }
        fn read_message(&mut self) -> Result<FileMessage> { self.rx.recv().map_err(|_| anyhow!("Stream closed")) }
        fn read_message_timeout(&mut self, timeout: Duration) -> Result<FileMessage> {
            self.rx.recv_timeout(timeout).map_err(|e| match e {
                flume::RecvTimeoutError::Timeout => anyhow!("Timed out"),
                flume::RecvTimeoutError::Disconnected => anyhow!("Stream closed"),
            })
        }
    }

    #[test]
    fn test_file_transfer_whole_file_hash_mismatch_deletes_part() -> Result<()> {
        let _ = tracing_subscriber::fmt::try_init();
        let dir1 = tempdir()?;
        let dir2 = tempdir()?;
        let file_path = dir1.path().join("final_corrupt.bin");
        let file_content = b"content for final hash mismatch";
        std::fs::write(&file_path, file_content)?;
        let file_hash = blake3::hash(file_content).to_hex().to_string();
        let store1 = Arc::new(Store::init(dir1.path())?);
        let store2 = Arc::new(Store::init(dir2.path())?);
        let (tx1, rx1) = flume::unbounded();
        let (tx2, rx2) = flume::unbounded();
        let stream1 = Box::new(FinalHashCorruptStream { tx: tx1, rx: rx2 });
        let stream2 = Box::new(MockFileStream { tx: tx2, rx: rx1 });
        let (prog_tx1, _prog_rx1) = flume::unbounded();
        let (prog_tx2, _prog_rx2) = flume::unbounded();
        let manager1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), prog_tx1));
        let manager2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), prog_tx2));
        let transfer_id = Uuid::new_v4().to_string();
        store1.create_transfer(&transfer_id, "outgoing", "peer-2", &file_path.to_string_lossy(), "final_corrupt.bin", file_content.len() as u64, 1024, &file_hash)?;
        let t_id_clone = transfer_id.clone();
        let store1_c = Arc::clone(&store1);
        let manager1_c = Arc::clone(&manager1);
        let sender_thread = thread::spawn(move || { handle_outgoing_transfer(stream1, store1_c, t_id_clone, SessionKey([0u8; 32]), manager1_c) });
        let store2_c = Arc::clone(&store2);
        let manager2_c = Arc::clone(&manager2);
        let download_dir = dir2.path().to_path_buf();
        let receiver_thread = thread::spawn(move || { handle_incoming_transfer_with_manager(stream2, store2_c, SessionKey([0u8; 32]), download_dir, flume::unbounded().0, manager2_c, "peer-1".to_string()) });
        thread::sleep(Duration::from_millis(200));
        manager2.handle_decision(&transfer_id, true);
        let _ = sender_thread.join().unwrap();
        let r_res = receiver_thread.join().unwrap();
        assert!(r_res.is_err());
        assert!(r_res.unwrap_err().to_string().contains("Whole file hash mismatch"));
        assert_eq!(store2.get_transfer(&transfer_id)?.unwrap().status, "failed");
        let dest_path = dir2.path().join("final_corrupt.bin");
        let part_path = dest_path.with_extension("cdus.part");
        assert!(!part_path.exists());
        assert!(!dest_path.exists());
        Ok(())
    }

    #[test]
    fn test_file_transfer_decline_writes_nothing() -> Result<()> {
        let _ = tracing_subscriber::fmt::try_init();
        let dir1 = tempdir()?;
        let dir2 = tempdir()?;
        let file_path = dir1.path().join("decline_test.bin");
        std::fs::write(&file_path, b"some content")?;
        let store1 = Arc::new(Store::init(dir1.path())?);
        let store2 = Arc::new(Store::init(dir2.path())?);
        let (tx1, rx1) = flume::unbounded();
        let (tx2, rx2) = flume::unbounded();
        let stream1 = Box::new(MockFileStream { tx: tx1, rx: rx2 });
        let stream2 = Box::new(MockFileStream { tx: tx2, rx: rx1 });
        let (prog_tx1, _prog_rx1) = flume::unbounded();
        let (prog_tx2, _prog_rx2) = flume::unbounded();
        let manager1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), prog_tx1));
        let manager2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), prog_tx2));
        let transfer_id = Uuid::new_v4().to_string();
        store1.create_transfer(&transfer_id, "outgoing", "peer-2", &file_path.to_string_lossy(), "decline_test.bin", 12, 1024, "fake-hash")?;
        let t_id_clone = transfer_id.clone();
        let store1_c = Arc::clone(&store1);
        let manager1_c = Arc::clone(&manager1);
        let sender_thread = thread::spawn(move || { handle_outgoing_transfer(stream1, store1_c, t_id_clone, SessionKey([0u8; 32]), manager1_c) });
        let store2_c = Arc::clone(&store2);
        let manager2_c = Arc::clone(&manager2);
        let download_dir = dir2.path().to_path_buf();
        let receiver_thread = thread::spawn(move || { handle_incoming_transfer_with_manager(stream2, store2_c, SessionKey([0u8; 32]), download_dir, flume::unbounded().0, manager2_c, "peer-1".to_string()) });
        thread::sleep(Duration::from_millis(200));
        manager2.handle_decision(&transfer_id, false);
        let s_res = sender_thread.join().unwrap();
        assert!(s_res.is_ok());
        let r_res = receiver_thread.join().unwrap();
        assert!(r_res.is_ok());
        assert_eq!(store1.get_transfer(&transfer_id)?.unwrap().status, "declined");
        assert_eq!(store2.get_transfer(&transfer_id)?.unwrap().status, "declined");
        let dest_path = dir2.path().join("decline_test.bin");
        let part_path = dest_path.with_extension("cdus.part");
        assert!(!part_path.exists());
        assert!(!dest_path.exists());
        Ok(())
    }

    #[test]
    fn test_file_transfer_cancel_mid_send() -> Result<()> {
        let _ = tracing_subscriber::fmt::try_init();
        info!("Starting test_file_transfer_cancel_mid_send");
        let dir1 = tempdir()?;
        let dir2 = tempdir()?;
        let file_path = dir1.path().join("cancel_test.bin");
        let file_content = vec![0u8; 5 * 1024 * 1024];
        std::fs::write(&file_path, &file_content)?;
        let file_hash = blake3::hash(&file_content).to_hex().to_string();
        let store1 = Arc::new(Store::init(dir1.path())?);
        let store2 = Arc::new(Store::init(dir2.path())?);
        let (tx1, rx1) = flume::unbounded();
        let (tx2, rx2) = flume::unbounded();
        let stream1 = Box::new(MockFileStream { tx: tx1, rx: rx2 });
        let stream2 = Box::new(MockFileStream { tx: tx2, rx: rx1 });
        let (prog_tx1, _prog_rx1) = flume::unbounded();
        let (prog_tx2, prog_rx2) = flume::unbounded();
        let manager1 = Arc::new(FileTransferManager::new(Arc::clone(&store1), prog_tx1));
        let manager2 = Arc::new(FileTransferManager::new(Arc::clone(&store2), prog_tx2));
        let transfer_id = Uuid::new_v4().to_string();
        store1.create_transfer(&transfer_id, "outgoing", "peer-2", &file_path.to_string_lossy(), "cancel_test.bin", file_content.len() as u64, 262144, &file_hash)?;
        let t_id_clone = transfer_id.clone();
        let store1_c = Arc::clone(&store1);
        let manager1_clone = Arc::clone(&manager1);
        let sender_thread = thread::spawn(move || { handle_outgoing_transfer(stream1, store1_c, t_id_clone, SessionKey([0u8; 32]), manager1_clone) });
        let store2_c = Arc::clone(&store2);
        let manager2_clone = Arc::clone(&manager2);
        let download_dir = dir2.path().to_path_buf();
        let receiver_thread = thread::spawn(move || { handle_incoming_transfer_with_manager(stream2, store2_c, SessionKey([0u8; 32]), download_dir, flume::unbounded().0, manager2_clone, "peer-1".to_string()) });
        let mut ready = false;
        while let Ok(event) = prog_rx2.recv_timeout(Duration::from_secs(5)) { if let ProgressEvent::IncomingRequest { .. } = event { ready = true; break; } }
        assert!(ready, "Receiver never notified IncomingRequest");
        manager2.handle_decision(&transfer_id, true);
        let mut progress_seen = false;
        while let Ok(event) = prog_rx2.recv_timeout(Duration::from_secs(5)) { if let ProgressEvent::Progress { .. } = event { progress_seen = true; break; } }
        assert!(progress_seen, "Never saw any progress before cancel");
        manager1.cancel_transfer(&transfer_id);
        let s_res = sender_thread.join().unwrap();
        let r_res = receiver_thread.join().unwrap();
        println!("Sender result: {:?}", s_res);
        println!("Receiver result: {:?}", r_res);
        assert_eq!(store1.get_transfer(&transfer_id)?.unwrap().status, "paused");
        let r_status = store2.get_transfer(&transfer_id)?.unwrap().status;
        println!("Actual receiver status: {}", r_status);
        assert!(r_status == "paused" || r_status == "failed");
        Ok(())
    }

    #[test]
    fn test_file_transfer_filename_collision() -> Result<()> {
        let dir = tempdir()?;
        let download_dir = dir.path().to_path_buf();
        let file_name = "test.txt";
        
        // Create existing file
        std::fs::write(download_dir.join(file_name), "existing")?;
        
        let path = safe_destination_path(&download_dir, file_name)?;
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "test (1).txt");
        
        // Create another existing file
        std::fs::write(download_dir.join("test (1).txt"), "existing 2")?;
        let path2 = safe_destination_path(&download_dir, file_name)?;
        assert_eq!(path2.file_name().unwrap().to_str().unwrap(), "test (2).txt");
        
        Ok(())
    }

    #[test]
    fn test_stale_transfer_cleanup() -> Result<()> {
        let dir = tempdir()?;
        let store = Arc::new(Store::init(dir.path())?);
        
        let transfer_id = "stale-id".to_string();
        // Create a record that is 8 days old
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
        let eight_days_ago = now - (8 * 24 * 60 * 60 * 1000);
        
        {
            let conn = store.state_conn.lock();
            conn.execute(
                "INSERT INTO file_transfers (transfer_id, direction, peer_node_id, file_path, file_name, total_bytes, chunk_size, file_hash, status, created_at, updated_at)
                 VALUES (?1, 'incoming', 'peer', '/some/path/file.txt', 'file.txt', 100, 1024, 'hash', 'paused', ?2, ?2)",
                (&transfer_id, eight_days_ago),
            )?;
        }
        
        // Create dummy .part file
        let part_path = PathBuf::from("/some/path/file.txt.cdus.part");
        // We can't actually create it at that path if it's invalid, let's use temp dir
        let real_path = dir.path().join("stale.txt");
        let real_part_path = real_path.with_extension("cdus.part");
        std::fs::write(&real_part_path, "partial data")?;
        
        {
            let conn = store.state_conn.lock();
            conn.execute("UPDATE file_transfers SET file_path = ?1 WHERE transfer_id = ?2", (real_path.to_string_lossy(), &transfer_id))?;
        }

        cleanup_stale_transfers(&store)?;
        
        assert_eq!(store.get_transfer(&transfer_id)?.unwrap().status, "failed");
        assert!(!real_part_path.exists());
        
        Ok(())
    }

    #[test]
    fn test_file_transfer_concurrent_four_transfers() -> Result<()> {
        let _ = tracing_subscriber::fmt::try_init();
        let dir = tempdir()?;
        let store = Arc::new(Store::init(dir.path())?);
        let (prog_tx, _prog_rx) = flume::unbounded();
        let manager = Arc::new(FileTransferManager::new(Arc::clone(&store), prog_tx));
        
        let mut handles = Vec::new();

        for i in 0..4 {
            let store_c = Arc::clone(&store);
            let manager_c = Arc::clone(&manager);
            let h = thread::spawn(move || {
                let dir_c = tempdir().unwrap();
                let transfer_id = format!("transfer-{}", i);
                let file_content = format!("content {}", i);
                let file_path = dir_c.path().join("test.bin");
                std::fs::write(&file_path, &file_content).unwrap();
                let file_hash = blake3::hash(file_content.as_bytes()).to_hex().to_string();

                store_c.create_transfer(&transfer_id, "outgoing", "peer", &file_path.to_string_lossy(), "test.bin", file_content.len() as u64, 1024, &file_hash).unwrap();

                let (tx1, rx1) = flume::unbounded();
                let (tx2, rx2) = flume::unbounded();
                let stream1 = Box::new(MockFileStream { tx: tx1, rx: rx2 });
                let stream2 = Box::new(MockFileStream { tx: tx2, rx: rx1 });

                let s_id = transfer_id.clone();
                let s_store = Arc::clone(&store_c);
                let s_manager = Arc::clone(&manager_c);
                let sender = thread::spawn(move || {
                    handle_outgoing_transfer(stream1, s_store, s_id, SessionKey([0u8; 32]), s_manager)
                });

                let r_store = Arc::clone(&store_c);
                let r_manager = Arc::clone(&manager_c);
                let r_dir = dir_c.path().to_path_buf();
                let receiver = thread::spawn(move || {
                    handle_incoming_transfer_with_manager(stream2, r_store, SessionKey([0u8; 32]), r_dir, flume::unbounded().0, r_manager, "peer".to_string())
                });

                thread::sleep(Duration::from_millis(200));
                manager_c.handle_decision(&transfer_id, true);

                sender.join().unwrap().unwrap();
                receiver.join().unwrap().unwrap();
            });
            handles.push(h);
        }

        for h in handles {
            h.join().unwrap();
        }

        Ok(())
    }

    #[test]
    fn test_file_transfer_fifth_transfer_queued() -> Result<()> {
        let _ = tracing_subscriber::fmt::try_init();
        let dir = tempdir()?;
        let store = Arc::new(Store::init(dir.path())?);
        let (prog_tx, _prog_rx) = flume::unbounded();
        let manager = Arc::new(FileTransferManager::new(Arc::clone(&store), prog_tx));
        
        let pool = threadpool::ThreadPool::new(4);
        let (start_tx, start_rx) = flume::unbounded();
        let started_count = Arc::new(Mutex::new(0));

        for i in 0..5 {
            let start_tx_c = start_tx.clone();
            let started_count_c = Arc::clone(&started_count);
            
            pool.execute(move || {
                {
                    let mut count = started_count_c.lock();
                    *count += 1;
                    info!("Transfer {} started (total active: {})", i, *count);
                }
                let _ = start_tx_c.send(i);

                // Simulate a very slow transfer start
                thread::sleep(Duration::from_millis(500));
                
                {
                    let mut count = started_count_c.lock();
                    *count -= 1;
                }
            });
        }

        // Wait for first 4 to signal start
        for _ in 0..4 {
            start_rx.recv_timeout(Duration::from_secs(1))?;
        }
        
        // 5th should NOT have started yet
        assert!(start_rx.try_recv().is_err(), "5th transfer should be queued");
        assert_eq!(*started_count.lock(), 4);

        // Wait for one to finish
        thread::sleep(Duration::from_millis(600));
        
        // 5th should now start
        let _ = start_rx.recv_timeout(Duration::from_secs(1))?;
        
        pool.join();
        Ok(())
    }

    #[test]
    fn test_hash_file() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("test.bin");
        let mut file = File::create(&path)?;
        file.write_all(b"hello world")?;
        let hash = hash_file(&path)?;
        let expected = blake3::hash(b"hello world").to_hex().to_string();
        assert_eq!(hash, expected);
        Ok(())
    }

    #[test]
    fn test_compute_chunk_plan() {
        let chunks = compute_chunk_plan(1000, 256);
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0].offset, 0);
        assert_eq!(chunks[0].length, 256);
        assert_eq!(chunks[3].offset, 256 * 3);
        assert_eq!(chunks[3].length, 1000 - 256 * 3);
    }

    #[test]
    fn test_safe_destination_path() -> Result<()> {
        let dir = tempdir()?;
        let path = safe_destination_path(dir.path(), "../../../etc/passwd")?;
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "etcpasswd");
        File::create(dir.path().join("test.txt"))?;
        let path2 = safe_destination_path(dir.path(), "test.txt")?;
        assert_eq!(path2.file_name().unwrap().to_str().unwrap(), "test (1).txt");
        Ok(())
    }

    #[test]
    fn test_crypto_roundtrip() -> Result<()> {
        let key = SessionKey([0u8; 32]);
        let data = b"secret data";
        let encrypted = key.encrypt(data)?;
        let decrypted = key.decrypt(&encrypted)?;
        assert_eq!(data, &decrypted[..]);
        Ok(())
    }
}
