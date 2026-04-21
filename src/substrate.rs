use anyhow::{Context, Result};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub fn sessions_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".ting").join("sessions")
}

pub fn forum_dir(id: &str) -> PathBuf {
    sessions_dir().join(id)
}

pub fn create_forum_dir(id: &str) -> Result<PathBuf> {
    let dir = forum_dir(id);
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create forum directory: {}", dir.display()))?;
    Ok(dir)
}

pub fn round_dir(forum: &Path, round: u32) -> PathBuf {
    forum.join(format!("round-{}", round))
}

pub fn create_round_dir(forum: &Path, round: u32) -> Result<PathBuf> {
    let dir = round_dir(forum, round);
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create round directory: {}", dir.display()))?;
    Ok(dir)
}

pub fn create_final_dir(forum: &Path) -> Result<PathBuf> {
    let dir = forum.join("final");
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create final directory: {}", dir.display()))?;
    Ok(dir)
}

/// Write a file atomically: write to .tmp, then rename
pub fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let tmp_path = path.with_extension("md.tmp");
    fs::write(&tmp_path, content)
        .with_context(|| format!("Failed to write temp file: {}", tmp_path.display()))?;
    fs::rename(&tmp_path, path)
        .with_context(|| format!("Failed to rename {} -> {}", tmp_path.display(), path.display()))?;
    Ok(())
}

/// Write a TOML file atomically
pub fn write_atomic_toml(path: &Path, content: &str) -> Result<()> {
    let tmp_path = path.with_extension("toml.tmp");
    fs::write(&tmp_path, content)
        .with_context(|| format!("Failed to write temp file: {}", tmp_path.display()))?;
    fs::rename(&tmp_path, path)
        .with_context(|| format!("Failed to rename {} -> {}", tmp_path.display(), path.display()))?;
    Ok(())
}

pub fn read_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("Failed to read: {}", path.display()))
}

pub fn read_response(forum: &Path, round: u32, participant: &str) -> Result<Option<String>> {
    let path = forum
        .join(format!("round-{}", round))
        .join(format!("{}.md", participant));
    if path.exists() {
        Ok(Some(read_file(&path)?))
    } else {
        Ok(None)
    }
}

pub fn read_all_responses(
    forum: &Path,
    round: u32,
    participants: &[String],
) -> Result<HashMap<String, String>> {
    let mut responses = HashMap::new();
    for name in participants {
        if let Some(content) = read_response(forum, round, name)? {
            responses.insert(name.clone(), content);
        }
    }
    Ok(responses)
}

