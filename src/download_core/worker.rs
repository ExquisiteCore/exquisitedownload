use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::Client;
use tokio::io::AsyncWriteExt;

use crate::util::speed::SpeedLimiter;

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
    speed_limiter: Option<&Arc<SpeedLimiter>>,
    on_progress: &ProgressCallback,
    retry_count: u32,
) -> Result<()> {
    let mut last_err = None;
    for attempt in 0..=retry_count {
        if attempt > 0 {
            tracing::warn!("segment {}: retry {}/{}", segment.index, attempt, retry_count);
            tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt.min(4)))).await;
        }
        match download_segment_once(client, url, segment, download_dir, task_id, cancel_flag, speed_limiter, on_progress).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                if cancel_flag.load(Ordering::Relaxed) {
                    return Ok(());
                }
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap())
}

async fn download_segment_once(
    client: &Client,
    url: &str,
    segment: &mut Segment,
    download_dir: &std::path::Path,
    task_id: &str,
    cancel_flag: &AtomicBool,
    speed_limiter: Option<&Arc<SpeedLimiter>>,
    on_progress: &ProgressCallback,
) -> Result<()> {
    let temp_path = download_dir.join(segment.temp_filename(task_id));

    // When resuming, trust the actual file size on disk over state counter
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

    if segment.is_complete() {
        segment.status = SegmentStatus::Completed;
        return Ok(());
    }

    let resume_from = segment.resume_offset();
    let end_byte = segment.end - 1;

    let mut request = client.get(url);

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

        // Shared token-bucket speed limiting
        if let Some(limiter) = speed_limiter {
            if let Some(delay) = limiter.consume(chunk_len) {
                tokio::time::sleep(delay).await;
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
