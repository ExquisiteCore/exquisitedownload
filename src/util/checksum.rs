use std::path::Path;

use anyhow::{Context, Result};
use digest::Digest;
use tokio::io::AsyncReadExt;

/// Parse a checksum string like "sha256=abc123..." into (algorithm, expected_hex)
pub fn parse_checksum(s: &str) -> Result<(&str, &str)> {
    let (algo, hash) = s.split_once('=').ok_or_else(|| {
        anyhow::anyhow!("invalid checksum format, expected algo=hex (e.g., sha256=abc123)")
    })?;
    let algo = algo.trim();
    let hash = hash.trim();
    match algo {
        "sha256" | "sha1" | "md5" => Ok((algo, hash)),
        _ => anyhow::bail!(
            "unsupported checksum algorithm: {} (supported: sha256, sha1, md5)",
            algo
        ),
    }
}

/// Verify a file's checksum against an expected hex digest
pub async fn verify_file(path: &Path, algo: &str, expected: &str) -> Result<()> {
    let hex = compute_hash(path, algo).await?;
    if hex.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        anyhow::bail!(
            "checksum mismatch ({}): expected {}, got {}",
            algo,
            expected,
            hex
        )
    }
}

async fn compute_hash(path: &Path, algo: &str) -> Result<String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("failed to open file for checksum: {}", path.display()))?;

    let mut buf = vec![0u8; 1024 * 1024];

    match algo {
        "sha256" => {
            let mut hasher = sha2::Sha256::new();
            loop {
                let n = file.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        }
        "sha1" => {
            let mut hasher = sha1::Sha1::new();
            loop {
                let n = file.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        }
        "md5" => {
            let mut hasher = md5::Md5::new();
            loop {
                let n = file.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(format!("{:x}", hasher.finalize()))
        }
        _ => anyhow::bail!("unsupported algorithm: {}", algo),
    }
}
