use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use anyhow::{Context, Result};
use indicatif::MultiProgress;
use tokio::sync::{RwLock, Semaphore, broadcast};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::net::http;
use crate::storage::state;
use crate::util::progress;
use crate::util::speed::{self, SpeedLimiter};

use super::merge::merge_segments;
use super::segment::{create_segments, create_single_segment};
use super::task::{DownloadTask, TaskStatus};
use super::worker::{self, ProgressCallback};

/// Options for creating a new download task.
/// Used by `prepare_download`, `download`, and `download_background`.
pub struct DownloadOptions {
    pub url: String,
    pub output: Option<String>,
    pub dir: Option<PathBuf>,
    pub split: u8,
    pub max_connections: u8,
    pub extra_headers: Vec<String>,
    pub extra_cookie: Option<String>,
}

/// Lifecycle events emitted by the download engine.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum EngineEvent {
    TaskStarted {
        task_id: String,
        url: String,
        file_path: String,
    },
    TaskProgress {
        task_id: String,
        downloaded: u64,
        total: Option<u64>,
    },
    TaskPaused {
        task_id: String,
    },
    TaskCompleted {
        task_id: String,
        file_path: String,
        size: u64,
    },
    TaskError {
        task_id: String,
        message: String,
    },
}

/// The download engine manages all download tasks
pub struct DownloadEngine {
    config: Config,
    client: reqwest::Client,
    tasks: Arc<RwLock<HashMap<String, DownloadTask>>>,
    cancel_flags: Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>,
    speed_limiter: Arc<SpeedLimiter>,
    multi_progress: MultiProgress,
    /// Semaphore limiting concurrent downloads
    concurrency_sem: Arc<Semaphore>,
    /// Broadcast channel for engine lifecycle events
    event_tx: broadcast::Sender<EngineEvent>,
}

impl DownloadEngine {
    pub fn new(config: Config) -> Result<Self> {
        let client = http::build_client(
            &config.user_agent,
            config.timeout_secs,
            config.max_redirects as usize,
            None,
            None,
        )?;
        let speed_limiter = Arc::new(SpeedLimiter::new(config.max_speed.unwrap_or(0)));
        let concurrency_sem = Arc::new(Semaphore::new(config.max_concurrent_tasks));
        let (event_tx, _) = broadcast::channel(256);

        Ok(Self {
            config,
            client,
            tasks: Arc::new(RwLock::new(HashMap::new())),
            cancel_flags: Arc::new(RwLock::new(HashMap::new())),
            speed_limiter,
            multi_progress: MultiProgress::new(),
            concurrency_sem,
            event_tx,
        })
    }

    /// Create engine with a custom proxy and headers
    pub fn with_options(
        config: Config,
        proxy: Option<&str>,
        headers: Option<reqwest::header::HeaderMap>,
    ) -> Result<Self> {
        let client = http::build_client(
            &config.user_agent,
            config.timeout_secs,
            config.max_redirects as usize,
            proxy,
            headers,
        )?;
        let speed_limiter = Arc::new(SpeedLimiter::new(config.max_speed.unwrap_or(0)));
        let concurrency_sem = Arc::new(Semaphore::new(config.max_concurrent_tasks));
        let (event_tx, _) = broadcast::channel(256);

        Ok(Self {
            config,
            client,
            tasks: Arc::new(RwLock::new(HashMap::new())),
            cancel_flags: Arc::new(RwLock::new(HashMap::new())),
            speed_limiter,
            multi_progress: MultiProgress::new(),
            concurrency_sem,
            event_tx,
        })
    }

