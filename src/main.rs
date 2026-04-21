mod classifier;
mod config;
mod convergence;
mod dashboard_state;
mod eval;
mod events;
mod metric_scoring;
mod protocol;
mod report;
mod server;
mod substrate;
mod synthesis;
mod types;

use crate::types::*;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ting", version, about = "Multi-agent deliberation tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create and start a new deliberation forum
    New {
        /// The topic or question for deliberation
        topic: String,

        /// Participants: preset name (codex, gemini, claude, opencode, human) or name:command:"cmd"
        #[arg(short, long, required = true)]
        participant: Vec<String>,

        /// Round timeout (e.g. "5m", "30s", "1h")
        #[arg(short, long, default_value = "5m")]
        timeout: String,

        /// Maximum number of rounds (default 2; auto-extends if score < 5)
        #[arg(long, default_value_t = 2)]
        max_rounds: u32,

        /// Supplementary context (file path or inline text) included in every round prompt
        #[arg(short, long)]
        context: Option<String>,

        /// Output format: "review" produces a prioritized findings list instead of narrative
        #[arg(long)]
        output_format: Option<String>,

        /// Enable the v0.4 live dashboard: runs the pre-round classifier,
        /// emits events to dashboard-events.jsonl, and starts a localhost
        /// HTTP server (see --port) exposing the forum snapshot at
        /// /api/state. Live streaming UI arrives in a later phase.
        #[arg(long)]
        dashboard: bool,

        /// Skip the pre-round classifier (only meaningful with --dashboard).
        /// Useful when you want lifecycle events without the extra LLM call.
        #[arg(long)]
        no_classifier: bool,

        /// Skip the per-round metric scoring pass (only meaningful with
        /// --dashboard). Dashboard then shows static metric labels with no
        /// animated values — roughly 50% fewer Fire Keeper calls.
        #[arg(long)]
        no_metric_scoring: bool,

        /// Port for the live dashboard HTTP server (only meaningful with
        /// --dashboard). Binds 127.0.0.1 only. Default 3420.
        #[arg(long, default_value_t = 3420)]
        port: u16,

        /// Suppress the auto-open of the dashboard URL in a browser
        /// (only meaningful with --dashboard).
        #[arg(long)]
        no_open: bool,
    },

    /// Serve the live dashboard for a forum directory without running the
    /// forum itself. Works for in-progress and completed forums.
    Serve {
        /// Forum ID
        forum_id: String,

        /// Port for the dashboard HTTP server. Binds 127.0.0.1 only.
        #[arg(long, default_value_t = 3420)]
        port: u16,

        /// Suppress the auto-open of the dashboard URL in a browser.
        #[arg(long)]
        no_open: bool,
    },

    /// Check the status of a forum
    Status {
        /// Forum ID
        forum_id: String,

        /// Show detailed view of a specific round (responses, synthesis)
        #[arg(short, long)]
        round: Option<u32>,
    },

    /// List all forums
    List,

    /// Show the final result of a completed forum
    Result {
        /// Forum ID
        forum_id: String,

        /// Generate an HTML report to final/report.html
        #[arg(long)]
        html: bool,

        /// Publish the HTML report via here.now (requires --html)
        #[arg(long, requires = "html")]
        publish: bool,
    },

    /// Submit a response (for human participants). Auto-detects round and name if omitted.
    Respond {
        /// Forum ID
        forum_id: String,

        /// Round number (auto-detected if omitted)
        #[arg(short, long)]
        round: Option<u32>,

        /// Participant name (auto-detected if omitted)
        #[arg(short = 'n', long)]
        participant: Option<String>,

        /// Path to response file (opens $EDITOR if omitted)
        #[arg(short, long)]
        file: Option<PathBuf>,
    },

    /// Manage participant presets
    Preset {
        #[command(subcommand)]
        action: PresetAction,
    },

    /// Evaluate: compare single-model baseline vs. Ting forum (blind judged)
    Eval {
        /// The question or topic to evaluate
        topic: String,

        /// Baseline model preset (single-model response)
        #[arg(short, long)]
        baseline: String,

        /// Forum participants (comma-separated presets)
        #[arg(short, long, value_delimiter = ',')]
        forum: Vec<String>,

        /// Judge model preset (default: auto-select one not in forum)
        #[arg(short, long)]
        judge: Option<String>,

        /// Supplementary context (file path or inline text)
        #[arg(short, long)]
        context: Option<String>,

        /// Timeout per participant (default 10m for thorough eval)
        #[arg(short, long, default_value = "10m")]
        timeout: String,

        /// Max rounds for the forum
        #[arg(long, default_value_t = 3)]
        max_rounds: u32,

        /// Generate HTML report
        #[arg(long)]
        html: bool,

        /// Eval ID to generate HTML for (instead of running a new eval)
        #[arg(long)]
        report: Option<String>,
    },
}

