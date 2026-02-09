use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::segment::Segment;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Downloading,
    Paused,
    Completed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadTask {
    pub id: String,
    pub url: String,
    pub file_path: PathBuf,
    pub total_size: Option<u64>,
    pub supports_range: bool,
    pub etag: Option<String>,
    pub segments: Vec<Segment>,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub max_connections: u8,
    pub download_dir: PathBuf,
    pub error_message: Option<String>,
}

impl DownloadTask {
    pub fn new(
        url: String,
        file_path: PathBuf,
        download_dir: PathBuf,
        max_connections: u8,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            url,
            file_path,
            total_size: None,
            supports_range: false,
            etag: None,
            segments: Vec::new(),
            status: TaskStatus::Pending,
            created_at: Utc::now(),
            max_connections,
            download_dir,
            error_message: None,
        }
    }

    pub fn downloaded_bytes(&self) -> u64 {
        self.segments.iter().map(|s| s.downloaded).sum()
    }

    #[allow(dead_code)]
    pub fn progress_percent(&self) -> f64 {
        match self.total_size {
            Some(total) if total > 0 => self.downloaded_bytes() as f64 / total as f64 * 100.0,
            _ => 0.0,
        }
    }

    #[allow(dead_code)]
    pub fn is_complete(&self) -> bool {
        self.segments.iter().all(|s| s.is_complete())
    }
}