/// Watch a directory for expected participant response files using notify.
/// Returns collected responses when all are present or timeout is reached.
/// Shows a live countdown on TTY and word count per response.
pub fn watch_for_responses(
    round_dir: &Path,
    expected: &[String],
    timeout: Duration,
) -> Result<HashMap<String, String>> {
    let mut responses = HashMap::new();
    let start = Instant::now();
    let is_tty = std::io::IsTerminal::is_terminal(&std::io::stderr());

    // Start watcher BEFORE scanning for existing files to avoid race condition
    // (file could arrive between scan and watch registration)
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = RecommendedWatcher::new(tx, Config::default())
        .with_context(|| "Failed to create filesystem watcher")?;
    watcher
        .watch(round_dir, RecursiveMode::NonRecursive)
        .with_context(|| format!("Failed to watch directory: {}", round_dir.display()))?;

    // Now check for files already present
    for name in expected {
        let path = round_dir.join(format!("{}.md", name));
        if path.exists() {
            let content = read_file(&path)?;
            let words = content.split_whitespace().count();
            eprintln!("  \u{2713} {} responded ({} words)", name, words);
            responses.insert(name.clone(), content);
        }
    }

    if responses.len() == expected.len() {
        return Ok(responses);
    }

    // Print initial countdown
    print_countdown(is_tty, timeout.saturating_sub(start.elapsed()));

    loop {
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            if is_tty {
                eprint!("\r\x1b[K"); // clear countdown line
            }
            break;
        }
        let remaining = timeout - elapsed;
        let poll = Duration::from_secs(15);
        let wait_time = remaining.min(poll);

        match rx.recv_timeout(wait_time) {
            Ok(Ok(event)) => {
                for path in &event.paths {
                    if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                        if let Some(name) = filename.strip_suffix(".md") {
                            if expected.contains(&name.to_string())
                                && !responses.contains_key(name)
                                && !name.ends_with(".tmp") // ignore temp files
                            {
                                // Retry with bounded backoff for atomic rename
                                let mut read_ok = false;
                                for delay_ms in [10, 50, 100, 200] {
                                    std::thread::sleep(Duration::from_millis(delay_ms));
                                    if path.exists() {
                                        if let Ok(content) = read_file(path) {
                                            if !content.is_empty() {
                                                if is_tty {
                                                    eprint!("\r\x1b[K"); // clear countdown line
                                                }
                                                let words = content.split_whitespace().count();
                                                eprintln!("  \u{2713} {} responded ({} words)", name, words);
                                                responses.insert(name.to_string(), content);
                                                read_ok = true;
                                                break;
                                            }
                                        }
                                    }
                                }
                                if !read_ok {
                                    eprintln!("  Warning: could not read response from {}", name);
                                }
                            }
                        }
                    }
                }
                // Refresh countdown if still waiting
                if responses.len() < expected.len() {
                    print_countdown(is_tty, timeout.saturating_sub(start.elapsed()));
                }
            }
            Ok(Err(e)) => eprintln!("Watch error: {}", e),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Refresh countdown
                let left = timeout.saturating_sub(start.elapsed());
                if left > Duration::ZERO && responses.len() < expected.len() {
                    print_countdown(is_tty, left);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if responses.len() == expected.len() {
            break;
        }
    }

    Ok(responses)
}

fn print_countdown(is_tty: bool, remaining: Duration) {
    if !is_tty || remaining.is_zero() {
        return;
    }
    let mins = remaining.as_secs() / 60;
    let secs = remaining.as_secs() % 60;
    eprint!("\r  Watching for your file... (timeout in {}m{:02}s)  ", mins, secs);
}

/// List all forum IDs and their directory paths
pub fn list_forums() -> Result<Vec<(String, PathBuf)>> {
    let dir = sessions_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut forums = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let meta_path = entry.path().join("meta.toml");
            if meta_path.exists() {
                if let Some(name) = entry.file_name().to_str() {
                    forums.push((name.to_string(), entry.path()));
                }
            }
        }
    }
    forums.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(forums)
}

/// Determine current round number from existing round directories
pub fn current_round(forum: &Path) -> u32 {
    let mut round = 0;
    loop {
        let next = forum.join(format!("round-{}", round + 1));
        if next.exists() {
            round += 1;
        } else {
            break;
        }
    }
    round
}

/// Check if a forum has completed (final/synthesis.md exists)
pub fn is_completed(forum: &Path) -> bool {
    forum.join("final").join("synthesis.md").exists()
}