    /// Prepare a download task (create or resume) and return its id.
    /// Does NOT run the download — use `run_task` for that.
    pub async fn prepare_download(&self, opts: DownloadOptions) -> Result<DownloadTask> {
        let url = opts.url;
        let output = opts.output;
        let split = opts.split;
        let max_connections = opts.max_connections;
        let extra_headers = opts.extra_headers;
        let extra_cookie = opts.extra_cookie;
        let download_dir = opts.dir.unwrap_or_else(|| self.config.download_dir.clone());
        tokio::fs::create_dir_all(&download_dir).await?;

        // Build per-task extra headers for metadata requests
        let extra_hdr_map = if !extra_headers.is_empty() || extra_cookie.is_some() {
            Some(http::parse_headers(&extra_headers, extra_cookie.as_deref()).unwrap_or_default())
        } else {
            None
        };

        // Check for resumable state
        let existing = state::find_state_by_url(&url, &download_dir).await?;

        if let Some(mut prev) = existing {
            // ETag validation: re-fetch metadata and compare
            let metadata = http::fetch_metadata(&self.client, &url, extra_hdr_map.as_ref()).await?;
            let etag_mismatch = matches!(
                (&prev.etag, &metadata.etag),
                (Some(old), Some(new)) if old != new
            );

            if etag_mismatch {
                warn!(
                    "ETag changed ({} -> {}), discarding old state",
                    prev.etag.as_deref().unwrap_or("?"),
                    metadata.etag.as_deref().unwrap_or("?"),
                );
                state::remove_state(&prev.id, &download_dir).await?;
                for seg in &prev.segments {
                    let p = download_dir.join(seg.temp_filename(&prev.id));
                    let _ = tokio::fs::remove_file(&p).await;
                }
                // Fall through to create new task below
            } else {
                let resumed_bytes = prev.downloaded_bytes();
                let total = prev.total_size.unwrap_or(0);
                let completed_segs = prev.segments.iter().filter(|s| s.is_complete()).count();
                let total_segs = prev.segments.len();
                info!(
                    "Resuming task {} — {}/{} downloaded, {}/{} segments done",
                    prev.id,
                    speed::format_bytes(resumed_bytes),
                    speed::format_bytes(total),
                    completed_segs,
                    total_segs,
                );
                if let Some(final_url) = metadata.final_url {
                    prev.url = final_url;
                }
                prev.status = TaskStatus::Downloading;
                prev.error_message = None;
                return Ok(prev);
            }
        }

        // New download
        info!("Fetching file info from {}", url);
        let metadata = http::fetch_metadata(&self.client, &url, extra_hdr_map.as_ref()).await?;

        // Use the final URL after redirects for Range requests
        let effective_url = metadata.final_url.clone().unwrap_or(url);

        let filename = output
            .or(metadata.filename)
            .unwrap_or_else(|| "download".to_string());
        let file_path = resolve_file_conflict(&download_dir.join(&filename)).await;

        info!(
            "File: {} | Size: {} | Range: {}",
            file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&filename),
            metadata
                .content_length
                .map(speed::format_bytes)
                .unwrap_or_else(|| "unknown".into()),
            if metadata.supports_range { "yes" } else { "no" }
        );

