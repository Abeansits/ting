//! Durable run outcome, separate from partially written final artifacts.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

pub const FILENAME: &str = "run-status.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Completed,
    Failed,
    Interrupted,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn is_terminal(self) -> bool {
        self != Self::Running
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunStatus {
    pub status: Status,
    pub pid: u32,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Write and sync an outcome before atomically publishing it to readers.
pub fn write(forum: &Path, status: Status, error: Option<String>) -> Result<()> {
    let record = RunStatus {
        status,
        pid: std::process::id(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        error,
    };
    let path = forum.join(FILENAME);
    let tmp = forum.join("run-status.json.tmp");
    let mut file = File::create(&tmp).context("Create run status")?;
    file.write_all(&serde_json::to_vec_pretty(&record)?)?;
    file.sync_all()?;
    fs::rename(tmp, path).context("Publish run status")?;
    if let Ok(dir) = File::open(forum) {
        dir.sync_all()?;
    }
    Ok(())
}

/// Resolve a dead runner as interrupted without mutating a historical session.
pub fn read(forum: &Path) -> Result<Option<RunStatus>> {
    let bytes = match fs::read(forum.join(FILENAME)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("Read run status"),
    };
    let mut record: RunStatus = serde_json::from_slice(&bytes).context("Invalid run status")?;
    #[cfg(unix)]
    if record.status == Status::Running {
        // Reject invalid PIDs before kill(0), which also accepts process groups.
        let alive = record.pid > 0 && record.pid <= i32::MAX as u32 && {
            let result = unsafe { libc::kill(record.pid as i32, 0) };
            result == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
        };
        if !alive {
            record.status = Status::Interrupted;
            record.error = Some("The runner exited before recording a final outcome.".into());
        }
    }
    Ok(Some(record))
}

/// Older forums have no outcome record; require every final artifact.
pub fn has_final_artifacts(forum: &Path) -> bool {
    ["synthesis.md", "claims.toml", "dissent.md", "meta-summary.toml"]
        .iter()
        .all(|name| forum.join("final").join(name).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_round_trip_and_dead_runner_is_interrupted() {
        let dir = std::env::temp_dir().join(format!("ting-status-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        for status in [Status::Running, Status::Failed, Status::Completed] {
            write(&dir, status, None).unwrap();
            assert_eq!(read(&dir).unwrap().unwrap().status, status);
        }
        #[cfg(unix)]
        {
            fs::write(dir.join(FILENAME), r#"{"status":"running","pid":0,"updated_at":"test"}"#).unwrap();
            assert_eq!(read(&dir).unwrap().unwrap().status, Status::Interrupted);
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
