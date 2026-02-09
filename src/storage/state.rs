use std::path::Path;

use anyhow::{Context, Result};

use crate::core::task::DownloadTask;

/// State file extension
const STATE_EXT: &str = ".edl.state";

/// Save task state to a JSON file alongside the download
pub async fn save_state(task: &DownloadTask, dir: &Path) -> Result<()> {
    let state_path = dir.join(format!("{}{}", task.id, STATE_EXT));
    let json = serde_json::to_string_pretty(task).context("failed to serialize task state")?;
    tokio::fs::write(&state_path, json)
        .await
        .context("failed to write state file")?;
    Ok(())
}

/// Load task state from a state file
pub async fn load_state(task_id: &str, dir: &Path) -> Result<Option<DownloadTask>> {
    let state_path = dir.join(format!("{}{}", task_id, STATE_EXT));
    if !state_path.exists() {
        return Ok(None);
    }
    let json = tokio::fs::read_to_string(&state_path)
        .await
        .context("failed to read state file")?;
    let task: DownloadTask =
        serde_json::from_str(&json).context("failed to deserialize task state")?;
    Ok(Some(task))
}

/// Remove state file
pub async fn remove_state(task_id: &str, dir: &Path) -> Result<()> {
    let state_path = dir.join(format!("{}{}", task_id, STATE_EXT));
    if state_path.exists() {
        tokio::fs::remove_file(&state_path)
            .await
            .context("failed to remove state file")?;
    }
    Ok(())
}

/// Find all state files in a directory
pub async fn find_all_states(dir: &Path) -> Result<Vec<DownloadTask>> {
    let mut tasks = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.ends_with(STATE_EXT) {
                let json = tokio::fs::read_to_string(&path).await?;
                if let Ok(task) = serde_json::from_str::<DownloadTask>(&json) {
                    tasks.push(task);
                }
            }
        }
    }
    Ok(tasks)
}
