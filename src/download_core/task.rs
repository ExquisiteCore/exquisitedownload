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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download_core::segment::Segment;

    fn make_task(segments: Vec<Segment>, total_size: Option<u64>) -> DownloadTask {
        let mut task = DownloadTask::new(
            "http://example.com/file".into(),
            "file.zip".into(),
            ".".into(),
            8,
        );
        task.segments = segments;
        task.total_size = total_size;
        task
    }

    #[test]
    fn test_downloaded_bytes() {
        let mut s1 = Segment::new(0, 0, 100);
        s1.downloaded = 50;
        let mut s2 = Segment::new(1, 100, 200);
        s2.downloaded = 30;
        let task = make_task(vec![s1, s2], Some(200));
        assert_eq!(task.downloaded_bytes(), 80);
    }

    #[test]
    fn test_downloaded_bytes_empty() {
        let task = make_task(vec![], None);
        assert_eq!(task.downloaded_bytes(), 0);
    }

    #[test]
    fn test_progress_percent() {
        let mut s1 = Segment::new(0, 0, 100);
        s1.downloaded = 25;
        let task = make_task(vec![s1], Some(100));
        assert!((task.progress_percent() - 25.0).abs() < 0.01);
    }

    #[test]
    fn test_progress_percent_unknown_size() {
        let task = make_task(vec![], None);
        assert!((task.progress_percent() - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_progress_percent_zero_size() {
        let task = make_task(vec![], Some(0));
        assert!((task.progress_percent() - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_is_complete() {
        let mut s1 = Segment::new(0, 0, 100);
        s1.downloaded = 100;
        let mut s2 = Segment::new(1, 100, 200);
        s2.downloaded = 100;
        let task = make_task(vec![s1, s2], Some(200));
        assert!(task.is_complete());
    }

    #[test]
    fn test_is_not_complete() {
        let mut s1 = Segment::new(0, 0, 100);
        s1.downloaded = 100;
        let s2 = Segment::new(1, 100, 200);
        let task = make_task(vec![s1, s2], Some(200));
        assert!(!task.is_complete());
    }
}
