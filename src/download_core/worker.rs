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
    cancel_flag: &AtomicBool,
    speed_limiter: Option<&Arc<AtomicU64>>,
    on_progress: &ProgressCallback,
) -> Result<()> {
    let temp_path = download_dir.join(segment.temp_filename("dl"));

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
        anyhow::bail!("segment {} download failed with status {}", segment.index, status);
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

        // Speed limiting: simple token bucket check
        if let Some(limiter) = speed_limiter {
            let limit = limiter.load(Ordering::Relaxed);
            if limit > 0 {
                // Simple delay-based throttling
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
