use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Token-bucket style speed limiter
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

    /// Return the delay needed before consuming `bytes` more.
    /// Returns None if no delay is needed.
    pub fn consume(&self, bytes: u64) -> Option<Duration> {
        let limit = self.limit.load(Ordering::Relaxed);
        if limit == 0 {
            return None;
        }

        let mut window_start = self.window_start.lock().unwrap();
        let elapsed = window_start.elapsed();

        // Reset window every second
        if elapsed >= Duration::from_secs(1) {
            *window_start = Instant::now();
            self.consumed.store(0, Ordering::Relaxed);
        }

        let prev = self.consumed.fetch_add(bytes, Ordering::Relaxed);
        let total = prev + bytes;

        if total > limit {
            // Calculate how long we need to wait
            let overshoot = total - limit;
            let delay_ms = (overshoot as f64 / limit as f64 * 1000.0) as u64;
            Some(Duration::from_millis(delay_ms.max(1)))
        } else {
            None
        }
    }
}

/// Speed tracker for display purposes
pub struct SpeedTracker {
    samples: Vec<(Instant, u64)>,
    total_bytes: u64,
}

impl SpeedTracker {
    pub fn new() -> Self {
        Self {
            samples: Vec::new(),
            total_bytes: 0,
        }
    }

    pub fn record(&mut self, bytes: u64) {
        let now = Instant::now();
        self.total_bytes += bytes;
        self.samples.push((now, bytes));

        // Keep only last 5 seconds of samples
        let cutoff = now - Duration::from_secs(5);
        self.samples.retain(|(t, _)| *t >= cutoff);
    }

    /// Current speed in bytes/sec (average over last 5 seconds)
    pub fn speed_bps(&self) -> f64 {
        if self.samples.len() < 2 {
            return 0.0;
        }
        let first = self.samples.first().unwrap().0;
        let elapsed = self.samples.last().unwrap().0.duration_since(first);
        if elapsed.is_zero() {
            return 0.0;
        }
        let bytes: u64 = self.samples.iter().map(|(_, b)| b).sum();
        bytes as f64 / elapsed.as_secs_f64()
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

/// Create a shared speed limiter as Arc<AtomicU64> for worker compatibility
pub fn create_speed_limit_value(limit: Option<u64>) -> Arc<AtomicU64> {
    Arc::new(AtomicU64::new(limit.unwrap_or(0)))
}
