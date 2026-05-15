use anyhow::{anyhow, Result};
use blake3;
use cdus_common::{FileChunk, FileManifest};
use fastcdc::v2020::StreamCDC;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Generates a manifest for a file using Content-Defined Chunking (FastCDC).
pub fn generate_manifest(path: &Path) -> Result<FileManifest> {
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
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    let file_hash = hasher.finalize().to_string();

    // Pass 2: Chunking and chunk hashes
    // We use StreamCDC to avoid loading the entire file into memory.
    // We open a separate file handle for the chunker to keep the read positions independent,
    // though we could also seek.
    let chunker_file = File::open(path)?;

    // Constraints (Avg 1MB chunks)
    let min_size = 128 * 1024; // 128 KB
    let avg_size = 1024 * 1024; // 1 MB
    let max_size = 4 * 1024 * 1024; // 4 MB

    let chunker = StreamCDC::new(chunker_file, min_size, avg_size, max_size);

    let mut data_file = File::open(path)?;
    let mut chunks = Vec::new();

    for chunk_res in chunker {
        let chunk = chunk_res?;
        // Since we are reading sequentially, data_file should already be at chunk.offset
        // but we verify or seek to be safe.
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
    }

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
}
