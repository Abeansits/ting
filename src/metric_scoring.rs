//! Per-round metric scoring.
//!
//! After each round's responses + synthesis are written, ask the Fire Keeper to
//! score every classifier metric on its own scale. One LLM call per round
//! (batched over all metrics). The result lands on disk as
//! `round-N/metric-scores.json` and as a `metric_scores` event on
//! `dashboard-events.jsonl`.
//!
//! Resume semantics: if `round-N/metric-scores.json` already exists we
//! short-circuit — no LLM call, no duplicate event. If the file exists but the
//! matching event is missing (crash between write and append), the event is
//! backfilled on resume. Same shape as `classifier::ensure_classifier`.
//!
//! `--no-metric-scoring` bypasses this module at the CLI layer; when on, the
//! dashboard sees static metric definitions with no animated values.

use crate::classifier::{ClassifierMetric, ClassifierMetricsFile};
use crate::events::{self, DashboardEvent, EventType};
use crate::substrate;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Snapshot schema version written to `metric-scores.json`. Bump on breaking
/// changes to the top-level file shape; additive score fields do not bump.
pub const SCORES_VERSION: u32 = 1;

pub const SCORES_FILENAME: &str = "metric-scores.json";

/// A single metric's score for one round. `rationale` is optional — the LLM
/// usually supplies one, but consumers (dashboard, TUI) must not require it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricScore {
    pub metric_id: String,
    pub score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

/// On-disk envelope for `round-N/metric-scores.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricScoresFile {
    pub version: u32,
    pub forum_id: String,
    pub round: u32,
    pub scored_at: DateTime<Utc>,
    pub model: String,
    pub scores: Vec<MetricScore>,
}

/// Shape the LLM emits — `{ "scores": [...] }`. Kept separate from the on-disk
/// envelope so `parse_scoring_response` can deserialize straight into it.
#[derive(Debug, Deserialize)]
struct ScoringResponse {
    scores: Vec<MetricScore>,
}

pub fn scores_path(forum_dir: &Path, round: u32) -> PathBuf {
    substrate::round_dir(forum_dir, round).join(SCORES_FILENAME)
}

/// Build the scoring prompt. Lists every metric with its scale and asks for a
/// single JSON object back — fractional scores allowed (e.g. 7.5) because the
/// dashboard renders them as gauges rather than integer bars.
pub fn build_scoring_prompt(
    topic: &str,
    round: u32,
    metrics: &[ClassifierMetric],
    responses: &HashMap<String, String>,
    synthesis: Option<&str>,
) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You are the Fire Keeper scoring a completed round of a structured \
         multi-agent deliberation. Score each metric on its own scale based on \
         the round's participant responses and synthesis.\n\n",
    );
    prompt.push_str("## Forum Topic\n\n");
    prompt.push_str(topic.trim());
    prompt.push_str(&format!("\n\n## Round {}\n\n", round));

    let mut names: Vec<&String> = responses.keys().collect();
    names.sort();
    for name in &names {
        prompt.push_str(&format!("### {}\n{}\n\n", name, responses[*name]));
    }

    if let Some(synth) = synthesis {
        prompt.push_str("## Synthesis\n\n");
        prompt.push_str(synth);
        prompt.push_str("\n\n");
    }

    prompt.push_str("## Metrics\n\n");
    for m in metrics {
        let desc = m.description.as_deref().unwrap_or("");
        prompt.push_str(&format!(
            "- `{}` ({}): scale 1..={}. {}\n",
            m.id, m.name, m.scale, desc,
        ));
    }

    prompt.push_str(
        "\n## Output Format\n\n\
         Output ONLY a JSON object with a single `scores` array. No prose, no \
         markdown fences, no trailing commentary. Score every metric listed \
         above, using its specific scale. Scores may be fractional (e.g. 7.5) \
         and must satisfy 1.0 <= score <= scale. Shape:\n\n\
         {\n  \"scores\": [\n    { \"metric_id\": \"…\", \"score\": 7.5, \"rationale\": \"one sentence\" },\n    …\n  ]\n}\n",
    );
    prompt
}

