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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_checksum_sha256() {
        let (algo, hash) = parse_checksum("sha256=abc123").unwrap();
        assert_eq!(algo, "sha256");
        assert_eq!(hash, "abc123");
    }

    #[test]
    fn test_parse_checksum_md5() {
        let (algo, hash) = parse_checksum("md5=deadbeef").unwrap();
        assert_eq!(algo, "md5");
        assert_eq!(hash, "deadbeef");
    }

    #[test]
    fn test_parse_checksum_sha1() {
        let (algo, hash) = parse_checksum("sha1=0123456789abcdef").unwrap();
        assert_eq!(algo, "sha1");
        assert_eq!(hash, "0123456789abcdef");
    }

    #[test]
    fn test_parse_checksum_invalid_format() {
        assert!(parse_checksum("noequalssign").is_err());
    }

    #[test]
    fn test_parse_checksum_unsupported_algo() {
        assert!(parse_checksum("crc32=12345678").is_err());
    }

    #[test]
    fn test_parse_checksum_whitespace() {
        let (algo, hash) = parse_checksum(" sha256 = abc123 ").unwrap();
        assert_eq!(algo, "sha256");
        assert_eq!(hash, "abc123");
    }

    #[tokio::test]
    async fn test_verify_file_sha256() {
        let dir = std::env::temp_dir().join("edl_test_checksum");
        let _ = tokio::fs::create_dir_all(&dir).await;
        let path = dir.join("test_sha256.txt");
        tokio::fs::write(&path, b"hello world").await.unwrap();

        // sha256 of "hello world"
        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        verify_file(&path, "sha256", expected).await.unwrap();

        // Wrong hash should fail
        assert!(verify_file(&path, "sha256", "wrong").await.is_err());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_verify_file_md5() {
        let dir = std::env::temp_dir().join("edl_test_checksum_md5");
        let _ = tokio::fs::create_dir_all(&dir).await;
        let path = dir.join("test_md5.txt");
        tokio::fs::write(&path, b"hello world").await.unwrap();

        // md5 of "hello world"
        let expected = "5eb63bbbe01eeed093cb22bb8f5acdc3";
        verify_file(&path, "md5", expected).await.unwrap();

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
