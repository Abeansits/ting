use crate::events::{self, EventType};
use crate::{classifier, config, convergence, metric_scoring, substrate, synthesis, types::*};
use anyhow::Result;
use rand::seq::SliceRandom;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;

/// Runtime flags that shape a single `run_forum` invocation. Not persisted to
/// `meta.toml` — resume semantics come from on-disk artifacts.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// When true, run the pre-round classifier before round 1. Controlled by
    /// `--dashboard` minus `--no-classifier` at the CLI layer.
    pub classify: bool,
    /// When true, run per-round metric scoring after each round's content is
    /// written. Requires `classify` — scoring has no metrics to score without
    /// the classifier. Controlled by `--dashboard` minus `--no-metric-scoring`.
    pub score: bool,
    /// When true, append lifecycle events (synthesis / convergence /
    /// forum_complete) to `dashboard-events.jsonl`. Tracks `--dashboard`.
    pub emit_events: bool,
}

/// Run a complete forum deliberation through the modified Delphi protocol.
/// Supports auto-extend: if convergence score < 5 at max_rounds, runs one extra round
/// to avoid premature termination while capping sycophancy from over-deliberation.
pub fn run_forum(forum_config: &ForumConfig, forum_path: &Path, opts: &RunOptions) -> Result<()> {
    let mut prior_rounds: Vec<RoundData> = Vec::new();
    let review_mode = is_review_mode(forum_config);

    // Warn if judge model family overlaps with participants
    warn_judge_overlap(forum_config);

    if opts.emit_events {
        events::emit(
            forum_path,
            &forum_config.forum.id,
            EventType::ForumStarted,
            json!({
                "topic": forum_config.forum.topic,
                "participants": forum_config.participants.names,
                "max_rounds": forum_config.forum.max_rounds,
            }),
        )?;
    }

    let classifier_metrics = if opts.classify {
        Some(run_classifier(forum_config, forum_path)?)
    } else {
        None
    };
    let mut effective_max = forum_config.forum.max_rounds;
    let mut auto_extended = false;
    let mut last_convergence: Option<ConvergenceResult> = None;

    let mut round_num = 0u32;
    loop {
        round_num += 1;
        if round_num > effective_max {
            break;
        }

        let stage = match round_num {
            1 => Stage::Proposal,
            2 => Stage::CrossExam,
            _ => Stage::Revision,
        };

        eprintln!("\n=== Round {} ({}) ===", round_num, stage);

        // Generate and write prompt
        let prompt = generate_prompt(forum_config, round_num, &stage, &prior_rounds)?;
        let round_dir = substrate::create_round_dir(forum_path, round_num)?;
        substrate::write_atomic(&round_dir.join("prompt.md"), &prompt)?;
        eprintln!("  Wrote round-{}/prompt.md", round_num);
        if opts.emit_events {
            events::emit(
                forum_path,
                &forum_config.forum.id,
                EventType::RoundStarted,
                json!({ "round": round_num, "stage": stage.to_string() }),
            )?;
        }

        // Invoke participants and collect responses
        let responses = invoke_participants(forum_config, &prompt, forum_path, round_num, opts.emit_events)?;

        if responses.is_empty() {
            eprintln!("  No responses received. Ending deliberation.");
            break;
        }

        eprintln!(
            "  Collected {}/{} responses",
            responses.len(),
            forum_config.participants.names.len()
        );

        // Generate synthesis
        eprintln!("  Generating synthesis...");
        let prior_synth = prior_rounds.last().and_then(|r| r.synthesis.as_deref());
        let synth = synthesis::generate_synthesis(
            &forum_config.synthesis,
            &forum_config.forum.topic,
            round_num,
            &stage,
            &responses,
            prior_synth,
            review_mode,
        )?;
        substrate::write_atomic(&round_dir.join("synthesis.md"), &synth)?;
        if opts.emit_events {
            events::emit(
                forum_path,
                &forum_config.forum.id,
                EventType::Synthesis,
                json!({ "round": round_num, "word_count": synth.split_whitespace().count() }),
            )?;
        }

        // Generate claims
        eprintln!("  Generating claims...");
        let claims = synthesis::generate_claims(
            &forum_config.synthesis,
            &forum_config.forum.topic,
            &responses,
        )?;
        substrate::write_atomic_toml(&round_dir.join("claims.toml"), &claims)?;

        let round_data = RoundData {
            number: round_num,
            stage,
            responses: responses.clone(),
            synthesis: Some(synth),
            claims: Some(claims),
        };
        prior_rounds.push(round_data);

        // Per-round metric scoring (dashboard substrate). Requires classifier
        // output; the CLI layer guarantees `opts.score` implies `opts.classify`,
        // but guard on the Option anyway so a future caller can't break it.
        //
        // Scoring failures are warn-and-continue: the dashboard is opt-in and
        // the rest of the round's output (synthesis, claims, convergence) is
        // independently valuable. A flaky Fire-Keeper call shouldn't abort
        // work already done. Next resume will retry this round's scoring
        // because the file was never written.
        if let Some(ref metrics_file) = classifier_metrics
            && opts.score
        {
            let last = prior_rounds.last().expect("just pushed");
            if let Err(e) = run_scoring(
                forum_config,
                forum_path,
                metrics_file,
                round_num,
                &last.responses,
                last.synthesis.as_deref(),
            ) {
                eprintln!(
                    "  Warning: metric scoring failed for round {}: {}. \
                     Continuing forum; dashboard will show classifier metrics \
                     without a score for this round.",
                    round_num, e,
                );
            }
        }

        // Score per-participant alignment for position shift tracking (every round)
        if let Some(ref synth) = prior_rounds.last().and_then(|r| r.synthesis.clone()) {
            eprintln!("  Scoring alignment...");
            if let Ok(alignment) =
                convergence::evaluate_alignment(&forum_config.convergence, &synth, &responses)
            {
                let alignment_toml: String = alignment
                    .iter()
                    .map(|(k, v)| format!("{} = {:.1}", k, v))
                    .collect::<Vec<_>>()
                    .join("\n");
                let content = format!("[alignment]\nround = {}\n{}\n", round_num, alignment_toml);
                substrate::write_atomic_toml(&round_dir.join("alignment.toml"), &content)?;
            }
        }

        // Convergence check (only after min_rounds)
        if round_num >= forum_config.convergence.min_rounds {
            eprintln!("  Evaluating convergence...");
            let result = convergence::evaluate(
                &forum_config.convergence,
                &forum_config.forum.topic,
                &responses,
                forum_config.convergence.threshold,
            )?;

            if opts.emit_events {
                events::emit(
                    forum_path,
                    &forum_config.forum.id,
                    EventType::Convergence,
                    json!({ "round": round_num, "score": result.score() }),
                )?;
            }

            match &result {
                ConvergenceResult::Converged { score, summary, .. } => {
                    eprintln!("  CONVERGED (score: {:.1}): {}", score, summary);
                    last_convergence = Some(result);
                    break; // converged — exit loop, write final output below
                }
                ConvergenceResult::Divergent {
                    score,
                    key_disagreements,
                } => {
                    eprintln!("  Divergent (score: {:.1})", score);
                    for d in key_disagreements {
                        eprintln!("    - {}", d);
                    }

                    // Auto-extend: if at max_rounds with very low score, add one more round
                    if round_num == effective_max && *score < 5.0 && !auto_extended {
                        effective_max += 1;
                        auto_extended = true;
                        eprintln!(
                            "  Auto-extending: score {:.1} < 5.0, adding round {}",
                            score, effective_max
                        );
                    }

                    last_convergence = Some(result);
                }
            }
        }
    }

    // Write final output
    let final_result = match last_convergence {
        Some(result) => result,
        None => {
            // No convergence check ran (e.g., max_rounds < min_rounds)
            let last_responses = prior_rounds
                .last()
                .map(|r| r.responses.clone())
                .unwrap_or_default();
            convergence::evaluate(
                &forum_config.convergence,
                &forum_config.forum.topic,
                &last_responses,
                forum_config.convergence.threshold,
            )?
        }
    };

    match &final_result {
        ConvergenceResult::Converged { .. } => {}
        _ => eprintln!("\n=== Max rounds ({}) reached ===", effective_max),
    }

    // Hollow consensus detection: check if claims contradict the convergence score
    let hollow_warning = detect_hollow_consensus(&final_result, &prior_rounds);
    if let Some(ref warning) = hollow_warning {
        eprintln!("  {}", warning);
    }

    write_final_output(
        forum_config,
        forum_path,
        &prior_rounds,
        &final_result,
        hollow_warning.as_deref(),
    )?;

    if opts.emit_events {
        events::emit(
            forum_path,
            &forum_config.forum.id,
            EventType::ForumComplete,
            json!({ "rounds_used": prior_rounds.len() }),
        )?;
    }
    Ok(())
}

