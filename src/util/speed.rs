use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Token-bucket style speed limiter shared across all workers
pub struct SpeedLimiter {
    /// Max bytes per second (0 = unlimited)
    limit: AtomicU64,
    /// Bytes consumed in the current window
    consumed: AtomicU64,
    /// Window start time
    window_start: std::sync::Mutex<Instant>,
}

impl SpeedLimiter {
    pub fn new(limit: u64) -> Self {
        Self {
            limit: AtomicU64::new(limit),
            consumed: AtomicU64::new(0),
            window_start: std::sync::Mutex::new(Instant::now()),
        }
    }

    pub fn set_limit(&self, limit: u64) {
        self.limit.store(limit, Ordering::Relaxed);
    }

    pub fn limit(&self) -> u64 {
        self.limit.load(Ordering::Relaxed)
    }

    /// Consume `bytes` from the bucket. Returns delay to wait if over limit.
    /// Uses a 100ms window for smoother throughput compared to 1-second bursts.
    pub fn consume(&self, bytes: u64) -> Option<Duration> {
        const WINDOW_MS: u64 = 100;

        let limit = self.limit.load(Ordering::Relaxed);
        if limit == 0 {
            return None;
        }

        // Lock only to check/reset window, then release immediately
        {
            let mut window_start = self.window_start.lock().unwrap();
            if window_start.elapsed() >= Duration::from_millis(WINDOW_MS) {
                *window_start = Instant::now();
                self.consumed.store(0, Ordering::Relaxed);
            }
        }

        let window_budget = limit * WINDOW_MS / 1000;
        let prev = self.consumed.fetch_add(bytes, Ordering::Relaxed);
        let total = prev + bytes;

        if total > window_budget {
            let overshoot = total - window_budget;
            let delay_ms = (overshoot as f64 / limit as f64 * 1000.0) as u64;
            Some(Duration::from_millis(delay_ms.max(1)))
        } else {
            None
        }
    }
}

/// Format bytes per second to human-readable string
pub fn format_speed(bps: f64) -> String {
    if bps >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GB/s", bps / (1024.0 * 1024.0 * 1024.0))
    } else if bps >= 1024.0 * 1024.0 {
        format!("{:.1} MB/s", bps / (1024.0 * 1024.0))
    } else if bps >= 1024.0 {
        format!("{:.1} KB/s", bps / 1024.0)
    } else {
        format!("{:.0} B/s", bps)
    }
}

/// Format bytes to human-readable string
pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.00 GB");
    }

    #[test]
    fn test_format_speed() {
        assert_eq!(format_speed(0.0), "0 B/s");
        assert_eq!(format_speed(512.0), "512 B/s");
        assert_eq!(format_speed(1024.0), "1.0 KB/s");
        assert_eq!(format_speed(1024.0 * 1024.0), "1.0 MB/s");
        assert_eq!(format_speed(1024.0 * 1024.0 * 1024.0), "1.0 GB/s");
    }

    #[test]
    fn test_speed_limiter_unlimited() {
        let limiter = SpeedLimiter::new(0);
        assert!(limiter.consume(1024).is_none());
        assert!(limiter.consume(999999).is_none());
    }

    #[test]
    fn test_speed_limiter_under_limit() {
        let limiter = SpeedLimiter::new(10000);
        // 100ms window budget = 10000 * 100 / 1000 = 1000
        assert!(limiter.consume(500).is_none());
    }

    #[test]
    fn test_speed_limiter_over_limit() {
        let limiter = SpeedLimiter::new(10000);
        // 100ms window budget = 1000
        assert!(limiter.consume(500).is_none());
        // Second consume puts us over the 1000 budget
        let delay = limiter.consume(600);
        assert!(delay.is_some());
    }

    #[test]
    fn test_speed_limiter_set_limit() {
        let limiter = SpeedLimiter::new(1000);
        assert_eq!(limiter.limit(), 1000);
        limiter.set_limit(2000);
        assert_eq!(limiter.limit(), 2000);
    }
}
