use std::path::Path;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::segment::Segment;

/// Merge all segment temp files into the final output file.
/// Verifies the total size after merge if `expected_size` is provided.
pub async fn merge_segments(
    segments: &[Segment],
    task_id: &str,
    download_dir: &Path,
    output_path: &Path,
    expected_size: Option<u64>,
) -> Result<()> {
    let mut output = tokio::fs::File::create(output_path)
        .await
        .context("failed to create output file")?;

    let mut buf = vec![0u8; 1024 * 1024]; // 1MB buffer
    let mut total_written = 0u64;

    for segment in segments {
        let temp_path = download_dir.join(segment.temp_filename(task_id));
        let mut temp_file = tokio::fs::File::open(&temp_path)
            .await
            .with_context(|| format!("failed to open segment file: {}", temp_path.display()))?;

        loop {
            let n = temp_file.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            output.write_all(&buf[..n]).await?;
            total_written += n as u64;
        }
    }

    output.flush().await?;

    // Verify total size
    if let Some(expected) = expected_size
        && total_written != expected
    {
        anyhow::bail!(
            "size mismatch after merge: expected {} bytes, got {}",
            expected,
            total_written
        );
    }

    // Clean up temp files
    for segment in segments {
        let temp_path = download_dir.join(segment.temp_filename(task_id));
        let _ = tokio::fs::remove_file(&temp_path).await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download_core::segment::Segment;

    #[tokio::test]
    async fn test_merge_segments() {
        let dir = std::env::temp_dir().join("edl_test_merge");
        let _ = tokio::fs::create_dir_all(&dir).await;
        let task_id = "test-merge-id";

        // Create fake segment files
        let mut s1 = Segment::new(0, 0, 5);
        s1.downloaded = 5;
        let mut s2 = Segment::new(1, 5, 10);
        s2.downloaded = 5;

        tokio::fs::write(dir.join(s1.temp_filename(task_id)), b"hello")
            .await
            .unwrap();
        tokio::fs::write(dir.join(s2.temp_filename(task_id)), b"world")
            .await
            .unwrap();

        let output = dir.join("merged.txt");
        merge_segments(&[s1, s2], task_id, &dir, &output, Some(10))
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(&output).await.unwrap();
        assert_eq!(content, "helloworld");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_merge_size_mismatch() {
        let dir = std::env::temp_dir().join("edl_test_merge_mismatch");
        let _ = tokio::fs::create_dir_all(&dir).await;
        let task_id = "test-merge-mm";

        let mut s1 = Segment::new(0, 0, 5);
        s1.downloaded = 5;
        tokio::fs::write(dir.join(s1.temp_filename(task_id)), b"hello")
            .await
            .unwrap();

        let output = dir.join("merged_mm.txt");
        // Expect 100 bytes but only 5 written
        let result = merge_segments(&[s1], task_id, &dir, &output, Some(100)).await;
        assert!(result.is_err());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_merge_no_expected_size() {
        let dir = std::env::temp_dir().join("edl_test_merge_nosize");
        let _ = tokio::fs::create_dir_all(&dir).await;
        let task_id = "test-merge-ns";

        let mut s1 = Segment::new(0, 0, 3);
        s1.downloaded = 3;
        tokio::fs::write(dir.join(s1.temp_filename(task_id)), b"abc")
            .await
            .unwrap();

        let output = dir.join("merged_ns.txt");
        merge_segments(&[s1], task_id, &dir, &output, None)
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(&output).await.unwrap();
        assert_eq!(content, "abc");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