fn invoke_participants(
    config: &ForumConfig,
    prompt: &str,
    forum_path: &Path,
    round: u32,
    emit_events: bool,
) -> Result<HashMap<String, String>> {
    let round_dir = forum_path.join(format!("round-{}", round));
    let mut responses = HashMap::new();

    // Save exactly what each participant sees, including aliases sharing a CLI.
    let prompts_dir = round_dir.join("prompts");
    std::fs::create_dir_all(&prompts_dir)?;
    let mut participant_prompts = HashMap::new();
    for name in &config.participants.names {
        let personalized = format!(
            "# Your participant identity\n\n\
             You are participant `{name}` in this forum. Use this exact name for \
             your own prior responses and cross-examination assignment, regardless \
             of your model or CLI name. Speak only for `{name}`.\n\n{prompt}",
        );
        substrate::write_atomic(&prompts_dir.join(format!("{}.md", name)), &personalized)?;
        participant_prompts.insert(name.clone(), personalized);
    }

    // Split participants by type
    let command_participants: Vec<String> = config
        .participants
        .names
        .iter()
        .filter(|n| {
            config
                .participants
                .configs
                .get(*n)
                .is_some_and(|c| c.participant_type == "command")
        })
        .cloned()
        .collect();

    let manual_participants: Vec<String> = config
        .participants
        .names
        .iter()
        .filter(|n| {
            config
                .participants
                .configs
                .get(*n)
                .is_some_and(|c| c.participant_type == "manual")
        })
        .cloned()
        .collect();

    // Parse participant timeout
    let participant_timeout = config::parse_duration(&config.timing.participant_timeout)?;

    // Invoke command participants concurrently, show progress as they complete
    if !command_participants.is_empty() {
        for name in &command_participants {
            eprintln!("  Invoking: {}", name);
        }

        let (tx, rx) = std::sync::mpsc::channel::<(String, Result<String>)>();

        for name in &command_participants {
            let tx = tx.clone();
            let name = name.clone();
            let cmd_template = config.participants.configs[&name].command.clone().unwrap();
            let prompt = participant_prompts[&name].clone();
            let round_dir = round_dir.clone();
            let timeout = participant_timeout;

            std::thread::spawn(move || {
                let result = substrate::invoke_command(&cmd_template, &prompt, timeout)
                    .and_then(|response| {
                        substrate::write_atomic(&round_dir.join(format!("{}.md", name)), &response)?;
                        Ok(response)
                    });
                tx.send((name, result)).ok();
            });
        }
        drop(tx);

        let mut failures: Vec<String> = Vec::new();
        for (name, result) in rx {
            match result {
                Ok(response) => {
                    emit_participant_response(config, forum_path, round, &name, &response, emit_events)?;
                    let words = response.split_whitespace().count();
                    eprintln!("  \u{2713} {} responded ({} words)", name, words);
                    responses.insert(name, response);
                }
                Err(e) => {
                    eprintln!("  \u{2717} {} failed: {}", name, e);
                    failures.push(format!("{}: {}", name, e));
                }
            }
        }
        // A requested participant that errors mid-round is not optional input —
        // silently dropping it would produce a synthesis that misleads the user
        // about whose voice was in the room. Abort so they can investigate and
        // re-run. Manual participant timeouts continue to honor `late_policy`.
        if !failures.is_empty() {
            anyhow::bail!(
                "Aborting round {round}: {n} requested participant(s) failed and would be silently dropped:\n  - {list}",
                n = failures.len(),
                list = failures.join("\n  - "),
            );
        }
    }

    // Wait for manual participants via filesystem watching
    if !manual_participants.is_empty() {
        let forum_id = &config.forum.id;
        let timeout = config::parse_duration(&config.timing.round_timeout)?;

        eprintln!();
        for name in &manual_participants {
            eprintln!("  \u{23f3} Waiting for YOU ({})", name);
            eprintln!("    Read your prompt: {}", prompts_dir.join(format!("{}.md", name)).display());
        }
        eprintln!();
        eprintln!(
            "    Read others' responses:  ting status {} --round {}",
            forum_id, round
        );
        eprintln!("    Write your response:     ting respond {}", forum_id);
        for name in &manual_participants {
            eprintln!(
                "    Or edit directly:        {}/round-{}/{}.md",
                forum_path.display(),
                round,
                name,
            );
        }
        eprintln!();

        let manual_responses = substrate::watch_for_responses(
            &round_dir,
            &manual_participants,
            timeout,
            |name, response| emit_participant_response(config, forum_path, round, name, response, emit_events),
        )?;

        let missing: Vec<&String> = manual_participants
            .iter()
            .filter(|n| !manual_responses.contains_key(*n))
            .collect();

        if !missing.is_empty() {
            for name in &missing {
                eprintln!("  \u{2717} {} timed out", name);
            }
        }

        responses.extend(manual_responses);
    }

    Ok(responses)
}

