use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use indicatif::MultiProgress;
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::config::Config;
use crate::net::http;
use crate::storage::state;
use crate::util::{progress, speed};

use super::merge::merge_segments;
use super::segment::{create_segments, create_single_segment, SegmentStatus};
use super::task::{DownloadTask, TaskStatus};
use super::worker::{self, ProgressCallback};

/// The download engine manages all download tasks
pub struct DownloadEngine {
    config: Config,
    client: reqwest::Client,
    tasks: Arc<RwLock<HashMap<String, DownloadTask>>>,
    cancel_flags: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
    speed_limit: Arc<AtomicU64>,
    multi_progress: MultiProgress,
}

impl DownloadEngine {
    pub fn new(config: Config) -> Result<Self> {
        let client = http::build_client(&config.user_agent, config.timeout_secs)?;
        let speed_limit = speed::create_speed_limit_value(config.max_speed);

        Ok(Self {
            config,
            client,
            tasks: Arc::new(RwLock::new(HashMap::new())),
            cancel_flags: Arc::new(RwLock::new(HashMap::new())),
            speed_limit,
            multi_progress: MultiProgress::new(),
        })
    }

    /// Add and immediately start a download task
    pub async fn download(
        &self,
        url: String,
        output: Option<String>,
        dir: Option<PathBuf>,
        split: u8,
        max_connections: u8,
        speed_limit_override: Option<u64>,
    ) -> Result<String> {
        let download_dir = dir.unwrap_or_else(|| self.config.download_dir.clone());
        tokio::fs::create_dir_all(&download_dir).await?;

        // Fetch metadata
        info!("Fetching file info from {}", url);
        let metadata = http::fetch_metadata(&self.client, &url).await?;

        let filename = output
            .or(metadata.filename)
            .unwrap_or_else(|| "download".to_string());

        let file_path = download_dir.join(&filename);

        info!(
            "File: {} | Size: {} | Range: {}",
            filename,
            metadata
                .content_length
                .map(speed::format_bytes)
                .unwrap_or_else(|| "unknown".into()),
            if metadata.supports_range { "yes" } else { "no" }
        );

        // Create or resume task
        let mut task = DownloadTask::new(url.clone(), file_path.clone(), max_connections);
        task.total_size = metadata.content_length;
        task.supports_range = metadata.supports_range;

        // Create segments
        if let Some(total_size) = metadata.content_length {
            if metadata.supports_range && split > 1 {
                task.segments = create_segments(total_size, split);
                info!("Split into {} segments", split);
            } else {
                task.segments = create_single_segment(total_size);
                if !metadata.supports_range {
                    info!("Server doesn't support Range, using single connection");
                }
            }
        } else {
            // Unknown size, single segment with u64::MAX end
            task.segments = vec![super::segment::Segment::new(0, 0, u64::MAX)];
        }

        task.status = TaskStatus::Downloading;
        let task_id = task.id.clone();

        // Set up cancellation
        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.cancel_flags
            .write()
            .await
            .insert(task_id.clone(), cancel_flag.clone());

        // Store task
        self.tasks.write().await.insert(task_id.clone(), task.clone());

        // Set speed limit
        if let Some(limit) = speed_limit_override {
            self.speed_limit.store(limit, Ordering::Relaxed);
        }

        // Save initial state
        state::save_state(&task, &download_dir).await?;

        // Run the download
        let result = self
            .run_download(task, &download_dir, cancel_flag)
            .await;

        // Clean up
        self.cancel_flags.write().await.remove(&task_id);

        match &result {
            Ok(()) => {
                state::remove_state(&task_id, &download_dir).await?;
                let mut tasks = self.tasks.write().await;
                if let Some(t) = tasks.get_mut(&task_id) {
                    t.status = TaskStatus::Completed;
                }
            }
            Err(e) => {
                error!("Download failed: {}", e);
                let mut tasks = self.tasks.write().await;
                if let Some(t) = tasks.get_mut(&task_id) {
                    t.status = TaskStatus::Error;
                    t.error_message = Some(e.to_string());
                    // Save error state for potential resume
                    let _ = state::save_state(t, &download_dir).await;
                }
            }
        }

        result?;
        Ok(task_id)
    }