/// Parse a scoring response into a validated vector. Tolerates code fences the
/// same way `classifier::parse_classifier_response` does. Enforces: every
/// classifier metric id is scored exactly once, no scores reference unknown
/// ids, each score is finite and within its metric's scale.
pub fn parse_scoring_response(raw: &str, metrics: &[ClassifierMetric]) -> Result<Vec<MetricScore>> {
    let cleaned = strip_code_fences(raw.trim());
    let response: ScoringResponse = serde_json::from_str(cleaned)
        .with_context(|| "Scoring response was not valid JSON with a `scores` array")?;
    validate_scores(&response.scores, metrics)?;
    Ok(response.scores)
}

fn strip_code_fences(s: &str) -> &str {
    let trimmed = s.trim();
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    body.trim().strip_suffix("```").unwrap_or(body).trim()
}

fn validate_scores(scores: &[MetricScore], metrics: &[ClassifierMetric]) -> Result<()> {
    let expected: HashMap<&str, u32> = metrics.iter().map(|m| (m.id.as_str(), m.scale)).collect();

    let mut seen: HashSet<&str> = HashSet::with_capacity(scores.len());
    for s in scores {
        let scale = *expected.get(s.metric_id.as_str()).ok_or_else(|| {
            anyhow::anyhow!("Score references unknown metric id `{}`", s.metric_id)
        })?;
        if !seen.insert(&s.metric_id) {
            bail!("Duplicate score for metric id `{}`", s.metric_id);
        }
        if !s.score.is_finite() {
            bail!("Metric `{}` has non-finite score", s.metric_id);
        }
        if s.score < 1.0 || s.score > scale as f32 {
            bail!(
                "Metric `{}` score {} out of range 1..={}",
                s.metric_id,
                s.score,
                scale,
            );
        }
    }

    let missing: Vec<&str> = metrics
        .iter()
        .map(|m| m.id.as_str())
        .filter(|id| !seen.contains(id))
        .collect();
    if !missing.is_empty() {
        bail!("Scoring output missing metric ids: {}", missing.join(", "));
    }
    Ok(())
}

/// Atomic write of `metric-scores.json` under `round-N/`. Same
/// `.tmp` + rename + fsync pattern as `classifier::write_metrics`.
pub fn write_scores(forum_dir: &Path, file: &MetricScoresFile) -> Result<()> {
    let dir = substrate::create_round_dir(forum_dir, file.round)?;
    let final_path = dir.join(SCORES_FILENAME);
    let tmp_path = final_path.with_extension("json.tmp");

    let mut body =
        serde_json::to_vec_pretty(file).with_context(|| "Failed to serialize MetricScoresFile")?;
    body.push(b'\n');

    {
        let mut tmp = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp_path)
            .with_context(|| format!("Failed to open temp scores file: {}", tmp_path.display()))?;
        tmp.write_all(&body)
            .with_context(|| format!("Failed to write temp scores file: {}", tmp_path.display()))?;
        tmp.sync_all()
            .with_context(|| format!("Failed to fsync temp scores file: {}", tmp_path.display()))?;
    }

    fs::rename(&tmp_path, &final_path).with_context(|| {
        format!(
            "Failed to rename {} -> {}",
            tmp_path.display(),
            final_path.display()
        )
    })?;

    if let Ok(dir_file) = File::open(&dir) {
        let _ = dir_file.sync_all();
    }
    Ok(())
}

pub fn read_scores(forum_dir: &Path, round: u32) -> Result<Option<MetricScoresFile>> {
    let path = scores_path(forum_dir, round);
    if !path.exists() {
        return Ok(None);
    }
    let body = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read metric-scores.json: {}", path.display()))?;
    let file: MetricScoresFile = serde_json::from_str(&body)
        .with_context(|| format!("Failed to parse metric-scores.json: {}", path.display()))?;
    Ok(Some(file))
}