/// Emit only after the response is available on disk, from the protocol thread.
fn emit_participant_response(
    config: &ForumConfig,
    forum_path: &Path,
    round: u32,
    name: &str,
    response: &str,
    enabled: bool,
) -> Result<()> {
    if enabled {
        events::emit(
            forum_path,
            &config.forum.id,
            EventType::ParticipantResponse,
            json!({ "round": round, "participant": name, "word_count": response.split_whitespace().count() }),
        )?;
    }
    Ok(())
}

fn generate_prompt(
    config: &ForumConfig,
    _round: u32,
    stage: &Stage,
    prior_rounds: &[RoundData],
) -> Result<String> {
    match stage {
        Stage::Proposal => Ok(generate_proposal_prompt(config)),
        Stage::CrossExam => generate_crossexam_prompt(config, prior_rounds),
        Stage::Revision => generate_revision_prompt(config, prior_rounds),
    }
}

fn context_section(config: &ForumConfig) -> String {
    match &config.forum.context {
        Some(ctx) if !ctx.is_empty() => format!("\n## Context\n\n{}\n", ctx),
        _ => String::new(),
    }
}

fn generate_proposal_prompt(config: &ForumConfig) -> String {
    format!(
        "# Forum Topic\n\n\
         {}{}\n\n\
         ## Instructions\n\n\
         You are participating in a structured deliberation. \
         Provide your independent analysis and proposal for the topic above.\n\n\
         Consider:\n\
         - Key factors and tradeoffs\n\
         - Your recommended approach with clear reasoning\n\
         - Potential risks and mitigations\n\
         - Specific evidence or examples supporting your position\n\n\
         Write your response in clear, structured markdown.\n",
        config.forum.topic,
        context_section(config),
    )
}