/// Invoke a participant command with timeout.
///
/// The prompt is delivered via:
///   1. **stdin** — piped to the child (only if command does NOT contain `{prompt_file}`)
///   2. **{prompt_file}** — replaced with a temp file path in the command template
///   3. **$TING_PROMPT_FILE** — env var pointing to the same temp file
///
/// Stdin and {prompt_file} are mutually exclusive to avoid double delivery.
///
/// Recommended command patterns:
///   - Codex:    `codex exec --full-auto -`           (reads stdin)
///   - Gemini:   `gemini --prompt`                    (reads stdin)
///   - Claude:   `cat {prompt_file} | claude -p -`    (pipe from file, no shell expansion)
///   - OpenCode: `opencode run`                       (reads stdin)
///   - Any CLI:  `cat {prompt_file} | some-cli`       (pipe through cat)
pub fn invoke_command(
    command_template: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<String> {
    use std::io::{self, Write};
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};

    let tmp_file = std::env::temp_dir().join(format!("ting-{}.md", uuid::Uuid::new_v4()));
    fs::write(&tmp_file, prompt)
        .with_context(|| "Failed to write prompt temp file")?;

    // Guard: clean up temp file on all exit paths
    let tmp_file_cleanup = tmp_file.clone();
    let _cleanup = CleanupGuard(Some(tmp_file_cleanup));

    // If command uses {prompt_file}, substitute it and DON'T pipe stdin (avoid double delivery)
    let uses_prompt_file = command_template.contains("{prompt_file}");
    let command = command_template
        .replace("{prompt_file}", &tmp_file.display().to_string());

    let prompt_for_stdin = if uses_prompt_file {
        None
    } else {
        Some(prompt.to_string())
    };
    let tmp_display = tmp_file.display().to_string();
    let cmd_for_thread = command.clone();

    // Share the child PID so we can kill the process group on timeout
    let child_pid: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
    let child_pid_for_thread = child_pid.clone();

    // Run in a thread so we can enforce a timeout
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = (|| -> io::Result<std::process::Output> {
            let mut cmd = std::process::Command::new("sh");
            cmd.arg("-c")
                .arg(&cmd_for_thread)
                .stdin(if prompt_for_stdin.is_some() {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .env("TING_PROMPT_FILE", &tmp_display);

            // Make child a process group leader so kill(-pgid) reaps all descendants
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                unsafe {
                    cmd.pre_exec(|| {
                        libc::setpgid(0, 0);
                        Ok(())
                    });
                }
            }

            let mut child = cmd.spawn()?;

            // Store PID (= PGID since we called setpgid) for timeout kill
            *child_pid_for_thread.lock().unwrap() = Some(child.id());

            // Write stdin in a separate thread to avoid deadlock:
            // if the child fills stdout/stderr before reading all stdin,
            // write_all blocks while wait_with_output isn't draining yet.
            let stdin_handle = if let Some(prompt_data) = prompt_for_stdin {
                let stdin = child.stdin.take();
                Some(std::thread::spawn(move || {
                    if let Some(mut stdin) = stdin {
                        match stdin.write_all(prompt_data.as_bytes()) {
                            Ok(()) => {}
                            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {}
                            Err(e) => eprintln!("  Warning: stdin write error: {}", e),
                        }
                    }
                }))
            } else {
                None
            };

            let output = child.wait_with_output()?;
            if let Some(h) = stdin_handle {
                let _ = h.join();
            }
            Ok(output)
        })();
        tx.send(result).ok();
    });

    let output = match rx.recv_timeout(timeout) {
        Ok(result) => result.with_context(|| format!("Failed to execute: {}", command_template))?,
        Err(_) => {
            // Timeout: kill the process group
            if let Some(pid) = *child_pid.lock().unwrap() {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
                #[cfg(windows)]
                {
                    // Best-effort kill on Windows
                    let _ = std::process::Command::new("taskkill")
                        .args(&["/F", "/T", "/PID", &pid.to_string()])
                        .output();
                }
            }
            // Join the worker thread to prevent leak
            let _ = worker.join();
            anyhow::bail!(
                "Command timed out after {:?}: {}",
                timeout,
                command_template
            );
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.is_empty() { &stdout } else { &stderr };
        anyhow::bail!("Command failed ({}): {}", command_template, detail);
    }

    String::from_utf8(output.stdout)
        .with_context(|| "Invalid UTF-8 in command output")
        .map(|s| s.trim().to_string())
}

/// RAII guard that deletes a temp file when dropped (any exit path)
struct CleanupGuard(Option<std::path::PathBuf>);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if let Some(ref path) = self.0 {
            if let Err(e) = fs::remove_file(path) {
                eprintln!("  Warning: failed to clean up temp file {}: {}", path.display(), e);
            }
        }
    }
}

