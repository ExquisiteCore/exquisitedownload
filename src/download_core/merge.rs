use std::path::Path;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::segment::Segment;

/// Merge all segment temp files into the final output file
pub async fn merge_segments(
    segments: &[Segment],
    task_id: &str,
    download_dir: &Path,
    output_path: &Path,
) -> Result<()> {
    let mut output = tokio::fs::File::create(output_path)
        .await
        .context("failed to create output file")?;

    let mut buf = vec![0u8; 64 * 1024]; // 64KB buffer

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
        }
    }

    output.flush().await?;

    // Clean up temp files
    for segment in segments {
        let temp_path = download_dir.join(segment.temp_filename(task_id));
        let _ = tokio::fs::remove_file(&temp_path).await;
    }

    Ok(())
}