fn generate_crossexam_prompt(config: &ForumConfig, prior_rounds: &[RoundData]) -> Result<String> {
    let round1 = prior_rounds
        .last()
        .ok_or_else(|| anyhow::anyhow!("No prior round data for cross-examination"))?;

    // Assign cross-exam pairs
    let assignments = assign_cross_exam(&config.participants.names);

    let mut prompt = format!(
        "# Forum Topic\n\n{}{}\n\n## Round 1 Responses\n",
        config.forum.topic,
        context_section(config),
    );

    for name in &config.participants.names {
        if let Some(response) = round1.responses.get(name) {
            prompt.push_str(&format!("\n### {}\n{}\n", name, response));
        }
    }

    if let Some(ref synth) = round1.synthesis {
        prompt.push_str(&format!("\n## Round 1 Synthesis\n{}\n", synth));
    }

    prompt.push_str("\n## Cross-Examination Assignments\n\n");
    for (critic, target) in &assignments {
        prompt.push_str(&format!("- **{}** critiques **{}**\n", critic, target));
    }

    prompt.push_str(
        "\n## Instructions\n\n\
         Use your supplied participant identity to find your assignment above.\n\n\
         1. **Critique**: Examine your assigned participant's position. \
         Find weaknesses, gaps, contradictions, or unstated assumptions.\n\
         2. **Defend/Revise**: Reconsider your own Round 1 position in light of ALL responses. \
         Defend it, revise it, or adopt elements from others.\n\n\
         Structure your response as:\n\
         ### Critique\n\
         ...\n\
         ### Revised Position\n\
         ...\n",
    );

    Ok(prompt)
}

fn generate_revision_prompt(config: &ForumConfig, prior_rounds: &[RoundData]) -> Result<String> {
    let last = prior_rounds
        .last()
        .ok_or_else(|| anyhow::anyhow!("No prior round data for revision"))?;

    let mut prompt = format!(
        "# Forum Topic\n\n{}{}\n\n",
        config.forum.topic,
        context_section(config)
    );

    if let Some(ref synth) = last.synthesis {
        prompt.push_str(&format!("## Previous Round Synthesis\n{}\n\n", synth));
    }

    prompt.push_str("## Previous Round Responses\n");
    for name in &config.participants.names {
        if let Some(response) = last.responses.get(name) {
            prompt.push_str(&format!("\n### {}\n{}\n", name, response));
        }
    }

    prompt.push_str(
        "\n## Instructions\n\n\
         Based on the discussion so far, provide your FINAL revised position.\n\n\
         Consider:\n\
         - Points raised in critiques\n\
         - Areas of agreement you want to reinforce\n\
         - Disagreements you want to address directly\n\
         - Your updated recommendation\n\n\
         Be specific about what you've changed and why, or why you're holding firm.\n\
         Write in clear markdown.\n",
    );

    Ok(prompt)
}

/// Assign cross-examination: shuffle participants, each critiques the next
fn assign_cross_exam(participants: &[String]) -> Vec<(String, String)> {
    let n = participants.len();
    if n < 2 {
        return Vec::new();
    }

    let mut shuffled = participants.to_vec();
    let mut rng = rand::thread_rng();
    shuffled.shuffle(&mut rng);

    shuffled
        .iter()
        .enumerate()
        .map(|(i, critic)| {
            let target = &shuffled[(i + 1) % n];
            (critic.clone(), target.clone())
        })
        .collect()
}