/// Invoke a model for fire keeper internal operations (synthesis, convergence).
/// If a custom command is provided, routes through invoke_command.
/// Otherwise falls back to the claude CLI with the given model ID.
pub fn invoke_fire_keeper_model(
    custom_command: Option<&str>,
    model: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<String> {
    if let Some(cmd) = custom_command {
        invoke_command(cmd, prompt, timeout)
    } else {
        invoke_claude(model, prompt)
    }
}

/// Invoke the claude CLI directly (no shell, safe from metacharacters)
fn invoke_claude(model: &str, prompt: &str) -> Result<String> {
    let output = std::process::Command::new("claude")
        .arg("--model")
        .arg(model)
        .arg("-p")
        .arg(prompt)
        .arg("--output-format")
        .arg("text")
        .output()
        .with_context(|| {
            "Failed to invoke 'claude' CLI. Ensure Claude Code is installed and in PATH."
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Claude CLI failed: {}", stderr);
    }

    String::from_utf8(output.stdout)
        .with_context(|| "Invalid UTF-8 in model output")
        .map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_atomic() {
        let dir = std::env::temp_dir().join("ting-test-atomic");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.md");

        write_atomic(&path, "hello world").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello world");
        assert!(!path.with_extension("md.tmp").exists());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_write_atomic_toml() {
        let dir = std::env::temp_dir().join("ting-test-atomic-toml");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("claims.toml");

        write_atomic_toml(&path, "[test]\nkey = \"value\"").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[test]\nkey = \"value\""
        );
        assert!(!path.with_extension("toml.tmp").exists());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_current_round() {
        let dir = std::env::temp_dir().join("ting-test-rounds");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        assert_eq!(current_round(&dir), 0);

        fs::create_dir_all(dir.join("round-1")).unwrap();
        assert_eq!(current_round(&dir), 1);

        fs::create_dir_all(dir.join("round-2")).unwrap();
        assert_eq!(current_round(&dir), 2);

        // Gap: round-3 missing, round-4 exists — should stop at 2
        fs::create_dir_all(dir.join("round-4")).unwrap();
        assert_eq!(current_round(&dir), 2);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_is_completed() {
        let dir = std::env::temp_dir().join("ting-test-completed");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        assert!(!is_completed(&dir));

        let final_dir = dir.join("final");
        fs::create_dir_all(&final_dir).unwrap();
        assert!(!is_completed(&dir)); // dir exists but no synthesis.md

        fs::write(final_dir.join("synthesis.md"), "done").unwrap();
        assert!(is_completed(&dir));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_read_all_responses() {
        let dir = std::env::temp_dir().join("ting-test-responses");
        let _ = fs::remove_dir_all(&dir);
        let round_dir = dir.join("round-1");
        fs::create_dir_all(&round_dir).unwrap();

        fs::write(round_dir.join("alice.md"), "Alice's response").unwrap();
        fs::write(round_dir.join("bob.md"), "Bob's response").unwrap();

        let participants = vec!["alice".to_string(), "bob".to_string(), "charlie".to_string()];
        let responses = read_all_responses(&dir, 1, &participants).unwrap();

        assert_eq!(responses.len(), 2);
        assert_eq!(responses["alice"], "Alice's response");
        assert_eq!(responses["bob"], "Bob's response");
        assert!(!responses.contains_key("charlie"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_invoke_command_stdin_piping() {
        // Command reads from stdin — should get the prompt
        let result = invoke_command("cat", "hello from stdin", Duration::from_secs(5));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "hello from stdin");
    }

    #[test]
    fn test_invoke_command_stdin_with_metacharacters() {
        // Prompt with shell metacharacters must pass through safely via stdin
        let prompt = "Use `backticks` and $HOME and \"quotes\" and $(echo danger)";
        let result = invoke_command("cat", prompt, Duration::from_secs(5));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), prompt);
    }

    #[test]
    fn test_invoke_command_prompt_file() {
        // Command reads from {prompt_file} — file should exist and contain the prompt
        let result = invoke_command(
            "cat {prompt_file}",
            "hello from file",
            Duration::from_secs(5),
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "hello from file");
    }

    #[test]
    fn test_invoke_command_env_var() {
        // Command reads TING_PROMPT_FILE env var
        let result = invoke_command(
            "cat $TING_PROMPT_FILE",
            "hello from env",
            Duration::from_secs(5),
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "hello from env");
    }

    #[test]
    fn test_invoke_command_timeout() {
        // Command that exceeds timeout should fail
        let result = invoke_command("sleep 30", "ignored", Duration::from_secs(1));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("timed out"), "Expected timeout error, got: {}", err);
    }
}
