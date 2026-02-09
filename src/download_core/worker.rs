use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::Client;
use tokio::io::AsyncWriteExt;

use super::segment::{Segment, SegmentStatus};

/// Progress callback: (segment_index, bytes_just_downloaded)
pub type ProgressCallback = Arc<dyn Fn(u32, u64) + Send + Sync>;

/// Download a single segment to a temp file
pub async fn download_segment(
    client: &Client,
    url: &str,
    segment: &mut Segment,
    download_dir: &std::path::Path,
    task_id: &str,
    cancel_flag: &AtomicBool,
    speed_limiter: Option<&Arc<AtomicU64>>,
    on_progress: &ProgressCallback,
) -> Result<()> {
    let temp_path = download_dir.join(segment.temp_filename(task_id));

    // When resuming, trust the actual file size on disk over state counter,
    // because the process may have been killed between a file write and a state save.
    if temp_path.exists() {
        let file_len = tokio::fs::metadata(&temp_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        if file_len != segment.downloaded {
            tracing::debug!(
                "segment {}: adjusting downloaded {} -> {} (actual file size)",
                segment.index,
                segment.downloaded,
                file_len
            );
            segment.downloaded = file_len;
        }
    }

    // Already complete?
    if segment.is_complete() {
        segment.status = SegmentStatus::Completed;
        return Ok(());
    }

    let resume_from = segment.resume_offset();
    let end_byte = segment.end - 1; // Range header is inclusive

    let mut request = client.get(url);

    // Set Range header
    if segment.start > 0 || segment.end < u64::MAX {
        request = request.header(
            reqwest::header::RANGE,
            format!("bytes={}-{}", resume_from, end_byte),
        );
    }

    let response = request.send().await.context("segment request failed")?;
    let status = response.status();
    if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
        anyhow::bail!(
            "segment {} download failed with status {}",
            segment.index,
            status
        );
    }

    segment.status = SegmentStatus::Downloading;

    // Open file for append if resuming, otherwise create new
    let mut file = if segment.downloaded > 0 {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(&temp_path)
            .await
            .context("failed to open temp file for resume")?
    } else {
        tokio::fs::File::create(&temp_path)
            .await
            .context("failed to create temp file")?
    };

    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        if cancel_flag.load(Ordering::Relaxed) {
            return Ok(());
        }

        let chunk = chunk.context("error reading chunk")?;
        let chunk_len = chunk.len() as u64;

        // Speed limiting: simple delay-based throttling
        if let Some(limiter) = speed_limiter {
            let limit = limiter.load(Ordering::Relaxed);
            if limit > 0 {
                let delay_ms = (chunk_len * 1000) / limit;
                if delay_ms > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                }
            }
        }

        file.write_all(&chunk).await.context("failed to write chunk")?;
        segment.downloaded += chunk_len;
        on_progress(segment.index, chunk_len);
    }

    file.flush().await?;
    segment.status = SegmentStatus::Completed;

    Ok(())
}