        let mut task = DownloadTask::new(effective_url, file_path, download_dir, max_connections);
        task.total_size = metadata.content_length;
        task.supports_range = metadata.supports_range;
        task.etag = metadata.etag;
        task.extra_headers = extra_headers;
        task.extra_cookie = extra_cookie;

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
            task.segments = vec![super::segment::Segment::new(0, 0, u64::MAX)];
        }

        task.status = TaskStatus::Downloading;
        Ok(task)
    }

    /// Run a download task to completion (blocking).
    /// Used by CLI direct download.
    pub async fn download(
        &self,
        opts: DownloadOptions,
        speed_limit_override: Option<u64>,
    ) -> Result<String> {
        let task = self.prepare_download(opts).await?;
        let task_id = task.id.clone();

        if let Some(limit) = speed_limit_override {
            self.speed_limiter.set_limit(limit);
        }

        self.execute_task(task).await?;
        Ok(task_id)
    }

    /// Spawn a download task in background (non-blocking).
    /// Used by RPC `addUri`.
    pub async fn download_background(
        self: &Arc<Self>,
        opts: DownloadOptions,
    ) -> Result<String> {
        let task = self.prepare_download(opts).await?;
        let task_id = task.id.clone();

        let engine = self.clone();
        tokio::spawn(async move {
            if let Err(e) = engine.execute_task(task).await {
                error!("Background download failed: {}", e);
            }
        });

        Ok(task_id)
    }

    /// Execute a task: register, acquire semaphore, download, merge, clean up.
    async fn execute_task(&self, task: DownloadTask) -> Result<()> {
        let task_id = task.id.clone();
        let download_dir = task.download_dir.clone();

        // Set up cancellation
        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.cancel_flags
            .write()
            .await
            .insert(task_id.clone(), cancel_flag.clone());

        // Store task
        self.tasks
            .write()
            .await
            .insert(task_id.clone(), task.clone());

        // Save initial state
        state::save_state(&task, &download_dir).await?;

        // Acquire concurrency semaphore — waits if too many tasks are running
        let _permit = self
            .concurrency_sem
            .acquire()
            .await
            .map_err(|_| anyhow::anyhow!("concurrency semaphore closed"))?;

        // Check if cancelled while waiting in queue
        if cancel_flag.load(Ordering::Relaxed) {
            self.cancel_flags.write().await.remove(&task_id);
            return Ok(());
        }

        // Emit TaskStarted event
        let file_path_str = task.file_path.to_str().unwrap_or_default().to_string();
        let _ = self.event_tx.send(EngineEvent::TaskStarted {
            task_id: task_id.clone(),
            url: task.url.clone(),
            file_path: file_path_str.clone(),
        });

        // Run the download
        let result = self.run_download(task, &download_dir, cancel_flag).await;

        // Clean up
        self.cancel_flags.write().await.remove(&task_id);

        match &result {
            Ok(()) => {
                state::remove_state(&task_id, &download_dir).await?;
                let size = {
                    let mut tasks = self.tasks.write().await;
                    if let Some(t) = tasks.get_mut(&task_id) {
                        t.status = TaskStatus::Completed;
                    }
                    tasks
                        .get(&task_id)
                        .map(|t| t.downloaded_bytes())
                        .unwrap_or(0)
                };
                let _ = self.event_tx.send(EngineEvent::TaskCompleted {
                    task_id: task_id.clone(),
                    file_path: file_path_str,
                    size,
                });
            }
            Err(e) => {
                error!("Download failed: {}", e);
                let mut tasks = self.tasks.write().await;
                if let Some(t) = tasks.get_mut(&task_id) {
                    t.status = TaskStatus::Error;
                    t.error_message = Some(e.to_string());
                    let _ = state::save_state(t, &download_dir).await;
                }
                let _ = self.event_tx.send(EngineEvent::TaskError {
                    task_id: task_id.clone(),
                    message: e.to_string(),
                });
            }
        }

        result
    }

    async fn run_download(
        &self,
        mut task: DownloadTask,
        download_dir: &std::path::Path,
        cancel_flag: Arc<AtomicBool>,
    ) -> Result<()> {
        let start_time = Instant::now();
        let total_size = task.total_size.unwrap_or(0);
        let task_id = task.id.clone();
        let retry_count = self.config.retry_count;
        let filename = task
            .file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download")
            .to_string();

        // Build per-task extra headers (from extension cookies, referer, etc.)
        let extra_header_map = if !task.extra_headers.is_empty() || task.extra_cookie.is_some() {
            Some(Arc::new(
                http::parse_headers(&task.extra_headers, task.extra_cookie.as_deref())
                    .unwrap_or_default(),
            ))
        } else {
            None
        };

        // Progress bar
        let pb = if total_size > 0 {
            progress::create_download_progress(&self.multi_progress, total_size, &filename)
        } else {
            progress::create_unknown_size_progress(&self.multi_progress, &filename)
        };

        let downloaded = Arc::new(AtomicU64::new(task.downloaded_bytes()));
        pb.set_position(task.downloaded_bytes());

        // Per-segment counters for real-time state saving + RPC progress
        let seg_counters: Vec<Arc<AtomicU64>> = task
            .segments
            .iter()
            .map(|s| Arc::new(AtomicU64::new(s.downloaded)))
            .collect();
        let seg_counters = Arc::new(seg_counters);

        // Progress callback
        let downloaded_clone = downloaded.clone();
        let pb_clone = pb.clone();
        let counters_clone = seg_counters.clone();
        let on_progress: ProgressCallback = Arc::new(move |seg_idx, bytes| {
            downloaded_clone.fetch_add(bytes, Ordering::Relaxed);
            pb_clone.set_position(downloaded_clone.load(Ordering::Relaxed));
            counters_clone[seg_idx as usize].fetch_add(bytes, Ordering::Relaxed);
        });

        // Launch segment workers
        let mut handles = Vec::new();
        let segments_len = task.segments.len();
        let worker_sem = Arc::new(Semaphore::new(task.max_connections as usize));

        for i in 0..segments_len {
            if task.segments[i].is_complete() {
                continue;
            }

            let client = self.client.clone();
            let url = task.url.clone();
            let dir = download_dir.to_path_buf();
            let tid = task_id.clone();
            let cancel = cancel_flag.clone();
            let limiter = self.speed_limiter.clone();
            let progress_cb = on_progress.clone();
            let mut segment = task.segments[i].clone();
            let sem = worker_sem.clone();
            let hdrs = extra_header_map.clone();

            let handle = tokio::spawn(async move {
                // Wait for a connection slot
                let _permit = sem.acquire().await.unwrap();
                let result = worker::download_segment(
                    &client,
                    &url,
                    &mut segment,
                    &dir,
                    &tid,
                    &cancel,
                    Some(&limiter),
                    &progress_cb,
                    retry_count,
                    hdrs.as_deref(),
                )
                .await;
                (i, segment, result)
            });

            handles.push(handle);
        }

        // Periodic state saving + RPC progress update (every 3 seconds)
        let save_snapshot = task.clone();
        let save_dir = download_dir.to_path_buf();
        let save_counters = seg_counters.clone();
        let save_cancel = cancel_flag.clone();
        let save_tasks = self.tasks.clone();
        let event_tx = self.event_tx.clone();
        let save_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
            loop {
                interval.tick().await;
                if save_cancel.load(Ordering::Relaxed) {
                    break;
                }
                let mut snapshot = save_snapshot.clone();
                for (seg, counter) in snapshot.segments.iter_mut().zip(save_counters.iter()) {
                    seg.downloaded = counter.load(Ordering::Relaxed);
                }
                let dl_bytes = snapshot.downloaded_bytes();
                if let Some(t) = save_tasks.write().await.get_mut(&snapshot.id) {
                    t.segments = snapshot.segments.clone();
                }
                let _ = state::save_state(&snapshot, &save_dir).await;
                let _ = event_tx.send(EngineEvent::TaskProgress {
                    task_id: snapshot.id.clone(),
                    downloaded: dl_bytes,
                    total: snapshot.total_size,
                });
            }
        });

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

        save_handle.abort();
        pb.finish_and_clear();

        // Final update of segment progress to the shared task map
        {
            let mut tasks = self.tasks.write().await;
            if let Some(t) = tasks.get_mut(&task_id) {
                t.segments = task.segments.clone();
            }
        }

        if !errors.is_empty() {
            let _ = state::save_state(&task, download_dir).await;
            anyhow::bail!("download errors:\n{}", errors.join("\n"));
        }

        // Merge segments
        if segments_len > 1 {
            info!("Merging {} segments...", segments_len);
            merge_segments(
                &task.segments,
                &task_id,
                download_dir,
                &task.file_path,
                task.total_size,
            )
            .await
            .context("failed to merge segments")?;
        } else {
            let temp_path = download_dir.join(task.segments[0].temp_filename(&task_id));
            tokio::fs::rename(&temp_path, &task.file_path)
                .await
                .context("failed to rename temp file")?;

            if let Some(expected) = task.total_size {
                let actual = tokio::fs::metadata(&task.file_path).await?.len();
                if actual != expected {
                    anyhow::bail!("size mismatch: expected {} bytes, got {}", expected, actual);
                }
            }
        }

        let elapsed = start_time.elapsed().as_secs_f64();
        let final_size = task.downloaded_bytes();
        progress::print_summary(&filename, final_size, elapsed);

        Ok(())
    }

    /// Pause a task by setting its cancel flag
    pub async fn pause_task(&self, task_id: &str) -> Result<()> {
        if let Some(flag) = self.cancel_flags.read().await.get(task_id) {
            flag.store(true, Ordering::Relaxed);
        }
        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = TaskStatus::Paused;
            let _ = state::save_state(task, &task.download_dir.clone()).await;
        }
        let _ = self.event_tx.send(EngineEvent::TaskPaused {
            task_id: task_id.to_string(),
        });
        Ok(())
    }

    /// Resume a paused task by re-running it in background
    pub async fn resume_task(self: &Arc<Self>, task_id: &str) -> Result<()> {
        let task = {
            let tasks = self.tasks.read().await;
            tasks.get(task_id).cloned()
        };

        let Some(task) = task else {
            anyhow::bail!("task not found: {}", task_id);
        };

        if task.status != TaskStatus::Paused && task.status != TaskStatus::Error {
            anyhow::bail!("task {} is not paused or errored", task_id);
        }

        let url = task.url.clone();
        let dir = task.download_dir.clone();
        let split = task.segments.len() as u8;
        let max_conn = task.max_connections;
        let headers = task.extra_headers.clone();
        let cookie = task.extra_cookie.clone();
        self.download_background(DownloadOptions {
            url,
            output: None,
            dir: Some(dir),
            split,
            max_connections: max_conn,
            extra_headers: headers,
            extra_cookie: cookie,
        })
        .await?;
        Ok(())
    }

    /// Remove a task and clean up its files
    pub async fn remove_task(&self, task_id: &str) -> Result<()> {
        if let Some(flag) = self.cancel_flags.read().await.get(task_id) {
            flag.store(true, Ordering::Relaxed);
        }

        let task = self.tasks.write().await.remove(task_id);
        if let Some(task) = task {
            let _ = state::remove_state(task_id, &task.download_dir).await;
            for seg in &task.segments {
                let p = task.download_dir.join(seg.temp_filename(task_id));
                let _ = tokio::fs::remove_file(&p).await;
            }
        }
        Ok(())
    }

    pub async fn get_task(&self, task_id: &str) -> Option<DownloadTask> {
        self.tasks.read().await.get(task_id).cloned()
    }

    pub async fn list_tasks(&self) -> Vec<DownloadTask> {
        self.tasks.read().await.values().cloned().collect()
    }

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
            .filter(|t| {
                matches!(
                    t.status,
                    TaskStatus::Completed | TaskStatus::Error | TaskStatus::Paused
                )
            })
            .count();

        GlobalStats {
            active,
            waiting,
            stopped,
            speed_limit: self.speed_limiter.limit(),
        }
    }

    pub fn set_speed_limit(&self, limit: u64) {
        self.speed_limiter.set_limit(limit);
    }

    pub fn default_connections(&self) -> u8 {
        self.config.max_connections_per_task
    }

    /// Subscribe to engine lifecycle events (for WebSocket, hooks, etc.)
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.event_tx.subscribe()
    }

    /// Spawn a background task that listens for events and runs config hooks
    pub fn spawn_hook_runner(&self) {
        use crate::config::run_hook;

        let mut rx = self.event_tx.subscribe();
        let config = self.config.clone();

        tokio::spawn(async move {
            loop {
                let event = match rx.recv().await {
                    Ok(e) => e,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!("hook runner skipped {} events", n);
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };

                match event {
                    EngineEvent::TaskStarted {
                        task_id,
                        url,
                        file_path,
                    } => {
                        if let Some(ref tpl) = config.on_start {
                            run_hook(
                                tpl,
                                &[("file", &file_path), ("id", &task_id), ("url", &url)],
                            );
                        }
                    }
                    EngineEvent::TaskCompleted {
                        task_id,
                        file_path,
                        size,
                    } => {
                        if let Some(ref tpl) = config.on_complete {
                            run_hook(
                                tpl,
                                &[
                                    ("file", &file_path),
                                    ("size", &size.to_string()),
                                    ("id", &task_id),
                                ],
                            );
                        }
                    }
                    EngineEvent::TaskPaused { task_id } => {
                        if let Some(ref tpl) = config.on_pause {
                            run_hook(tpl, &[("id", &task_id)]);
                        }
                    }
                    EngineEvent::TaskError { task_id, message } => {
                        if let Some(ref tpl) = config.on_error {
                            run_hook(tpl, &[("id", &task_id), ("error", &message)]);
                        }
                    }
                    EngineEvent::TaskProgress { .. } => {} // no hook for progress
                }
            }
        });
    }

    /// Load persisted task states from disk (for RPC server restart recovery)
    pub async fn load_persisted_tasks(&self) -> Result<()> {
        let tasks = state::find_all_states(&self.config.download_dir).await?;
        if tasks.is_empty() {
            return Ok(());
        }
        let mut task_map = self.tasks.write().await;
        for task in tasks {
            info!(
                "Restored task {} ({:?}) — {}",
                task.id,
                task.status,
                task.file_path.display(),
            );
            task_map.insert(task.id.clone(), task);
        }
        Ok(())
    }
}

/// Resolve file name conflicts by appending (1), (2), etc.
async fn resolve_file_conflict(path: &std::path::Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let ext = path.extension().and_then(|s| s.to_str());
    let parent = path.parent().unwrap_or(std::path::Path::new("."));

    for i in 1..1000 {
        let new_name = match ext {
            Some(e) => format!("{}({}).{}", stem, i, e),
            None => format!("{}({})", stem, i),
        };
        let new_path = parent.join(new_name);
        if !new_path.exists() {
            return new_path;
        }
    }
    path.to_path_buf()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GlobalStats {
    pub active: usize,
    pub waiting: usize,
    pub stopped: usize,
    pub speed_limit: u64,
}
