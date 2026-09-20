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
    fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "Failed to rename {} -> {}",
            tmp_path.display(),
            path.display()
        )
    })?;
    Ok(())
}

/// Write a TOML file atomically
pub fn write_atomic_toml(path: &Path, content: &str) -> Result<()> {
    let tmp_path = path.with_extension("toml.tmp");
    fs::write(&tmp_path, content)
        .with_context(|| format!("Failed to write temp file: {}", tmp_path.display()))?;
    fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "Failed to rename {} -> {}",
            tmp_path.display(),
            path.display()
        )
    })?;
    Ok(())
}

pub fn read_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("Failed to read: {}", path.display()))
}

#[cfg(test)]
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

#[cfg(test)]
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
pub fn watch_for_responses<F>(
    round_dir: &Path,
    expected: &[String],
    timeout: Duration,
    mut on_response: F,
) -> Result<HashMap<String, String>>
where
    F: FnMut(&str, &str) -> Result<()>,
{
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
            on_response(name, &content)?;
            responses.insert(name.clone(), content);
        }
    }

    if responses.len() == expected.len() {
        return Ok(responses);
    }

    // Print initial countdown
    print_countdown(is_tty, timeout.saturating_sub(start.elapsed()));

    loop {
        crate::cancellation::check()?;
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            if is_tty {
                eprint!("\r\x1b[K"); // clear countdown line
            }
            break;
        }
        let remaining = timeout - elapsed;
        let poll = if crate::cancellation::current().is_some() {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(15)
        };
        let wait_time = remaining.min(poll);

        match rx.recv_timeout(wait_time) {
            Ok(Ok(event)) => {
                for path in &event.paths {
                    if let Some(filename) = path.file_name().and_then(|f| f.to_str())
                        && let Some(name) = filename.strip_suffix(".md")
                        && expected.contains(&name.to_string())
                        && !responses.contains_key(name)
                        && !name.ends_with(".tmp")
                    // ignore temp files
                    {
                        // Retry with bounded backoff for atomic rename
                        let mut read_ok = false;
                        for delay_ms in [10, 50, 100, 200] {
                            std::thread::sleep(Duration::from_millis(delay_ms));
                            if path.exists()
                                && let Ok(content) = read_file(path)
                                && !content.is_empty()
                            {
                                if is_tty {
                                    eprint!("\r\x1b[K"); // clear countdown line
                                }
                                let words = content.split_whitespace().count();
                                eprintln!("  \u{2713} {} responded ({} words)", name, words);
                                on_response(name, &content)?;
                                responses.insert(name.to_string(), content);
                                read_ok = true;
                                break;
                            }
                        }
                        if !read_ok {
                            eprintln!("  Warning: could not read response from {}", name);
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
    eprint!(
        "\r  Watching for your file... (timeout in {}m{:02}s)  ",
        mins, secs
    );
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
            if meta_path.exists()
                && let Some(name) = entry.file_name().to_str()
            {
                forums.push((name.to_string(), entry.path()));
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

/// Require a successful outcome and all final artifacts; support legacy forums.
pub fn is_completed(forum: &Path) -> bool {
    let successful = match crate::run_status::read(forum) {
        Ok(Some(record)) => record.status == crate::run_status::Status::Completed,
        Ok(None) => true,
        Err(_) => false,
    };
    successful && crate::run_status::has_final_artifacts(forum)
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
pub fn invoke_command(command_template: &str, prompt: &str, timeout: Duration) -> Result<String> {
    let tmp_file = std::env::temp_dir().join(format!("ting-{}.md", uuid::Uuid::new_v4()));
    fs::write(&tmp_file, prompt).with_context(|| "Failed to write prompt temp file")?;

    // Guard: clean up temp file on all exit paths
    let tmp_file_cleanup = tmp_file.clone();
    let _cleanup = CleanupGuard(Some(tmp_file_cleanup));

    // If command uses {prompt_file}, substitute it and DON'T pipe stdin (avoid double delivery)
    let uses_prompt_file = command_template.contains("{prompt_file}");
    let command = command_template.replace("{prompt_file}", &tmp_file.display().to_string());

    let prompt_for_stdin = if uses_prompt_file {
        None
    } else {
        Some(prompt.to_string())
    };
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .env("TING_PROMPT_FILE", &tmp_file);
    invoke_process(cmd, prompt_for_stdin, timeout, command_template)
}

/// Execute either a shell preset or a direct model command with the same
/// deadline and process-group cleanup. Spawn before starting the waiter so
/// even a zero timeout cannot race with publication of the child PID.
fn invoke_process(
    mut cmd: std::process::Command,
    prompt_for_stdin: Option<String>,
    timeout: Duration,
    description: &str,
) -> Result<String> {
    crate::cancellation::check()?;
    use std::io::{self, Write};
    use std::process::Stdio;

    let started = Instant::now();
    cmd.stdin(if prompt_for_stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Each invocation owns its process group, including ordinary children.
        cmd.process_group(0);
    }

    let mut child = cmd
        .spawn()
        .with_context(|| format!("Failed to execute: {}", description))?;
    let pid = child.id();
    let stdin = child.stdin.take();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        // Drain stdout/stderr concurrently with stdin to avoid pipe deadlocks.
        let stdin_handle = prompt_for_stdin.map(|prompt_data| {
            std::thread::spawn(move || {
                if let Some(mut stdin) = stdin {
                    match stdin.write_all(prompt_data.as_bytes()) {
                        Ok(()) => {}
                        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {}
                        Err(e) => eprintln!("  Warning: stdin write error: {}", e),
                    }
                }
            })
        });
        let output = child.wait_with_output();
        if let Some(handle) = stdin_handle {
            let _ = handle.join();
        }
        tx.send(output).ok();
    });

    let mut interrupted = false;
    let result = loop {
        if crate::cancellation::requested() {
            interrupted = true;
            break Err(mpsc::RecvTimeoutError::Timeout);
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        let poll = if crate::cancellation::current().is_some() {
            remaining.min(Duration::from_millis(50))
        } else {
            remaining
        };
        match rx.recv_timeout(poll) {
            Err(mpsc::RecvTimeoutError::Timeout) if started.elapsed() < timeout => continue,
            result => break result,
        }
    };
    if matches!(result, Err(mpsc::RecvTimeoutError::Timeout)) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/T", "/PID", &pid.to_string()])
                .output();
        }
    }
    // Reap the child and join pipe writers before returning or deleting input.
    let _ = worker.join();
    if interrupted {
        return Err(crate::cancellation::Interrupted.into());
    }
    let output = match result {
        Ok(output) => output.with_context(|| format!("Failed to execute: {}", description))?,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            anyhow::bail!("Command timed out after {:?}: {}", timeout, description);
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            anyhow::bail!("Command worker stopped unexpectedly: {}", description);
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.is_empty() { &stdout } else { &stderr };
        anyhow::bail!("Command failed ({}): {}", description, detail);
    }

    String::from_utf8(output.stdout)
        .with_context(|| "Invalid UTF-8 in command output")
        .map(|s| s.trim().to_string())
}

/// RAII guard that deletes a temp file when dropped (any exit path)
struct CleanupGuard(Option<std::path::PathBuf>);
impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if let Some(ref path) = self.0
            && let Err(e) = fs::remove_file(path)
        {
            eprintln!(
                "  Warning: failed to clean up temp file {}: {}",
                path.display(),
                e
            );
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
        invoke_claude(model, prompt, timeout)
    }
}

/// Invoke Claude directly with literal arguments and the Fire Keeper deadline.
/// On timeout the shared runner terminates its process group and reaps the child.
fn invoke_claude(model: &str, prompt: &str, timeout: Duration) -> Result<String> {
    let mut cmd = std::process::Command::new("claude");
    cmd.arg("--model")
        .arg(model)
        .arg("-p")
        .arg(prompt)
        .arg("--output-format")
        .arg("text");
    invoke_process(cmd, None, timeout, "claude (Fire Keeper)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_interrupts_commands_and_manual_waits() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        for manual in [false, true] {
            let dir = std::env::temp_dir().join(format!("ting-cancel-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            let token = Arc::new(AtomicBool::new(false));
            let request = token.clone();
            let trigger = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                request.store(true, Ordering::SeqCst);
            });
            let start = Instant::now();
            let result = crate::cancellation::with_token(Some(token), || {
                if manual {
                    watch_for_responses(&dir, &["alice".into()], Duration::from_secs(30), |_, _| {
                        Ok(())
                    })
                    .map(|_| ())
                } else {
                    invoke_command("sleep 30 & wait", "prompt", Duration::from_secs(30)).map(|_| ())
                }
            });
            trigger.join().unwrap();
            assert!(crate::cancellation::is_interrupted(&result.unwrap_err()));
            assert!(start.elapsed() < Duration::from_secs(5));
            assert!(crate::cancellation::current().is_none());
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn manual_response_notifies_before_all_participants_finish() {
        let dir =
            std::env::temp_dir().join(format!("ting-test-response-event-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("alice.md"), "Already here").unwrap();
        let mut seen = Vec::new();
        let responses = watch_for_responses(
            &dir,
            &["alice".into(), "bob".into()],
            Duration::from_millis(20),
            |name, response| {
                seen.push((name.to_owned(), response.to_owned()));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(seen, vec![("alice".into(), "Already here".into())]);
        assert_eq!(responses.len(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn direct_process_enforces_deadline_and_kills_descendants() {
        let dir = std::env::temp_dir().join(format!("ting-test-deadline-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("survived");
        let ready = dir.join("ready");
        let release = dir.join("release");
        let mut cmd = std::process::Command::new("sh");
        // A fixed sleep before writing the marker races with a delayed test
        // thread. Instead, the descendant can write only after invoke_process
        // returns and the test explicitly releases it. Redirect its pipes so
        // an escaped descendant cannot prevent the parent from being reaped.
        cmd.args([
            "-c",
            r#"sh -c 'printf ready > "$2"; while [ ! -e "$3" ]; do sleep 0.01; done; printf survived > "$1"' sh "$1" "$2" "$3" </dev/null >/dev/null 2>&1 & wait"#,
            "sh",
        ])
        .arg(&marker)
        .arg(&ready)
        .arg(&release);

        let started = Instant::now();
        let error =
            invoke_process(cmd, None, Duration::from_millis(250), "fake model").unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(error.to_string().contains("fake model"));
        assert!(started.elapsed() < Duration::from_secs(3));
        fs::write(&release, "go").unwrap();
        assert!(
            ready.exists(),
            "descendant did not start; cleanup was not exercised"
        );
        std::thread::sleep(Duration::from_millis(1200));
        assert!(
            !marker.exists(),
            "descendant survived after the runner returned"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn zero_deadline_does_not_wait_for_child_completion() {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "sleep 30"]);
        let started = Instant::now();
        let error = invoke_process(cmd, None, Duration::ZERO, "fake model").unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn direct_process_preserves_arguments_and_failure_details() {
        let mut cmd = std::process::Command::new("printf");
        cmd.arg("%s").arg("literal $(echo unsafe) `text` $HOME");
        let output = invoke_process(cmd, None, Duration::from_secs(5), "fake model").unwrap();
        assert_eq!(output, "literal $(echo unsafe) `text` $HOME");

        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "printf 'model unavailable' >&2; exit 1"]);
        let error = invoke_process(cmd, None, Duration::from_secs(5), "fake model").unwrap_err();
        assert!(error.to_string().contains("model unavailable"));
        assert!(error.to_string().contains("fake model"));
    }

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
        assert!(!is_completed(&dir));
        for name in ["claims.toml", "dissent.md", "meta-summary.toml"] {
            fs::write(final_dir.join(name), "done").unwrap();
        }
        assert!(is_completed(&dir));
        crate::run_status::write(
            &dir,
            crate::run_status::Status::Failed,
            Some("finalization failed".into()),
        )
        .unwrap();
        assert!(!is_completed(&dir));

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

        let participants = vec![
            "alice".to_string(),
            "bob".to_string(),
            "charlie".to_string(),
        ];
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
        assert!(
            err.contains("timed out"),
            "Expected timeout error, got: {}",
            err
        );
    }
}