    async fn run_download(
        &self,
        mut task: DownloadTask,
        download_dir: &std::path::Path,
        cancel_flag: Arc<AtomicBool>,
    ) -> Result<()> {
        let start_time = Instant::now();
        let total_size = task.total_size.unwrap_or(0);
        let filename = task
            .file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download")
            .to_string();

        // Create progress bar
        let pb = if total_size > 0 {
            progress::create_download_progress(&self.multi_progress, total_size, &filename)
        } else {
            progress::create_unknown_size_progress(&self.multi_progress, &filename)
        };

        // Shared downloaded counter for progress updates
        let downloaded = Arc::new(AtomicU64::new(task.downloaded_bytes()));
        pb.set_position(task.downloaded_bytes());

        // Progress callback
        let downloaded_clone = downloaded.clone();
        let pb_clone = pb.clone();
        let on_progress: ProgressCallback = Arc::new(move |_seg_idx, bytes| {
            downloaded_clone.fetch_add(bytes, Ordering::Relaxed);
            pb_clone.set_position(downloaded_clone.load(Ordering::Relaxed));
        });

        // Launch segment workers
        let mut handles = Vec::new();
        let segments_len = task.segments.len();

        for i in 0..segments_len {
            if task.segments[i].is_complete() {
                continue;
            }

            let client = self.client.clone();
            let url = task.url.clone();
            let dir = download_dir.to_path_buf();
            let cancel = cancel_flag.clone();
            let speed_limit = self.speed_limit.clone();
            let progress_cb = on_progress.clone();
            let mut segment = task.segments[i].clone();

            let handle = tokio::spawn(async move {
                let result = worker::download_segment(
                    &client,
                    &url,
                    &mut segment,
                    &dir,
                    &cancel,
                    Some(&speed_limit),
                    &progress_cb,
                )
                .await;
                (i, segment, result)
            });

            handles.push(handle);
        }

        // Wait for all segments
        let mut errors = Vec::new();
        for handle in handles {
            match handle.await {
                Ok((idx, segment, result)) => {
                    task.segments[idx] = segment;
                    if let Err(e) = result {
                        errors.push(format!("segment {}: {}", idx, e));
                    }
                }
                Err(e) => {
                    errors.push(format!("task join error: {}", e));
                }
            }
        }

        pb.finish_and_clear();

        if !errors.is_empty() {
            // Save state for resume
            let _ = state::save_state(&task, download_dir).await;
            anyhow::bail!("download errors:\n{}", errors.join("\n"));
        }

        // Merge segments
        if segments_len > 1 {
            info!("Merging {} segments...", segments_len);
            merge_segments(&task.segments, "dl", download_dir, &task.file_path)
                .await
                .context("failed to merge segments")?;
        } else {
            // Single segment: just rename the temp file
            let temp_path = download_dir.join(task.segments[0].temp_filename("dl"));
            tokio::fs::rename(&temp_path, &task.file_path)
                .await
                .context("failed to rename temp file")?;
        }

        let elapsed = start_time.elapsed().as_secs_f64();
        let final_size = task.downloaded_bytes();
        progress::print_summary(&filename, final_size, elapsed);

        Ok(())
    }

    /// Pause a task
    pub async fn pause_task(&self, task_id: &str) -> Result<()> {
        if let Some(flag) = self.cancel_flags.read().await.get(task_id) {
            flag.store(true, Ordering::Relaxed);
        }
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = TaskStatus::Paused;
        }
        Ok(())
    }

    /// Get task status
    pub async fn get_task(&self, task_id: &str) -> Option<DownloadTask> {
        self.tasks.read().await.get(task_id).cloned()
    }

    /// List all tasks
    pub async fn list_tasks(&self) -> Vec<DownloadTask> {
        self.tasks.read().await.values().cloned().collect()
    }

    /// Get global stats
    pub async fn global_stats(&self) -> GlobalStats {
        let tasks = self.tasks.read().await;
        let active = tasks
            .values()
            .filter(|t| t.status == TaskStatus::Downloading)
            .count();
        let waiting = tasks
            .values()
            .filter(|t| t.status == TaskStatus::Pending)
            .count();
        let stopped = tasks
            .values()
            .filter(|t| matches!(t.status, TaskStatus::Completed | TaskStatus::Error | TaskStatus::Paused))
            .count();

        GlobalStats {
            active,
            waiting,
            stopped,
            speed_limit: self.speed_limit.load(Ordering::Relaxed),
        }
    }

    /// Set global speed limit
    pub fn set_speed_limit(&self, limit: u64) {
        self.speed_limit.store(limit, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GlobalStats {
    pub active: usize,
    pub waiting: usize,
    pub stopped: usize,
    pub speed_limit: u64,
}