fn scores_payload(round: u32, scores: &[MetricScore]) -> Value {
    serde_json::json!({ "round": round, "scores": scores })
}

fn emit_scores_event(forum_dir: &Path, file: &MetricScoresFile) -> Result<()> {
    let seq = events::next_seq(forum_dir)?;
    let payload = scores_payload(file.round, &file.scores);
    let event = DashboardEvent::new(seq, file.forum_id.clone(), EventType::MetricScores, payload);
    events::append_event(forum_dir, &event)
}

/// True if a `metric_scores` event for the given round already exists in the
/// log. Scoring happens once per round, so event presence is keyed on `round`
/// inside the payload — not just event type.
fn log_contains_round_scores(forum_dir: &Path, round: u32) -> Result<bool> {
    let path = events::event_log_path(forum_dir);
    if !path.exists() {
        return Ok(false);
    }
    let body = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read event log: {}", path.display()))?;
    for line in body.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(evt) = serde_json::from_str::<DashboardEvent>(line) else {
            continue;
        };
        if evt.event_type != EventType::MetricScores {
            continue;
        }
        if evt.payload.get("round").and_then(|v| v.as_u64()) == Some(round as u64) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether `ensure_scores` ran the LLM (`Fresh`) or reused a prior round's
/// on-disk `metric-scores.json` (`Resumed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoringOutcome {
    Fresh,
    Resumed,
}

