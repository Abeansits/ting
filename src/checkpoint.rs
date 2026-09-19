//! Durable operation results and input fingerprints for resumable forums.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

// Bump for incompatible generation semantics, not release-only changes.
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    input_hash: String,
    output_hash: String,
    output: String,
    materialized: bool,
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn record_path(output: &Path) -> Result<PathBuf> {
    let parent = output.parent().context("Checkpoint output has no parent")?;
    let name = output
        .file_name()
        .context("Checkpoint output has no filename")?;
    Ok(parent
        .join(".checkpoints")
        .join(format!("{}.json", name.to_string_lossy())))
}

/// Preserve superseded artifacts rather than silently treating them as current.
pub fn archive(output: &Path) -> Result<()> {
    if output.exists() {
        anyhow::ensure!(
            output.is_file(),
            "Expected an artifact file: {}",
            output.display()
        );
        let dir = output
            .parent()
            .context("Artifact has no parent")?
            .join(".checkpoints/obsolete");
        fs::create_dir_all(&dir)?;
        fs::rename(
            output,
            dir.join(format!(
                "{}-{}",
                uuid::Uuid::new_v4(),
                output.file_name().unwrap().to_string_lossy()
            )),
        )?;
    }
    Ok(())
}

/// A manual answer for an older prompt must not satisfy a changed assignment.
pub fn prepare_manual(output: &Path, input: &Value) -> Result<()> {
    let hash = digest(&serde_json::to_vec(&(VERSION, input))?);
    let context_path = record_path(output)?.with_extension("manual-input.json");
    match fs::read_to_string(&context_path) {
        Ok(previous) if previous != hash => archive(output)?,
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    write_durable(&context_path, hash.as_bytes())?;
    match fs::read(record_path(output)?) {
        Ok(bytes) => {
            let record: Record = serde_json::from_slice(&bytes)?;
            if record.input_hash == hash {
                text(output, input.clone(), true, || {
                    anyhow::bail!("Missing manual checkpoint")
                })?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// Record an observed manual answer without rewriting a file owned by its author.
pub fn observe_manual(output: &Path, input: Value, answer: &str) -> Result<()> {
    let record = Record {
        version: VERSION,
        input_hash: digest(&serde_json::to_vec(&(VERSION, input))?),
        output_hash: digest(answer.as_bytes()),
        output: answer.to_string(),
        materialized: true,
    };
    write_durable(&record_path(output)?, &serde_json::to_vec_pretty(&record)?)
}

pub fn write_durable(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Output has no parent")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".ting-write-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

/// Editable inputs can be adopted only when their generating input still matches.
/// Derived artifacts are restored from recorded output, never silently adopted.
pub fn text(
    output: &Path,
    input: Value,
    editable: bool,
    generate: impl FnOnce() -> Result<String>,
) -> Result<String> {
    let path = record_path(output)?;
    // Value uses sorted object keys, including nested maps.
    let input_hash = digest(&serde_json::to_vec(&(VERSION, input))?);
    let existing = match fs::read(&path) {
        Ok(bytes) => {
            Some(serde_json::from_slice::<Record>(&bytes).context("Invalid operation checkpoint")?)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("Read checkpoint"),
    };
    if let Some(record) = &existing {
        anyhow::ensure!(record.version == VERSION, "Unsupported checkpoint version");
    }
    let mut record =
        if let Some(mut record) = existing.filter(|record| record.input_hash == input_hash) {
            eprintln!(
                "  Reusing {}",
                output.file_name().unwrap().to_string_lossy()
            );
            anyhow::ensure!(record.version == VERSION, "Unsupported checkpoint version");
            anyhow::ensure!(
                digest(record.output.as_bytes()) == record.output_hash,
                "Checkpoint output checksum mismatch"
            );
            if record.materialized {
                match fs::read_to_string(output) {
                    Ok(edited) => {
                        if editable && edited != record.output {
                            record.output = edited.clone();
                            record.output_hash = digest(record.output.as_bytes());
                            write_durable(&path, &serde_json::to_vec_pretty(&record)?)?;
                        }
                        if edited == record.output {
                            return Ok(record.output);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error).context("Read editable artifact"),
                }
            }
            record
        } else {
            archive(output)?;
            let value = generate()?;
            Record {
                version: VERSION,
                input_hash,
                output_hash: digest(value.as_bytes()),
                output: value,
                materialized: false,
            }
        };
    // Journal before materialization. After a crash, restore the journaled output
    // rather than misidentifying an old file as a user edit of the new result.
    record.materialized = false;
    write_durable(&path, &serde_json::to_vec_pretty(&record)?)?;
    write_durable(output, record.output.as_bytes())?;
    record.materialized = true;
    write_durable(&path, &serde_json::to_vec_pretty(&record)?)?;
    Ok(record.output)
}

pub fn json<T: Serialize + DeserializeOwned>(
    path: &Path,
    input: Value,
    generate: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let value = text(path, input, false, || {
        Ok(serde_json::to_string_pretty(&generate()?)?)
    })?;
    serde_json::from_str(&value).context("Invalid typed checkpoint result")
}

/// A kernel-held lock is released even when the runner is killed.
pub struct ForumLock(File);

/// Probe a checkpoint-era runner without creating or changing session files.
#[cfg(unix)]
pub fn runner_is_active(forum: &Path) -> Result<Option<bool>> {
    use std::os::fd::AsRawFd;
    if !forum.join("run-options.json").exists() {
        return Ok(None);
    }
    let file = match File::open(forum.join(".runner.lock")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Some(false)),
        Err(error) => return Err(error.into()),
    };
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        unsafe {
            libc::flock(file.as_raw_fd(), libc::LOCK_UN);
        }
        return Ok(Some(false));
    }
    let error = std::io::Error::last_os_error();
    if matches!(error.raw_os_error(), Some(libc::EWOULDBLOCK)) {
        Ok(Some(true))
    } else {
        Err(error.into())
    }
}

impl ForumLock {
    pub fn acquire(forum: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(forum.join(".runner.lock"))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result != 0 {
                return Err(std::io::Error::last_os_error())
                    .context("Another runner holds this forum's lock");
            }
        }
        #[cfg(not(unix))]
        anyhow::bail!("Resumable forum locks currently require Unix");
        Ok(Self(file))
    }
}
impl Drop for ForumLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            unsafe {
                libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn journal_recovers_after_materialization_failure() {
        let dir = std::env::temp_dir().join(format!("ting-journal-{}", uuid::Uuid::new_v4()));
        let output = dir.join("answer.md");
        fs::create_dir_all(&dir).unwrap();
        assert!(
            text(&output, json!("input"), true, || {
                fs::create_dir(&output)?;
                Ok("paid result".into())
            })
            .is_err()
        );
        fs::remove_dir(&output).unwrap();
        assert_eq!(
            text(&output, json!("input"), true, || panic!(
                "must not repeat provider call"
            ))
            .unwrap(),
            "paid result"
        );
        let mut record: Value =
            serde_json::from_slice(&fs::read(record_path(&output).unwrap()).unwrap()).unwrap();
        record["output"] = json!("corrupted");
        fs::write(record_path(&output).unwrap(), record.to_string()).unwrap();
        assert!(
            text(&output, json!("input"), true, || panic!(
                "must report corruption"
            ))
            .is_err()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_recomputation_does_not_leave_a_stale_artifact() {
        let dir = std::env::temp_dir().join(format!("ting-invalidation-{}", uuid::Uuid::new_v4()));
        let output = dir.join("score.json");
        text(&output, json!("old evidence"), false, || {
            Ok("old score".into())
        })
        .unwrap();
        assert!(
            text(&output, json!("new evidence"), false, || anyhow::bail!(
                "provider failed"
            ))
            .is_err()
        );
        assert!(!output.exists());
        assert_eq!(
            text(&output, json!("old evidence"), false, || panic!(
                "old checkpoint should still be available"
            ))
            .unwrap(),
            "old score"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn changed_manual_prompt_archives_the_old_answer() {
        let dir =
            std::env::temp_dir().join(format!("ting-manual-context-{}", uuid::Uuid::new_v4()));
        let output = dir.join("human.md");
        prepare_manual(&output, &json!("first prompt")).unwrap();
        text(&output, json!("first prompt"), true, || {
            Ok("old answer".into())
        })
        .unwrap();
        prepare_manual(&output, &json!("new prompt")).unwrap();
        assert!(!output.exists());
        assert_eq!(
            fs::read_dir(dir.join(".checkpoints/obsolete"))
                .unwrap()
                .count(),
            1
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn restore_invalidate_edit_and_lock() {
        let dir = std::env::temp_dir().join(format!("ting-checkpoint-{}", uuid::Uuid::new_v4()));
        let file = dir.join("answer.md");
        assert_eq!(
            text(&file, json!({"prompt":"first"}), true, || Ok(
                "answer".into()
            ))
            .unwrap(),
            "answer"
        );
        fs::remove_file(&file).unwrap();
        assert_eq!(
            text(&file, json!({"prompt":"first"}), true, || panic!(
                "must reuse"
            ))
            .unwrap(),
            "answer"
        );
        fs::write(&file, "edited answer").unwrap();
        assert_eq!(
            text(&file, json!({"prompt":"first"}), true, || panic!(
                "must adopt edit"
            ))
            .unwrap(),
            "edited answer"
        );
        assert_eq!(
            text(&file, json!({"prompt":"second"}), true, || Ok(
                "fresh answer".into()
            ))
            .unwrap(),
            "fresh answer"
        );
        let lock = ForumLock::acquire(&dir).unwrap();
        assert!(ForumLock::acquire(&dir).is_err());
        drop(lock);
        assert!(ForumLock::acquire(&dir).is_ok());
        fs::remove_dir_all(dir).unwrap();
    }
}
