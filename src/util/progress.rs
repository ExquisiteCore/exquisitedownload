use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use super::speed::format_bytes;

/// Create a progress bar for a download task
pub fn create_download_progress(
    mp: &MultiProgress,
    total_size: u64,
    filename: &str,
) -> ProgressBar {
    let pb = mp.add(ProgressBar::new(total_size));
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} {prefix:.cyan.bold} [{bar:30.green/dim}] {bytes}/{total_bytes} ({bytes_per_sec}) ETA {eta}"
        )
        .unwrap()
        .progress_chars("█▓░"),
    );
    pb.set_prefix(truncate_filename(filename, 20));
    pb
}

/// Create a spinner for tasks without known size
pub fn create_unknown_size_progress(mp: &MultiProgress, filename: &str) -> ProgressBar {
    let pb = mp.add(ProgressBar::new_spinner());
    pb.set_style(
        ProgressStyle::with_template("{spinner:.green} {prefix:.cyan.bold} {bytes} ({bytes_per_sec})")
            .unwrap(),
    );
    pb.set_prefix(truncate_filename(filename, 20));
    pb
}

/// Create a shared downloaded bytes counter
pub fn create_byte_counter() -> Arc<AtomicU64> {
    Arc::new(AtomicU64::new(0))
}

/// Print a summary line after download completes
pub fn print_summary(filename: &str, total_size: u64, elapsed_secs: f64) {
    let speed = if elapsed_secs > 0.0 {
        total_size as f64 / elapsed_secs
    } else {
        0.0
    };

    println!(
        "\nDownload complete: {} ({}) in {:.1}s (avg {})",
        filename,
        format_bytes(total_size),
        elapsed_secs,
        super::speed::format_speed(speed),
    );
}

fn truncate_filename(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        name.to_string()
    } else {
        format!("{}...", &name[..max_len - 3])
    }
}