/// Score one round's metrics, writing the result to disk and emitting the
/// matching event. See module docs for resume semantics. The closure is
/// injected so tests can stub the LLM.
#[allow(clippy::too_many_arguments)]
pub fn ensure_scores<F>(
    forum_dir: &Path,
    forum_id: &str,
    topic: &str,
    round: u32,
    classifier_metrics: &ClassifierMetricsFile,
    responses: &HashMap<String, String>,
    synthesis: Option<&str>,
    model: &str,
    invoke: F,
) -> Result<(MetricScoresFile, ScoringOutcome)>
where
    F: FnOnce(&str) -> Result<String>,
{
    if let Some(existing) = read_scores(forum_dir, round)? {
        validate_scores(&existing.scores, &classifier_metrics.metrics).with_context(|| {
            format!(
                "Existing round-{}/metric-scores.json failed validation",
                round
            )
        })?;
        if !log_contains_round_scores(forum_dir, round)? {
            emit_scores_event(forum_dir, &existing)?;
        }
        return Ok((existing, ScoringOutcome::Resumed));
    }

    let prompt = build_scoring_prompt(
        topic,
        round,
        &classifier_metrics.metrics,
        responses,
        synthesis,
    );
    let raw =
        invoke(&prompt).with_context(|| format!("Scoring LLM call failed for round {}", round))?;
    let scores = parse_scoring_response(&raw, &classifier_metrics.metrics)?;

    let file = MetricScoresFile {
        version: SCORES_VERSION,
        forum_id: forum_id.to_string(),
        round,
        scored_at: Utc::now(),
        model: model.to_string(),
        scores,
    };

    write_scores(forum_dir, &file)?;
    emit_scores_event(forum_dir, &file)?;
    Ok((file, ScoringOutcome::Fresh))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classifier::{self, METRICS_VERSION};
    use serde_json::json;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("ting-scoring-{}-{}-{}", label, pid, n));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_metrics() -> ClassifierMetricsFile {
        let raw = json!({
            "metrics": [
                { "id": "feasibility", "name": "Technical Feasibility", "scale": 10,
                  "description": "How realistic the proposed approach is." },
                { "id": "cost", "name": "Cost Impact", "scale": 5,
                  "description": "Expected resource cost." },
                { "id": "risk", "name": "Risk Level", "scale": 10,
                  "description": "Likelihood of negative outcomes." },
                { "id": "speed", "name": "Time to Ship", "scale": 10,
                  "description": "How fast this can reach users." },
                { "id": "user_value", "name": "User Value", "scale": 10,
                  "description": "How much end users benefit." },
                { "id": "dissent_axis", "name": "Dissent", "scale": 10,
                  "description": "Unresolved disagreement.", "mandatory": true }
            ]
        })
        .to_string();
        let metrics = classifier::parse_classifier_response(&raw).unwrap();
        ClassifierMetricsFile {
            version: METRICS_VERSION,
            forum_id: "ting-2026-04-19-abcd1234".to_string(),
            generated_at: Utc::now(),
            model: "claude-opus-4-6".to_string(),
            metrics,
        }
    }

    fn sample_llm_response() -> String {
        json!({
            "scores": [
                { "metric_id": "feasibility", "score": 7.5, "rationale": "Plan looks solid." },
                { "metric_id": "cost", "score": 3.0, "rationale": "Moderate." },
                { "metric_id": "risk", "score": 4.5, "rationale": "Few unknowns." },
                { "metric_id": "speed", "score": 6.0, "rationale": "Doable in a quarter." },
                { "metric_id": "user_value", "score": 8.0, "rationale": "Clear win." },
                { "metric_id": "dissent_axis", "score": 4.2, "rationale": "Some holdouts." }
            ]
        })
        .to_string()
    }

    fn sample_responses() -> HashMap<String, String> {
        let mut r = HashMap::new();
        r.insert("codex".to_string(), "Proposal body codex.".to_string());
        r.insert("gemini".to_string(), "Proposal body gemini.".to_string());
        r
    }

    // ---- prompt tests ------------------------------------------------------

    #[test]
    fn prompt_includes_topic_round_metrics_and_responses() {
        let metrics = sample_metrics();
        let responses = sample_responses();
        let prompt = build_scoring_prompt(
            "Is the sky falling?",
            2,
            &metrics.metrics,
            &responses,
            Some("Synthesis text."),
        );
        assert!(prompt.contains("Is the sky falling?"));
        assert!(prompt.contains("Round 2"));
        assert!(prompt.contains("feasibility"));
        assert!(prompt.contains("dissent_axis"));
        assert!(prompt.contains("codex"));
        assert!(prompt.contains("gemini"));
        assert!(prompt.contains("Synthesis text."));
        assert!(prompt.contains("1..=10"));
    }

    #[test]
    fn prompt_omits_synthesis_section_when_none() {
        let metrics = sample_metrics();
        let prompt = build_scoring_prompt("Topic", 1, &metrics.metrics, &sample_responses(), None);
        assert!(!prompt.contains("## Synthesis"));
    }

    // ---- parse tests -------------------------------------------------------

    #[test]
    fn parse_well_formed_response() {
        let metrics = sample_metrics();
        let scores = parse_scoring_response(&sample_llm_response(), &metrics.metrics).unwrap();
        assert_eq!(scores.len(), 6);
        assert!(scores.iter().any(|s| s.metric_id == "dissent_axis"));
    }

    #[test]
    fn parse_tolerates_code_fences() {
        let metrics = sample_metrics();
        let wrapped = format!("```json\n{}\n```", sample_llm_response());
        let scores = parse_scoring_response(&wrapped, &metrics.metrics).unwrap();
        assert_eq!(scores.len(), 6);
    }

    #[test]
    fn parse_rejects_missing_metric_id() {
        let metrics = sample_metrics();
        // drop dissent_axis
        let bad = json!({
            "scores": [
                { "metric_id": "feasibility", "score": 7.5 },
                { "metric_id": "cost", "score": 3.0 },
                { "metric_id": "risk", "score": 4.5 },
                { "metric_id": "speed", "score": 6.0 },
                { "metric_id": "user_value", "score": 8.0 }
            ]
        })
        .to_string();
        let err = parse_scoring_response(&bad, &metrics.metrics)
            .unwrap_err()
            .to_string();
        assert!(err.contains("missing"), "got: {err}");
        assert!(err.contains("dissent_axis"), "got: {err}");
    }

    #[test]
    fn parse_rejects_unknown_metric_id() {
        let metrics = sample_metrics();
        let bad = json!({
            "scores": [
                { "metric_id": "feasibility", "score": 7.5 },
                { "metric_id": "cost", "score": 3.0 },
                { "metric_id": "risk", "score": 4.5 },
                { "metric_id": "speed", "score": 6.0 },
                { "metric_id": "user_value", "score": 8.0 },
                { "metric_id": "dissent_axis", "score": 4.2 },
                { "metric_id": "invented_axis", "score": 5.0 }
            ]
        })
        .to_string();
        let err = parse_scoring_response(&bad, &metrics.metrics)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown"), "got: {err}");
    }

    #[test]
    fn parse_rejects_score_out_of_range() {
        let metrics = sample_metrics();
        // cost has scale 5; 9.0 is over
        let bad = json!({
            "scores": [
                { "metric_id": "feasibility", "score": 7.5 },
                { "metric_id": "cost", "score": 9.0 },
                { "metric_id": "risk", "score": 4.5 },
                { "metric_id": "speed", "score": 6.0 },
                { "metric_id": "user_value", "score": 8.0 },
                { "metric_id": "dissent_axis", "score": 4.2 }
            ]
        })
        .to_string();
        let err = parse_scoring_response(&bad, &metrics.metrics)
            .unwrap_err()
            .to_string();
        assert!(err.contains("out of range"), "got: {err}");
    }

    #[test]
    fn parse_rejects_duplicate_scores() {
        let metrics = sample_metrics();
        let bad = json!({
            "scores": [
                { "metric_id": "feasibility", "score": 7.5 },
                { "metric_id": "feasibility", "score": 4.0 },
                { "metric_id": "cost", "score": 3.0 },
                { "metric_id": "risk", "score": 4.5 },
                { "metric_id": "speed", "score": 6.0 },
                { "metric_id": "user_value", "score": 8.0 },
                { "metric_id": "dissent_axis", "score": 4.2 }
            ]
        })
        .to_string();
        let err = parse_scoring_response(&bad, &metrics.metrics)
            .unwrap_err()
            .to_string();
        assert!(err.contains("Duplicate"), "got: {err}");
    }

    #[test]
    fn parse_rejects_non_finite_score() {
        let metrics = sample_metrics();
        // Build a JSON literal with NaN? JSON doesn't allow NaN; use a bound-hack
        // via a very large number — the scale check catches it, and a separate
        // test verifies non-finite rejection at the validate layer.
        let scores = vec![MetricScore {
            metric_id: "feasibility".into(),
            score: f32::NAN,
            rationale: None,
        }];
        let err = validate_scores(&scores, &metrics.metrics)
            .unwrap_err()
            .to_string();
        assert!(err.contains("non-finite"), "got: {err}");
    }

    #[test]
    fn parse_rejects_invalid_json() {
        let metrics = sample_metrics();
        let err = parse_scoring_response("not json", &metrics.metrics)
            .unwrap_err()
            .to_string();
        assert!(err.contains("valid JSON"), "got: {err}");
    }

    // ---- write/read round trip --------------------------------------------

    fn sample_file(round: u32) -> MetricScoresFile {
        let metrics = sample_metrics();
        let scores = parse_scoring_response(&sample_llm_response(), &metrics.metrics).unwrap();
        MetricScoresFile {
            version: SCORES_VERSION,
            forum_id: "ting-2026-04-19-abcd1234".to_string(),
            round,
            scored_at: Utc::now(),
            model: "claude-opus-4-6".to_string(),
            scores,
        }
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = tmp_dir("round-trip");
        let file = sample_file(1);
        write_scores(&dir, &file).unwrap();
        let back = read_scores(&dir, 1).unwrap().unwrap();
        assert_eq!(back, file);
    }

    #[test]
    fn read_missing_returns_none() {
        let dir = tmp_dir("read-missing");
        assert!(read_scores(&dir, 1).unwrap().is_none());
    }

    #[test]
    fn write_cleans_up_tmp() {
        let dir = tmp_dir("cleanup");
        write_scores(&dir, &sample_file(1)).unwrap();
        let tmp = scores_path(&dir, 1).with_extension("json.tmp");
        assert!(!tmp.exists());
        assert!(scores_path(&dir, 1).exists());
    }

    // ---- schema golden ----------------------------------------------------

    fn load_event_validator() -> jsonschema::Validator {
        let schema_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("schemas")
            .join("dashboard-event.schema.json");
        let body = fs::read_to_string(&schema_path).unwrap();
        let schema: Value = serde_json::from_str(&body).unwrap();
        jsonschema::validator_for(&schema).unwrap()
    }

    #[test]
    fn event_payload_validates_against_phase_1a_schema() {
        let validator = load_event_validator();
        let file = sample_file(2);
        let payload = scores_payload(file.round, &file.scores);
        let event = DashboardEvent::new(1, &file.forum_id, EventType::MetricScores, payload);
        let value = serde_json::to_value(&event).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&value)
            .map(|e| e.to_string())
            .collect();
        assert!(
            errors.is_empty(),
            "event failed schema validation: {:#?}\nvalue: {}",
            errors,
            serde_json::to_string_pretty(&value).unwrap()
        );
    }

    // ---- integration: ensure_scores ---------------------------------------

    #[test]
    fn ensure_fresh_invokes_llm_writes_file_and_event() {
        let dir = tmp_dir("ensure-fresh");
        let metrics = sample_metrics();
        let called = Cell::new(0usize);
        let invoke = |prompt: &str| -> Result<String> {
            called.set(called.get() + 1);
            assert!(prompt.contains("Is the sky falling?"));
            assert!(prompt.contains("dissent_axis"));
            assert!(prompt.contains("Round 2"));
            Ok(sample_llm_response())
        };

        let (file, outcome) = ensure_scores(
            &dir,
            "ting-2026-04-19-abcd1234",
            "Is the sky falling?",
            2,
            &metrics,
            &sample_responses(),
            Some("Synthesis text."),
            "claude-opus-4-6",
            invoke,
        )
        .unwrap();

        assert_eq!(outcome, ScoringOutcome::Fresh);
        assert_eq!(called.get(), 1);
        assert_eq!(file.round, 2);
        assert_eq!(file.scores.len(), 6);
        assert!(scores_path(&dir, 2).exists());

        let log = fs::read_to_string(events::event_log_path(&dir)).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 1);
        let event: DashboardEvent = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(event.event_type, EventType::MetricScores);
        assert_eq!(event.payload["round"], 2);
    }

    #[test]
    fn ensure_resume_is_idempotent_when_event_already_logged() {
        let dir = tmp_dir("ensure-resume-idempotent");
        let metrics = sample_metrics();
        let seeded = sample_file(1);
        write_scores(&dir, &seeded).unwrap();
        emit_scores_event(&dir, &seeded).unwrap();

        let invoke = |_: &str| -> Result<String> {
            panic!("Scoring should not invoke LLM on resume");
        };
        let (file, outcome) = ensure_scores(
            &dir,
            &seeded.forum_id,
            "topic",
            1,
            &metrics,
            &sample_responses(),
            None,
            "claude-opus-4-6",
            invoke,
        )
        .unwrap();

        assert_eq!(outcome, ScoringOutcome::Resumed);
        assert_eq!(file, seeded);

        let log = fs::read_to_string(events::event_log_path(&dir)).unwrap();
        assert_eq!(log.lines().count(), 1);
    }

    #[test]
    fn ensure_resume_backfills_missing_event() {
        let dir = tmp_dir("ensure-resume-backfill");
        let metrics = sample_metrics();
        let seeded = sample_file(1);
        write_scores(&dir, &seeded).unwrap();
        assert!(!events::event_log_path(&dir).exists());

        let invoke = |_: &str| -> Result<String> {
            panic!("Scoring should not invoke LLM on resume");
        };
        let (_, outcome) = ensure_scores(
            &dir,
            &seeded.forum_id,
            "topic",
            1,
            &metrics,
            &sample_responses(),
            None,
            "claude-opus-4-6",
            invoke,
        )
        .unwrap();

        assert_eq!(outcome, ScoringOutcome::Resumed);
        let log = fs::read_to_string(events::event_log_path(&dir)).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 1);
        let event: DashboardEvent = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(event.event_type, EventType::MetricScores);
        assert_eq!(event.payload["round"], 1);
    }

    #[test]
    fn ensure_per_round_independence() {
        // Round 1 already scored and logged; Round 2 is fresh. The round-1
        // event presence must not cause the round-2 path to short-circuit.
        let dir = tmp_dir("per-round-independence");
        let metrics = sample_metrics();
        let r1 = sample_file(1);
        write_scores(&dir, &r1).unwrap();
        emit_scores_event(&dir, &r1).unwrap();

        let invoke = |_: &str| -> Result<String> { Ok(sample_llm_response()) };
        let (r2_file, outcome) = ensure_scores(
            &dir,
            &r1.forum_id,
            "topic",
            2,
            &metrics,
            &sample_responses(),
            None,
            "claude-opus-4-6",
            invoke,
        )
        .unwrap();

        assert_eq!(outcome, ScoringOutcome::Fresh);
        assert_eq!(r2_file.round, 2);
        assert!(scores_path(&dir, 2).exists());
        let log = fs::read_to_string(events::event_log_path(&dir)).unwrap();
        assert_eq!(log.lines().count(), 2);
    }

    #[test]
    fn ensure_resume_rejects_tampered_file() {
        let dir = tmp_dir("ensure-resume-tampered");
        let metrics = sample_metrics();
        let mut seeded = sample_file(1);
        // Tamper: overwrite feasibility with an out-of-range score.
        if let Some(s) = seeded
            .scores
            .iter_mut()
            .find(|s| s.metric_id == "feasibility")
        {
            s.score = 99.0;
        }
        fs::create_dir_all(substrate::round_dir(&dir, 1)).unwrap();
        let body = serde_json::to_vec_pretty(&seeded).unwrap();
        fs::write(scores_path(&dir, 1), body).unwrap();

        let invoke = |_: &str| -> Result<String> {
            panic!("Scoring should not invoke LLM on resume");
        };
        let err = ensure_scores(
            &dir,
            &seeded.forum_id,
            "topic",
            1,
            &metrics,
            &sample_responses(),
            None,
            "claude-opus-4-6",
            invoke,
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("validation") || err.contains("out of range"),
            "got: {err}"
        );
    }

    #[test]
    fn ensure_surfaces_llm_error() {
        let dir = tmp_dir("ensure-llm-error");
        let metrics = sample_metrics();
        let invoke = |_: &str| -> Result<String> { anyhow::bail!("network down") };
        let err = ensure_scores(
            &dir,
            "fid",
            "topic",
            1,
            &metrics,
            &sample_responses(),
            None,
            "model",
            invoke,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("Scoring LLM"), "got: {err}");
        assert!(!scores_path(&dir, 1).exists());
    }

    #[test]
    fn ensure_surfaces_parse_error_and_writes_nothing() {
        let dir = tmp_dir("ensure-parse-error");
        let metrics = sample_metrics();
        let invoke = |_: &str| -> Result<String> { Ok("not json".to_string()) };
        let err = ensure_scores(
            &dir,
            "fid",
            "topic",
            1,
            &metrics,
            &sample_responses(),
            None,
            "model",
            invoke,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("JSON"), "got: {err}");
        assert!(!scores_path(&dir, 1).exists());
        assert!(!events::event_log_path(&dir).exists());
    }
}