/// Check for hollow consensus: high convergence score but contested claims in claims.toml.
/// Returns a warning string if detected.
fn detect_hollow_consensus(result: &ConvergenceResult, rounds: &[RoundData]) -> Option<String> {
    let score = match result {
        ConvergenceResult::Converged { score, .. } => *score,
        _ => return None, // only check when judge says "converged"
    };

    let claims_text = rounds.last().and_then(|r| r.claims.as_ref())?;

    // Count "oppose" stances in claims — simple text matching on the generated TOML
    let oppose_count = claims_text.matches("oppose").count();
    let total_stances = claims_text.matches("support").count()
        + oppose_count
        + claims_text.matches("neutral").count();

    if total_stances == 0 {
        return None;
    }

    let oppose_ratio = oppose_count as f32 / total_stances as f32;

    // Hollow consensus: score >= 7 but >25% of stances are "oppose"
    if score >= 7.0 && oppose_ratio > 0.25 {
        Some(format!(
            "> **Hollow consensus detected.** Convergence score is {:.1} but \
             {}/{} claim stances are \"oppose\" ({:.0}%). \
             Positions may be superficially agreeing while substantive disagreements remain.",
            score,
            oppose_count,
            total_stances,
            oppose_ratio * 100.0,
        ))
    } else {
        None
    }
}

fn write_final_output(
    config: &ForumConfig,
    forum_path: &Path,
    rounds: &[RoundData],
    convergence_result: &ConvergenceResult,
    hollow_warning: Option<&str>,
) -> Result<()> {
    let final_dir = substrate::create_final_dir(forum_path)?;

    // Final synthesis — use last round's, prepend hollow consensus warning if detected
    if let Some(last) = rounds.last() {
        if let Some(ref synth) = last.synthesis {
            let final_synth = match hollow_warning {
                Some(warning) => format!("{}\n\n{}", warning, synth),
                None => synth.clone(),
            };
            substrate::write_atomic(&final_dir.join("synthesis.md"), &final_synth)?;
        }
        if let Some(ref claims) = last.claims {
            substrate::write_atomic_toml(&final_dir.join("claims.toml"), claims)?;
        }
    }

    // Reaching the stopping threshold does not imply unanimity. Inspect the
    // final positions even when the judge did not list any disagreements.
    let key_disagreements = match convergence_result {
        ConvergenceResult::Converged { key_disagreements, .. }
        | ConvergenceResult::Divergent { key_disagreements, .. } => key_disagreements,
    };
    let last_responses = rounds
        .last()
        .map(|r| &r.responses)
        .cloned()
        .unwrap_or_default();
    let dissent = synthesis::generate_dissent(
        &config.synthesis,
        &config.forum.topic,
        &last_responses,
        key_disagreements,
    )?;
    let dissent = match hollow_warning {
        Some(warning) => format!("{}\n\n{}", warning, dissent),
        None => dissent,
    };
    substrate::write_atomic(&final_dir.join("dissent.md"), &dissent)?;

    // Meta summary
    let (status, score) = match convergence_result {
        ConvergenceResult::Converged { score, .. } => ("converged", *score),
        ConvergenceResult::Divergent { score, .. } => ("divergent", *score),
    };
    let meta_summary = format!(
        "[summary]\n\
         status = \"{}\"\n\
         final_score = {:.1}\n\
         total_rounds = {}\n\
         participants = {}\n",
        status,
        score,
        rounds.len(),
        rounds.last().map_or(0, |r| r.responses.len()),
    );
    substrate::write_atomic_toml(&final_dir.join("meta-summary.toml"), &meta_summary)?;

    eprintln!(
        "\n=== Final output written to {}/final/ ===",
        forum_path.display()
    );

    Ok(())
}

/// Extract model family from a model ID or preset name (e.g., "claude-opus-4-6" → "claude")
fn model_family(id: &str) -> &str {
    if id.starts_with("claude") {
        return "claude";
    }
    if id.starts_with("gpt") || id.contains("codex") {
        return "openai";
    }
    if id.starts_with("gemini") {
        return "gemini";
    }
    if id.starts_with("kimi") || id.contains("opencode") {
        return "moonshot";
    }
    if id.starts_with("llama") || id.starts_with("deepseek") {
        return "meta/open";
    }
    if id.starts_with("glm") {
        return "zhipu";
    }
    id.split('-').next().unwrap_or(id)
}

/// Warn if the convergence judge uses the same model family as a participant
fn warn_judge_overlap(config: &ForumConfig) {
    let judge_id = if config.convergence.judge_command.is_some() {
        // Custom command — try to resolve from the model field
        config::resolve_model_id(&config.convergence.judge_model)
    } else {
        crate::config::resolve_model(&config.convergence.judge_model).to_string()
    };
    let judge_fam = model_family(&judge_id);

    for name in &config.participants.names {
        let participant_id = config::resolve_model_id(name);
        let participant_fam = model_family(&participant_id);
        if judge_fam == participant_fam {
            eprintln!(
                "  Warning: convergence judge ({}) is same model family as participant {} ({}). \
                 Consider using a different judge to avoid self-evaluation bias.",
                judge_id, name, participant_id,
            );
            return; // one warning is enough
        }
    }
}

