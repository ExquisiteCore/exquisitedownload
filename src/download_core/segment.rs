use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SegmentStatus {
    Pending,
    Downloading,
    Completed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub index: u32,
    /// Start byte offset (inclusive)
    pub start: u64,
    /// End byte offset (exclusive)
    pub end: u64,
    /// Bytes downloaded so far
    pub downloaded: u64,
    pub status: SegmentStatus,
}

impl Segment {
    pub fn new(index: u32, start: u64, end: u64) -> Self {
        Self {
            index,
            start,
            end,
            downloaded: 0,
            status: SegmentStatus::Pending,
        }
    }

    pub fn size(&self) -> u64 {
        self.end - self.start
    }

    pub fn is_complete(&self) -> bool {
        self.downloaded >= self.size()
    }

    /// The byte offset to resume downloading from
    pub fn resume_offset(&self) -> u64 {
        self.start + self.downloaded
    }

    /// Remaining bytes to download
    #[allow(dead_code)]
    pub fn remaining(&self) -> u64 {
        self.size().saturating_sub(self.downloaded)
    }

    /// Temp file path for this segment
    pub fn temp_filename(&self, task_id: &str) -> String {
        format!("{}.part{}", task_id, self.index)
    }
}

/// Split a total size into N segments
pub fn create_segments(total_size: u64, num_segments: u8) -> Vec<Segment> {
    let n = num_segments as u64;
    let segment_size = total_size / n;
    let remainder = total_size % n;

    let mut segments = Vec::with_capacity(num_segments as usize);
    let mut offset = 0u64;

    for i in 0..n {
        let extra = if i < remainder { 1 } else { 0 };
        let size = segment_size + extra;
        segments.push(Segment::new(i as u32, offset, offset + size));
        offset += size;
    }

    segments
}

/// Create a single segment covering the whole file (when Range not supported)
pub fn create_single_segment(total_size: u64) -> Vec<Segment> {
    vec![Segment::new(0, 0, total_size)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_segments() {
        let segments = create_segments(100, 3);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].start, 0);
        assert_eq!(segments[0].end, 34); // 33 + 1 remainder
        assert_eq!(segments[1].start, 34);
        assert_eq!(segments[1].end, 67); // 33
        assert_eq!(segments[2].start, 67);
        assert_eq!(segments[2].end, 100);

        let total: u64 = segments.iter().map(|s| s.size()).sum();
        assert_eq!(total, 100);
    }

    #[test]
    fn test_create_segments_even() {
        let segments = create_segments(100, 4);
        assert_eq!(segments.len(), 4);
        let total: u64 = segments.iter().map(|s| s.size()).sum();
        assert_eq!(total, 100);
    }

    #[test]
    fn test_segment_resume() {
        let mut seg = Segment::new(0, 100, 200);
        seg.downloaded = 50;
        assert_eq!(seg.resume_offset(), 150);
        assert_eq!(seg.remaining(), 50);
        assert!(!seg.is_complete());

        seg.downloaded = 100;
        assert!(seg.is_complete());
    }
}