#[derive(Subcommand)]
enum PresetAction {
    /// Add or update a custom preset
    Add {
        /// Preset name (e.g. "llama", "mistral")
        name: String,
        /// Command template (use {prompt_file} or stdin)
        command: String,
    },
    /// List all available presets (built-in + custom)
    List,
    /// Remove a custom preset
    Remove {
        /// Preset name to remove
        name: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::New {
            topic,
            participant,
            timeout,
            max_rounds,
            context,
            output_format,
            dashboard,
            no_classifier,
            no_metric_scoring,
            port,
            no_open,
        } => cmd_new(
            &topic,
            &participant,
            &timeout,
            max_rounds,
            context.as_deref(),
            output_format.as_deref(),
            dashboard,
            no_classifier,
            no_metric_scoring,
            port,
            no_open,
        ),
        Commands::Serve {
            forum_id,
            port,
            no_open,
        } => cmd_serve(&forum_id, port, no_open),
        Commands::Status { forum_id, round } => cmd_status(&forum_id, round),
        Commands::List => cmd_list(),
        Commands::Result {
            forum_id,
            html,
            publish,
        } => cmd_result(&forum_id, html, publish),
        Commands::Respond {
            forum_id,
            round,
            participant,
            file,
        } => cmd_respond(&forum_id, round, participant.as_deref(), file.as_ref()),
        Commands::Eval {
            topic,
            baseline,
            forum,
            judge,
            context,
            timeout,
            max_rounds,
            html,
            report,
        } => cmd_eval(
            &topic,
            &baseline,
            &forum,
            judge.as_deref(),
            context.as_deref(),
            &timeout,
            max_rounds,
            html,
            report.as_deref(),
        ),
        Commands::Preset { action } => match action {
            PresetAction::Add { name, command } => cmd_preset_add(&name, &command),
            PresetAction::List => cmd_preset_list(),
            PresetAction::Remove { name } => cmd_preset_remove(&name),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_new(
    topic: &str,
    participants: &[String],
    timeout: &str,
    max_rounds: u32,
    context: Option<&str>,
    output_format: Option<&str>,
    dashboard: bool,
    no_classifier: bool,
    no_metric_scoring: bool,
    port: u16,
    no_open: bool,
) -> Result<()> {
    // Validate timeout format early
    config::parse_duration(timeout)?;

    // Parse participant specs
    let mut names = Vec::new();
    let mut configs: HashMap<String, ParticipantConfig> = HashMap::new();

    for spec in participants {
        let (name, pc) = config::parse_participant_spec(spec)?;
        names.push(name.clone());
        configs.insert(name, pc);
    }

    // Resolve context: if it's a file path that exists, read it; otherwise treat as inline text
    let context_text = match context {
        Some(c) => {
            let path = std::path::Path::new(c);
            if path.exists() {
                eprintln!("  Reading context from: {}", path.display());
                let content = std::fs::read_to_string(path)
                    .with_context(|| format!("Failed to read context file: {}", path.display()))?;
                Some(content)
            } else {
                Some(c.to_string())
            }
        }
        None => None,
    };

    // Generate forum ID: ting-YYYY-MM-DD-UUID8 (collision-safe)
    let id = format!(
        "ting-{}-{}",
        chrono::Utc::now().format("%Y-%m-%d"),
        &uuid::Uuid::new_v4().to_string()[..8],
    );

    let forum_config = ForumConfig {
        forum: ForumSection {
            id: id.clone(),
            topic: topic.to_string(),
            created: chrono::Utc::now().to_rfc3339(),
            max_rounds,
            protocol: "delphi-crossexam".to_string(),
            context: context_text,
            output_format: output_format.map(|s| s.to_string()),
        },
        participants: ParticipantsSection { names, configs },
        timing: TimingSection {
            round_timeout: timeout.to_string(),
            participant_timeout: timeout.to_string(),
            quorum: 0,
            late_policy: "include_next".to_string(),
        },
        convergence: ConvergenceSection::default(),
        synthesis: SynthesisSection::default(),
    };

    // Validate before creating anything on disk
    config::validate(&forum_config)?;

    // Create forum directory and save config
    let forum_path = substrate::create_forum_dir(&id)?;
    config::save(&forum_config, &forum_path.join("meta.toml"))?;

    // Append [models] section with resolved model IDs
    {
        let meta_path = forum_path.join("meta.toml");
        let mut meta = std::fs::read_to_string(&meta_path)?;
        meta.push_str("\n[models]\n");
        meta.push_str(&format!(
            "synthesis = \"{}\"\n",
            config::resolve_model(&forum_config.synthesis.model)
        ));
        meta.push_str(&format!(
            "convergence_judge = \"{}\"\n",
            config::resolve_model(&forum_config.convergence.judge_model)
        ));
        for name in &forum_config.participants.names {
            meta.push_str(&format!(
                "{} = \"{}\"\n",
                name,
                config::resolve_model_id(name)
            ));
        }
        std::fs::write(&meta_path, meta)?;
    }

    print_banner();
    eprintln!();
    eprintln!("  Forum  {}", id);
    eprintln!("  Topic  {}", topic);
    eprintln!("  With   {}", forum_config.participants.names.join(", "));
    eprintln!("  Rules  {} rounds, {} timeout", max_rounds, timeout);
    eprintln!();

    // Dashboard artifacts: classifier + scoring gate on their own opt-outs;
    // lifecycle event emission tracks `--dashboard` unconditionally so the
    // Convergence / Latest Synthesis widgets still populate under
    // `--no-classifier`. Without `--dashboard`, behavior matches v0.3.
    let classify = dashboard && !no_classifier;
    let run_opts = protocol::RunOptions {
        classify,
        score: classify && !no_metric_scoring,
        emit_events: dashboard,
    };

    if dashboard {
        run_with_dashboard(forum_config, forum_path, run_opts, port, no_open)
    } else {
        protocol::run_forum(&forum_config, &forum_path, &run_opts)
    }
}

/// Run a forum with the live dashboard server co-scheduled.
///
/// Order matters for structured concurrency: we bind the listener *before*
/// spawning the blocking `run_forum` work. If the bind fails we exit without
/// ever starting a forum we can't cleanly cancel. Once both are running we
/// always join the forum task before returning, so a mid-flight server error
/// can't orphan it.
fn run_with_dashboard(
    forum_config: ForumConfig,
    forum_path: PathBuf,
    run_opts: protocol::RunOptions,
    port: u16,
    no_open: bool,
) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to build tokio runtime")?;

    let server_path = forum_path.clone();

    rt.block_on(async move {
        let listener = start_dashboard(port, no_open).await?;

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let forum_task = tokio::task::spawn_blocking(move || {
            let result = protocol::run_forum(&forum_config, &forum_path, &run_opts);
            let _ = shutdown_tx.send(());
            result
        });

        let serve_result = server::serve(listener, server_path, async move {
            let _ = shutdown_rx.await;
        })
        .await;

        let forum_result = forum_task
            .await
            .map_err(|e| anyhow::anyhow!("forum task panicked: {e}"))
            .and_then(|r| r);

        // Forum is the primary work — its error wins. A server error is only
        // returned on its own; if both fail, log the server failure and
        // surface the forum error.
        match (forum_result, serve_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(e)) => Err(e),
            (Err(e), Ok(())) => Err(e),
            (Err(forum_err), Err(serve_err)) => {
                eprintln!("  Dashboard server error: {serve_err:#}");
                Err(forum_err)
            }
        }
    })
}

/// Serve the dashboard for an existing forum without running it. Used to
/// inspect completed forums or attach to an already-running one. Ctrl+C
/// triggers a graceful shutdown so in-flight SSE clients get a clean close.
fn cmd_serve(forum_id: &str, port: u16, no_open: bool) -> Result<()> {
    let forum_path = substrate::forum_dir(forum_id);
    if !forum_path.exists() {
        anyhow::bail!("Forum not found: {}", forum_id);
    }

    let status = if substrate::is_completed(&forum_path) {
        "completed"
    } else {
        "in progress"
    };
    eprintln!();
    eprintln!("  Forum   {}", forum_id);
    eprintln!("  Status  {}", status);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to build tokio runtime")?;

    rt.block_on(async move {
        let listener = start_dashboard(port, no_open).await?;
        eprintln!("  Press Ctrl+C to stop");

        // Graceful shutdown on first Ctrl+C; on second Ctrl+C or a 5s stall
        // (long-lived SSE clients that hold axum's graceful-shutdown open)
        // force exit. Tokio doesn't restore the default SIGINT handler after
        // the first install, so the fallback is the correctness guarantee.
        server::serve(listener, forum_path, async {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("\n  Shutting down — Ctrl+C again to force.");
            tokio::spawn(async {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
                }
                std::process::exit(130);
            });
        })
        .await
    })
}