/// Drive the pre-round classifier: pick topic-specific metrics plus the
/// mandatory dissent axis, write `round-0/metrics.json`, emit a
/// `classifier_metrics` event. Skips cleanly on resume if `metrics.json`
/// already exists. Returns the metrics file so downstream scoring can reuse
/// the definitions without re-reading disk.
fn run_classifier(
    forum_config: &ForumConfig,
    forum_path: &Path,
) -> Result<classifier::ClassifierMetricsFile> {
    let synth = &forum_config.synthesis;
    let model = config::resolve_model(&synth.model).to_string();
    let custom_command = synth.command.clone();

    let invoke = |prompt: &str| -> Result<String> {
        substrate::invoke_fire_keeper_model(
            custom_command.as_deref(),
            &model,
            prompt,
            synthesis::FIRE_KEEPER_TIMEOUT,
        )
    };

    eprintln!("  Running pre-round classifier (Fire Keeper)...");
    let (file, outcome) = classifier::ensure_classifier(
        forum_path,
        &forum_config.forum.id,
        &forum_config.forum.topic,
        forum_config.forum.context.as_deref(),
        &model,
        invoke,
    )?;

    let verb = match outcome {
        classifier::ClassifierOutcome::Fresh => "picked",
        classifier::ClassifierOutcome::Resumed => "reused",
    };
    eprintln!(
        "  \u{2713} Classifier {} {} metrics (incl. dissent axis)",
        verb,
        file.metrics.len(),
    );
    Ok(file)
}

/// Score the just-completed round's metrics (Fire Keeper, one batched call).
/// Writes `round-N/metric-scores.json`, emits a `metric_scores` event, and
/// short-circuits on resume if the file already exists.
fn run_scoring(
    forum_config: &ForumConfig,
    forum_path: &Path,
    metrics_file: &classifier::ClassifierMetricsFile,
    round: u32,
    responses: &HashMap<String, String>,
    synthesis_text: Option<&str>,
) -> Result<()> {
    let synth = &forum_config.synthesis;
    let model = config::resolve_model(&synth.model).to_string();
    let custom_command = synth.command.clone();

    let invoke = |prompt: &str| -> Result<String> {
        substrate::invoke_fire_keeper_model(
            custom_command.as_deref(),
            &model,
            prompt,
            synthesis::FIRE_KEEPER_TIMEOUT,
        )
    };

    eprintln!("  Scoring metrics for round {}...", round);
    let (_, outcome) = metric_scoring::ensure_scores(
        forum_path,
        &forum_config.forum.id,
        &forum_config.forum.topic,
        round,
        metrics_file,
        responses,
        synthesis_text,
        &model,
        invoke,
    )?;

    let verb = match outcome {
        metric_scoring::ScoringOutcome::Fresh => "scored",
        metric_scoring::ScoringOutcome::Resumed => "reused",
    };
    eprintln!("  \u{2713} Metrics {} for round {}", verb, round);
    Ok(())
}

