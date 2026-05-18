use anyhow::{anyhow, Result};
use blake3;
use cdus_common::{FileChunk, FileManifest};
use fastcdc::v2020::StreamCDC;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Generates a manifest for a file using Content-Defined Chunking (FastCDC).
pub fn generate_manifest<F>(path: &Path, progress_callback: F) -> Result<FileManifest> 
where F: Fn(f32) {
    let mut file = File::open(path)?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("Invalid file name"))?
        .to_string();

    let metadata = file.metadata()?;
    let total_size = metadata.len();

    // Pass 1: Total file hash for integrity verification
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 65536];
    let mut read_bytes = 0u64;
    
    progress_callback(0.0);
    
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        read_bytes += n as u64;
        
        // Report up to 50%
        if total_size > 0 {
            let p = (read_bytes as f32 / total_size as f32) * 50.0;
            progress_callback(p);
        }
    }
    let file_hash = hasher.finalize().to_string();
    progress_callback(50.0);

    // Pass 2: Chunking and chunk hashes
    let chunker_file = File::open(path)?;

    // Constraints for better progress granularity (Avg 64KB chunks)
    let min_size = 16 * 1024; // 16 KB
    let avg_size = 64 * 1024; // 64 KB
    let max_size = 128 * 1024; // 128 KB

    let chunker = StreamCDC::new(chunker_file, min_size, avg_size, max_size);


    let mut data_file = File::open(path)?;
    let mut chunks = Vec::new();
    let mut chunked_bytes = 0u64;

    for chunk_res in chunker {
        let chunk = chunk_res?;
        let current_pos = data_file.stream_position()?;
        if current_pos != chunk.offset as u64 {
            data_file.seek(SeekFrom::Start(chunk.offset as u64))?;
        }

        let mut chunk_data = vec![0u8; chunk.length];
        data_file.read_exact(&mut chunk_data)?;
        let hash = blake3::hash(&chunk_data).to_string();

        chunks.push(FileChunk {
            hash,
            offset: chunk.offset as u64,
            size: chunk.length as u32,
        });
        
        chunked_bytes += chunk.length as u64;
        if total_size > 0 {
            let p = 50.0 + (chunked_bytes as f32 / total_size as f32) * 50.0;
            progress_callback(p);
        }
    }

    progress_callback(100.0);

    Ok(FileManifest {
        file_hash,
        file_name,
        total_size,
        chunks,
    })
}

/// Retrieves a specific chunk of data from a file.
pub fn get_chunk(path: &Path, offset: u64, size: u32) -> Result<Vec<u8>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut buffer = vec![0u8; size as usize];
    file.read_exact(&mut buffer)?;
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_manifest_generation() -> Result<()> {
        let mut tmp_file = NamedTempFile::new()?;
        let data = vec![0u8; 5 * 1024 * 1024]; // 5MB of zeros
        tmp_file.write_all(&data)?;

        let manifest = generate_manifest(tmp_file.path())?;

        assert_eq!(manifest.total_size, 5 * 1024 * 1024);
        assert!(!manifest.chunks.is_empty());

        // Verify total size from chunks
        let chunks_total: u64 = manifest.chunks.iter().map(|c| c.size as u64).sum();
        assert_eq!(chunks_total, 5 * 1024 * 1024);

        Ok(())
    }

    #[test]
    fn test_chunk_granularity() -> Result<()> {
        let mut tmp_file = NamedTempFile::new()?;
        let data = vec![0u8; 10 * 1024 * 1024]; // 10MB
        tmp_file.write_all(&data)?;

        let manifest = generate_manifest(tmp_file.path())?;
        println!("Chunks for 10MB: {}", manifest.chunks.len());
        
        // With 64KB avg size, we expect around 160 chunks.
        // Even with low entropy (zeros), max_size 128KB ensures at least 80 chunks.
        assert!(manifest.chunks.len() >= 80, "Should have at least 80 chunks for 10MB to provide granular progress");

        Ok(())
    }
}