/// Bind the dashboard listener on a loopback port, print the URL, and
/// optionally launch the browser. Split from `server::serve` so callers stay
/// in charge of the shutdown future.
async fn start_dashboard(port: u16, no_open: bool) -> Result<tokio::net::TcpListener> {
    let listener = server::bind_loopback(port).await?;
    let bound = listener
        .local_addr()
        .context("Failed to read bound address")?;
    let url = format!("http://{bound}");
    eprintln!("  Dashboard  {url}");
    if !no_open && std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        try_open_browser(&url);
    }
    Ok(listener)
}

/// Spawn the platform's "open this URL" helper. Failure is silent — the URL
/// was already printed, so the user can click or copy. The child is reaped in
/// a detached thread to avoid leaving a zombie for the server's lifetime.
fn try_open_browser(url: &str) {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else if cfg!(target_os = "windows") {
        // `start` is a shell built-in, not an exe. Route through cmd.
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    if let Ok(mut child) = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

fn cmd_status(forum_id: &str, round: Option<u32>) -> Result<()> {
    let forum_path = substrate::forum_dir(forum_id);
    if !forum_path.exists() {
        anyhow::bail!("Forum not found: {}", forum_id);
    }

    let cfg = config::load(&forum_path.join("meta.toml"))?;
    let current = substrate::current_round(&forum_path);
    let completed = substrate::is_completed(&forum_path);

    // Detailed view of a specific round
    if let Some(r) = round {
        return cmd_status_round(forum_id, &forum_path, &cfg, r);
    }

    // Overview with round-by-round progress
    println!("Forum:  {}", forum_id);
    println!("Topic:  {}", cfg.forum.topic);
    println!(
        "Status: {}",
        if completed {
            "completed".to_string()
        } else {
            format!(
                "in progress (round {} of {})",
                current, cfg.forum.max_rounds
            )
        }
    );
    println!();

    for r in 1..=current {
        let stage = match r {
            1 => "proposal",
            2 => "cross-examination",
            _ => "revision",
        };
        let is_current = r == current && !completed;
        let arrow = if is_current { "  \u{2190} current" } else { "" };

        println!("  Round {} ({}){}", r, stage, arrow);

        let round_dir = forum_path.join(format!("round-{}", r));
        let mut line = String::from("    ");
        for name in &cfg.participants.names {
            let responded = round_dir.join(format!("{}.md", name)).exists();
            if responded {
                line.push_str(&format!("\u{2713} {}  ", name));
            } else {
                line.push_str(&format!("\u{23f3} {}  ", name));
            }
        }
        println!("{}", line.trim_end());

        let has_synthesis = round_dir.join("synthesis.md").exists();
        if has_synthesis {
            println!("    Synthesis: \u{2713}");
        }
        println!();
    }

    if completed {
        println!("  Final output: {}/final/", forum_path.display());
    }

    Ok(())
}

fn cmd_status_round(
    forum_id: &str,
    forum_path: &std::path::Path,
    cfg: &ForumConfig,
    round: u32,
) -> Result<()> {
    let round_dir = forum_path.join(format!("round-{}", round));
    if !round_dir.exists() {
        anyhow::bail!("Round {} does not exist for forum {}", round, forum_id);
    }

    let stage = match round {
        1 => "proposal",
        2 => "cross-examination",
        _ => "revision",
    };

    println!("Forum:  {}", forum_id);
    println!("Round:  {} ({})", round, stage);
    println!();

    // Show prompt (first 15 lines)
    let prompt_path = round_dir.join("prompt.md");
    if prompt_path.exists() {
        println!("--- Prompt ---");
        let content = substrate::read_file(&prompt_path)?;
        let lines: Vec<&str> = content.lines().collect();
        let show = lines.len().min(15);
        for line in &lines[..show] {
            println!("  {}", line);
        }
        if lines.len() > 15 {
            println!("  ... ({} more lines)", lines.len() - 15);
        }
        println!();
    }

    // Show each participant's response
    for name in &cfg.participants.names {
        let path = round_dir.join(format!("{}.md", name));
        if path.exists() {
            let content = substrate::read_file(&path)?;
            let words = content.split_whitespace().count();
            println!("--- {} ({} words) ---", name, words);
            println!("{}", content);
            println!();
        } else {
            println!("--- {} ---", name);
            println!("  (no response yet)");
            println!();
        }
    }

    // Show synthesis if available
    let synth_path = round_dir.join("synthesis.md");
    if synth_path.exists() {
        println!("--- Synthesis ---");
        println!("{}", substrate::read_file(&synth_path)?);
    }

    Ok(())
}

fn cmd_list() -> Result<()> {
    let forums = substrate::list_forums()?;

    if forums.is_empty() {
        println!("No forums found.");
        return Ok(());
    }

    println!("{:<32} {:<10} {}", "ID", "Status", "Topic");
    println!("{}", "-".repeat(72));

    for (id, path) in &forums {
        let completed = substrate::is_completed(path);
        let status = if completed { "done" } else { "active" };

        let topic = config::load(&path.join("meta.toml"))
            .map(|c| c.forum.topic)
            .unwrap_or_else(|_| "<error>".into());

        let topic_display = if topic.len() > 35 {
            format!("{}...", &topic[..32])
        } else {
            topic
        };

        println!("{:<32} {:<10} {}", id, status, topic_display);
    }

    Ok(())
}

fn cmd_result(forum_id: &str, html: bool, publish: bool) -> Result<()> {
    let forum_path = substrate::forum_dir(forum_id);
    let final_dir = forum_path.join("final");

    if !final_dir.exists() {
        anyhow::bail!(
            "Forum '{}' has not completed yet. Run: ting status {}",
            forum_id,
            forum_id
        );
    }

    if html {
        let cfg = config::load(&forum_path.join("meta.toml"))?;
        let report_path = final_dir.join("report.html");
        let html_content = report::generate_html_report(&cfg, &forum_path)?;
        std::fs::write(&report_path, &html_content)
            .with_context(|| "Failed to write report.html")?;
        eprintln!("Report written to: {}", report_path.display());

        if publish {
            eprintln!("Publishing via here.now...");
            let output = std::process::Command::new("herenow")
                .arg("publish")
                .arg(&report_path)
                .output()
                .with_context(|| "Failed to run 'herenow publish'. Is here.now installed?")?;
            if output.status.success() {
                let url = String::from_utf8_lossy(&output.stdout);
                println!("{}", url.trim());
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("herenow publish failed: {}", stderr);
            }
        }
        return Ok(());
    }

    // Default: print to terminal
    let synthesis_path = final_dir.join("synthesis.md");
    if synthesis_path.exists() {
        println!("{}", substrate::read_file(&synthesis_path)?);
    }

    let dissent_path = final_dir.join("dissent.md");
    if dissent_path.exists() {
        let content = substrate::read_file(&dissent_path)?;
        if !content.contains("No unresolved disagreements") {
            println!("\n---\n\n{}", content);
        }
    }

    let meta_path = final_dir.join("meta-summary.toml");
    if meta_path.exists() {
        eprintln!("\n--- Meta ---");
        eprintln!("{}", substrate::read_file(&meta_path)?);
    }

    Ok(())
}

fn cmd_respond(
    forum_id: &str,
    round: Option<u32>,
    participant: Option<&str>,
    file: Option<&PathBuf>,
) -> Result<()> {
    let forum_path = substrate::forum_dir(forum_id);
    if !forum_path.exists() {
        anyhow::bail!("Forum not found: {}", forum_id);
    }

    let cfg = config::load(&forum_path.join("meta.toml"))?;

    // Auto-detect round: latest round directory that exists
    let round = match round {
        Some(r) => r,
        None => {
            let current = substrate::current_round(&forum_path);
            if current == 0 {
                anyhow::bail!("No active round found for forum {}", forum_id);
            }
            current
        }
    };

    let round_dir = forum_path.join(format!("round-{}", round));
    if !round_dir.exists() {
        anyhow::bail!("Round {} does not exist for forum {}", round, forum_id);
    }

    // Auto-detect participant: find manual participants without a response in this round
    let participant_name = match participant {
        Some(p) => p.to_string(),
        None => {
            let pending: Vec<&String> = cfg
                .participants
                .names
                .iter()
                .filter(|n| {
                    cfg.participants
                        .configs
                        .get(*n)
                        .is_some_and(|c| c.participant_type == "manual")
                })
                .filter(|n| !round_dir.join(format!("{}.md", n)).exists())
                .collect();

            match pending.len() {
                0 => anyhow::bail!(
                    "All manual participants have already responded in round {}",
                    round
                ),
                1 => pending[0].clone(),
                _ => anyhow::bail!(
                    "Multiple manual participants pending: {}. Use --participant to specify.",
                    pending
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
    };

    let response_path = round_dir.join(format!("{}.md", participant_name));

    match file {
        Some(f) => {
            // File provided: copy content to response path
            let content = std::fs::read_to_string(f)
                .with_context(|| format!("Failed to read response file: {}", f.display()))?;
            substrate::write_atomic(&response_path, &content)?;
            let words = content.split_whitespace().count();
            eprintln!(
                "\u{2713} Response submitted: {} \u{2192} round-{}/{}.md ({} words)",
                participant_name, round, participant_name, words
            );
        }
        None => {
            // No file: open $EDITOR with a draft, then atomic-write to response path
            let editor = find_editor();

            let draft_path =
                std::env::temp_dir().join(format!("ting-respond-{}.md", uuid::Uuid::new_v4()));

            // Seed draft with existing content if user is re-editing
            if response_path.exists() {
                std::fs::copy(&response_path, &draft_path)?;
            } else {
                std::fs::write(&draft_path, "")?;
            }

            eprintln!(
                "Opening {} with {}... (round {}, participant: {})",
                draft_path.file_name().unwrap_or_default().to_string_lossy(),
                editor,
                round,
                participant_name,
            );
            eprintln!("Save and quit when done.");

            let status = std::process::Command::new(&editor)
                .arg(&draft_path)
                .status()
                .with_context(|| format!("Failed to open editor: {}", editor))?;

            if !status.success() {
                let _ = std::fs::remove_file(&draft_path);
                anyhow::bail!("Editor exited with error");
            }

            let content = std::fs::read_to_string(&draft_path)?;
            let _ = std::fs::remove_file(&draft_path);

            if content.trim().is_empty() {
                anyhow::bail!("Response is empty \u{2014} not submitting.");
            }

            substrate::write_atomic(&response_path, &content)?;
            let words = content.split_whitespace().count();
            eprintln!(
                "\u{2713} Response submitted: {} \u{2192} round-{}/{}.md ({} words)",
                participant_name, round, participant_name, words
            );
        }
    }

    Ok(())
}

/// Find the user's preferred editor, falling back through common options
fn find_editor() -> String {
    if let Ok(editor) = std::env::var("EDITOR") {
        return editor;
    }
    if let Ok(editor) = std::env::var("VISUAL") {
        return editor;
    }
    for candidate in &["nano", "vim", "vi"] {
        if std::process::Command::new("which")
            .arg(candidate)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            return candidate.to_string();
        }
    }
    "nano".to_string()
}

fn print_banner() {
    let is_tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let (dim, reset, accent) = if is_tty {
        ("\x1b[2m", "\x1b[0m", "\x1b[38;5;75m")
    } else {
        ("", "", "")
    };
    eprint!("{}", accent);
    eprintln!(r"");
    eprintln!(r"  ████████╗██╗███╗   ██╗ ██████╗ ");
    eprintln!(r"  ╚══██╔══╝██║████╗  ██║██╔════╝ ");
    eprintln!(r"     ██║   ██║██╔██╗ ██║██║  ███╗");
    eprintln!(r"     ██║   ██║██║╚██╗██║██║   ██║");
    eprintln!(r"     ██║   ██║██║ ╚████║╚██████╔╝");
    eprintln!(r"     ╚═╝   ╚═╝╚═╝  ╚═══╝ ╚═════╝ ");
    eprint!("{}", reset);
    eprintln!(
        "  {}v{}  Structured deliberation between AI models{}",
        dim,
        env!("CARGO_PKG_VERSION"),
        reset,
    );
}

#[allow(clippy::too_many_arguments)]
fn cmd_eval(
    topic: &str,
    baseline: &str,
    forum: &[String],
    judge: Option<&str>,
    context: Option<&str>,
    timeout: &str,
    max_rounds: u32,
    html: bool,
    report: Option<&str>,
) -> Result<()> {
    // HTML report for existing eval
    if let Some(eval_id) = report {
        let eval_dir = eval::evals_dir().join(eval_id);
        if !eval_dir.exists() {
            anyhow::bail!("Eval not found: {}", eval_id);
        }
        let html_content = eval::generate_eval_html(&eval_dir)?;
        let report_path = eval_dir.join("report.html");
        std::fs::write(&report_path, &html_content)?;
        eprintln!("Report: {}", report_path.display());
        return Ok(());
    }

    config::parse_duration(timeout)?;

    // Auto-select judge: pick a model not in the forum
    let judge_preset = match judge {
        Some(j) => j.to_string(),
        None => {
            let candidates = ["claude", "gemini", "codex", "opencode"];
            candidates
                .iter()
                .find(|c| !forum.contains(&c.to_string()) && **c != baseline)
                .map(|c| c.to_string())
                .unwrap_or_else(|| "claude".to_string())
        }
    };

    // Resolve context
    let context_text =
        match context {
            Some(c) => {
                let path = std::path::Path::new(c);
                if path.exists() {
                    Some(std::fs::read_to_string(path).with_context(|| {
                        format!("Failed to read context file: {}", path.display())
                    })?)
                } else {
                    Some(c.to_string())
                }
            }
            None => None,
        };

    print_banner();
    eprintln!();
    eprintln!("  Mode     EVAL (baseline vs. forum)");
    eprintln!("  Topic    {}", topic);
    eprintln!("  Baseline {}", baseline);
    eprintln!("  Forum    {}", forum.join(", "));
    eprintln!("  Judge    {}", judge_preset);
    eprintln!();

    let cfg = eval::EvalConfig {
        topic: topic.to_string(),
        context: context_text,
        baseline_preset: baseline.to_string(),
        forum_presets: forum.to_vec(),
        judge_preset,
        timeout: timeout.to_string(),
        max_rounds,
    };

    let result = eval::run_eval(&cfg)?;

    if html {
        let html_content = eval::generate_eval_html(&result.eval_dir)?;
        let report_path = result.eval_dir.join("report.html");
        std::fs::write(&report_path, &html_content)?;
        eprintln!("HTML report: {}", report_path.display());
    }

    Ok(())
}

fn cmd_preset_add(name: &str, command: &str) -> Result<()> {
    config::save_user_preset(name, command)?;
    eprintln!("Preset '{}' saved: {}", name, command);
    Ok(())
}

fn cmd_preset_list() -> Result<()> {
    let presets = config::list_all_presets();
    println!("{:<14} {:<6} {}", "Name", "Type", "Command");
    println!("{}", "-".repeat(70));
    for (name, cmd, is_custom) in &presets {
        let tag = if *is_custom { "custom" } else { "built-in" };
        let cmd_display = if cmd.len() > 45 {
            format!("{}...", &cmd[..42])
        } else {
            cmd.clone()
        };
        println!("{:<14} {:<9} {}", name, tag, cmd_display);
    }
    println!(
        "{:<14} {:<9} {}",
        "human", "built-in", "(manual — writes files directly)"
    );
    Ok(())
}

fn cmd_preset_remove(name: &str) -> Result<()> {
    let path = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .join(".ting")
        .join("config.toml");
    if !path.exists() {
        anyhow::bail!("No custom presets configured");
    }
    let content = std::fs::read_to_string(&path)?;
    let mut table: toml::Table = content.parse().unwrap_or_default();
    if let Some(toml::Value::Table(p)) = table.get_mut("presets") {
        if p.remove(name).is_none() {
            anyhow::bail!("Preset '{}' not found in custom presets", name);
        }
    } else {
        anyhow::bail!("No custom presets configured");
    }
    let output = toml::to_string_pretty(&table)?;
    std::fs::write(&path, output)?;
    eprintln!("Preset '{}' removed", name);
    Ok(())
}