/// Detect review mode from output_format field or topic keywords
fn is_review_mode(config: &ForumConfig) -> bool {
    if let Some(ref fmt) = config.forum.output_format {
        if fmt == "review" {
            return true;
        }
    }
    let topic_lower = config.forum.topic.to_lowercase();
    topic_lower.contains("code review")
        || topic_lower.contains("review this")
        || topic_lower.contains("review the")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forum_emits_complete_lifecycle_only_when_enabled() {
        for enabled in [false, true] {
            let dir = std::env::temp_dir().join(format!("ting-test-lifecycle-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let mut config = make_test_config("A live topic");
            config.forum.max_rounds = 1;
            config.convergence.min_rounds = 1;
            config.convergence.judge_command = Some("printf 'SCORE: 8\nSUMMARY: Agreement\nDISAGREEMENTS:\nALIGNMENT: alice=8 bob=8\n'".into());
            config.synthesis.command = Some("printf 'A synthesis'".into());
            for participant in config.participants.configs.values_mut() {
                participant.participant_type = "command".into();
                participant.command = Some("printf 'A response'".into());
            }
            run_forum(&config, &dir, &RunOptions { emit_events: enabled, ..RunOptions::default() }).unwrap();
            if enabled {
                let log = std::fs::read_to_string(events::event_log_path(&dir)).unwrap();
                let events: Vec<events::DashboardEvent> = log.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
                let schema: serde_json::Value = serde_json::from_str(include_str!("../schemas/dashboard-event.schema.json")).unwrap();
                let validator = jsonschema::validator_for(&schema).unwrap();
                for (index, event) in events.iter().enumerate() {
                    assert_eq!(event.seq, index as u64 + 1);
                    assert!(validator.is_valid(&serde_json::to_value(event).unwrap()));
                }
                assert_eq!(events.iter().map(|event| event.event_type).collect::<Vec<_>>(), vec![
                    EventType::ForumStarted, EventType::RoundStarted,
                    EventType::ParticipantResponse, EventType::ParticipantResponse,
                    EventType::Synthesis, EventType::Convergence, EventType::ForumComplete,
                ]);
                assert_eq!(events[0].payload["topic"], "A live topic");
                assert_eq!(events[0].payload["participants"], json!(["alice", "bob"]));
                assert_eq!(events[1].payload["stage"], "proposal");
                assert_eq!(events[2].payload["word_count"], 2);
            } else {
                assert!(!events::event_log_path(&dir).exists());
            }
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn participant_aliases_receive_distinct_persisted_identities() {
        let dir = std::env::temp_dir().join(format!("ting-test-identities-{}", uuid::Uuid::new_v4()));
        let round_dir = substrate::create_round_dir(&dir, 2).unwrap();
        let mut config = make_test_config("Rollout strategy?");
        config.participants.names = vec!["optimist".into(), "skeptic".into(), "human".into()];
        config.participants.configs = HashMap::from([
            ("optimist".into(), ParticipantConfig { participant_type: "command".into(), command: Some("cat".into()) }),
            ("skeptic".into(), ParticipantConfig { participant_type: "command".into(), command: Some("cat".into()) }),
            ("human".into(), ParticipantConfig { participant_type: "manual".into(), command: None }),
        ]);
        config.timing.participant_timeout = "5s".into();
        std::fs::write(round_dir.join("human.md"), "Human response").unwrap();
        let prior = vec![RoundData {
            number: 1,
            stage: Stage::Proposal,
            responses: HashMap::from([
                ("optimist".into(), "Ship now".into()),
                ("skeptic".into(), "Test first".into()),
                ("human".into(), "Stage the rollout".into()),
            ]),
            synthesis: None,
            claims: None,
        }];
        let shared_prompt = generate_crossexam_prompt(&config, &prior).unwrap();
        let responses = invoke_participants(&config, &shared_prompt, &dir, 2, true).unwrap();
        for name in &config.participants.names {
            let saved = std::fs::read_to_string(round_dir.join("prompts").join(format!("{}.md", name))).unwrap();
            assert!(saved.starts_with(&format!("# Your participant identity\n\nYou are participant `{}`", name)));
            assert!(saved.contains(&format!("- **{}** critiques **", name)));
            assert!(saved.ends_with(&shared_prompt));
            if name != "human" {
                assert_eq!(responses[name], saved.trim());
            }
        }
        assert_ne!(responses["optimist"], responses["skeptic"]);
        assert_eq!(responses["human"], "Human response");
        let log = std::fs::read_to_string(events::event_log_path(&dir)).unwrap();
        let events: Vec<events::DashboardEvent> = log.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(events.len(), 3);
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event.seq, index as u64 + 1);
            assert_eq!(event.event_type, EventType::ParticipantResponse);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn final_output_preserves_dissent_independently_of_convergence() {
        // Echo the dissent prompt to verify the real model boundary without
        // paid calls: both the judge's objections and final positions must
        // reach the dissent pass, including when the judge lists no objections.
        for (score, objections, warning) in [
            (8.0, vec!["Rollout remains disputed".to_string()], None),
            (8.0, Vec::new(), None),
            (8.0, Vec::new(), Some("> **Hollow consensus detected.** Contested claims remain.")),
            (4.0, vec!["Rollout remains disputed".to_string()], None),
        ] {
            let dir = std::env::temp_dir().join(format!("ting-test-dissent-{}", uuid::Uuid::new_v4()));
            let mut config = make_test_config("Rollout strategy?");
            config.synthesis.command = Some("cat".into());
            let result = if score >= 7.0 {
                ConvergenceResult::Converged {
                    score,
                    summary: "Enough agreement to stop".into(),
                    key_disagreements: objections.clone(),
                }
            } else {
                ConvergenceResult::Divergent { score, key_disagreements: objections.clone() }
            };
            let rounds = vec![RoundData {
                number: 2,
                stage: Stage::CrossExam,
                responses: HashMap::from([
                    ("alice".into(), "Ship now".into()),
                    ("bob".into(), "Wait for the accessibility audit".into()),
                ]),
                synthesis: Some("The majority favors shipping".into()),
                claims: None,
            }];

            write_final_output(&config, &dir, &rounds, &result, warning).unwrap();
            let dissent = std::fs::read_to_string(dir.join("final/dissent.md")).unwrap();
            assert!(dissent.contains("Wait for the accessibility audit"));
            assert!(dissent.contains("Ship now"));
            assert!(dissent.contains("Do not infer unanimity"));
            for objection in objections {
                assert!(dissent.contains(&objection));
            }
            assert!(!dissent.contains("No unresolved disagreements — forum reached consensus"));
            if let Some(warning) = warning {
                assert!(dissent.starts_with(warning));
            }
            let summary = std::fs::read_to_string(dir.join("final/meta-summary.toml")).unwrap();
            let expected_status = if score >= 7.0 { "converged" } else { "divergent" };
            assert!(summary.contains(&format!("status = \"{}\"", expected_status)));
            // A sentence about one settled issue must not hide other dissent.
            std::fs::write(
                dir.join("final/dissent.md"),
                "No unresolved disagreements on language choice.\n\nBob still requests an accessibility audit.",
            ).unwrap();
            let html = crate::report::generate_html_report(&config, &dir).unwrap();
            assert!(html.contains("Bob still requests an accessibility audit."));
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn test_assign_cross_exam_all_assigned() {
        let participants = vec![
            "alice".to_string(),
            "bob".to_string(),
            "charlie".to_string(),
        ];
        let assignments = assign_cross_exam(&participants);
        assert_eq!(assignments.len(), 3);

        // Every participant appears exactly once as critic
        let mut critics: Vec<&str> = assignments.iter().map(|(c, _)| c.as_str()).collect();
        critics.sort();
        critics.dedup();
        assert_eq!(critics.len(), 3);

        // No one critiques themselves
        for (critic, target) in &assignments {
            assert_ne!(critic, target);
        }
    }

    #[test]
    fn test_assign_cross_exam_two_participants() {
        let participants = vec!["alice".to_string(), "bob".to_string()];
        let assignments = assign_cross_exam(&participants);
        assert_eq!(assignments.len(), 2);
        for (critic, target) in &assignments {
            assert_ne!(critic, target);
        }
    }

    #[test]
    fn test_assign_cross_exam_single_participant() {
        let participants = vec!["solo".to_string()];
        let assignments = assign_cross_exam(&participants);
        assert!(assignments.is_empty());
    }

    #[test]
    fn test_generate_proposal_prompt() {
        let config = make_test_config("Should we use Rust?");
        let prompt = generate_proposal_prompt(&config);
        assert!(prompt.contains("Should we use Rust?"));
        assert!(prompt.contains("Instructions"));
        assert!(prompt.contains("independent analysis"));
    }

    #[test]
    fn test_generate_crossexam_prompt() {
        let config = make_test_config("Rust vs Go?");
        let mut responses = HashMap::new();
        responses.insert("alice".to_string(), "Rust is great".to_string());
        responses.insert("bob".to_string(), "Go is simpler".to_string());

        let prior = vec![RoundData {
            number: 1,
            stage: Stage::Proposal,
            responses,
            synthesis: Some("Both have merits".to_string()),
            claims: None,
        }];

        let prompt = generate_crossexam_prompt(&config, &prior).unwrap();
        assert!(prompt.contains("Rust vs Go?"));
        assert!(prompt.contains("Rust is great"));
        assert!(prompt.contains("Go is simpler"));
        assert!(prompt.contains("Cross-Examination Assignments"));
        assert!(prompt.contains("Critique"));
    }

    #[test]
    fn test_invoke_participants_fails_loud_when_command_fails() {
        // A requested command participant that errors must abort the round
        // rather than silently dropping the participant from the synthesis.
        let dir = std::env::temp_dir().join(format!(
            "ting-test-fail-loud-{}",
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("round-0")).unwrap();

        let mut config = make_test_config("topic");
        config.participants.names = vec!["broken".to_string()];
        config.participants.configs = HashMap::from([(
            "broken".into(),
            ParticipantConfig {
                participant_type: "command".into(),
                command: Some("false".into()),
            },
        )]);
        config.timing.participant_timeout = "5s".into();

        let result = invoke_participants(&config, "test prompt", &dir, 0, false);
        assert!(result.is_err(), "expected aggregation guard to abort");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("broken"),
            "error should name the failed participant, got: {err}"
        );
        assert!(
            err.contains("Aborting"),
            "error should announce the abort, got: {err}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_generate_revision_prompt() {
        let config = make_test_config("Topic?");
        let mut responses = HashMap::new();
        responses.insert("alice".to_string(), "Revised position".to_string());

        let prior = vec![RoundData {
            number: 2,
            stage: Stage::CrossExam,
            responses,
            synthesis: Some("Synthesis from round 2".to_string()),
            claims: None,
        }];

        let prompt = generate_revision_prompt(&config, &prior).unwrap();
        assert!(prompt.contains("Topic?"));
        assert!(prompt.contains("Synthesis from round 2"));
        assert!(prompt.contains("FINAL revised position"));
    }

    fn make_test_config(topic: &str) -> ForumConfig {
        ForumConfig {
            forum: ForumSection {
                id: "test".into(),
                topic: topic.into(),
                created: "2026-03-27".into(),
                max_rounds: 3,
                protocol: "delphi-crossexam".into(),
                context: None,
                output_format: None,
            },
            participants: ParticipantsSection {
                names: vec!["alice".into(), "bob".into()],
                configs: HashMap::from([
                    (
                        "alice".into(),
                        ParticipantConfig {
                            participant_type: "manual".into(),
                            command: None,
                        },
                    ),
                    (
                        "bob".into(),
                        ParticipantConfig {
                            participant_type: "manual".into(),
                            command: None,
                        },
                    ),
                ]),
            },
            timing: TimingSection::default(),
            convergence: ConvergenceSection::default(),
            synthesis: SynthesisSection::default(),
        }
    }
}
