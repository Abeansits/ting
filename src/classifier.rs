//! Pre-round "Fire Keeper" classifier.
//!
//! Runs once per forum, before round 1, when `--dashboard` is on and
//! `--no-classifier` is not set. Asks the Fire Keeper model for 5-10
//! topic-specific metrics plus one mandatory `dissent_axis`. The result lands
//! on disk as `round-0/metrics.json` and as a `classifier_metrics` event on
//! `dashboard-events.jsonl`.
//!
//! Resume semantics: if `round-0/metrics.json` already exists we short-circuit
//! — no LLM call, no duplicate event. The disk artifact is the source of
//! truth; the event was appended on the fresh run.
//!
//! `ensure_classifier` is the single integration point for `protocol.rs`. It
//! takes an `invoke` closure so tests can stub the LLM without touching
//! `substrate::invoke_fire_keeper_model`.
#![allow(dead_code)]

use crate::events::{self, DashboardEvent, EventType};
use crate::substrate;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Snapshot schema version written to `metrics.json`. Bump on breaking changes
/// to the top-level file shape; additive metric fields do not bump this.
pub const METRICS_VERSION: u32 = 1;

pub const METRICS_FILENAME: &str = "metrics.json";

/// Classifier output lives alongside the round directories but is scored over
/// the forum as a whole, so it gets a pseudo-round 0.
pub const ROUND_0_INDEX: u32 = 0;

/// Required id of the mandatory dissent axis metric. Hardcoded so the prompt,
/// the validator, and every downstream consumer agree on one canonical key.
pub const DISSENT_AXIS_ID: &str = "dissent_axis";

/// 5-10 question-specific metrics plus the dissent axis (6-11 entries total).
pub const MIN_TOPIC_METRICS: usize = 5;
pub const MAX_TOPIC_METRICS: usize = 10;

/// Upper bound we accept for any metric's `scale`. Matches the prompt and the
/// committed JSON Schema, and is tight enough for the dashboard to render as
/// a discrete bar without rescaling.
pub const MAX_SCALE: u32 = 10;

/// A single axis the dashboard can render for this forum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifierMetric {
    pub id: String,
    pub name: String,
    pub scale: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mandatory: Option<bool>,
}

/// On-disk envelope for `round-0/metrics.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassifierMetricsFile {
    pub version: u32,
    pub forum_id: String,
    pub generated_at: DateTime<Utc>,
    pub model: String,
    pub metrics: Vec<ClassifierMetric>,
}

/// Shape the LLM emits — a `{ "metrics": [...] }` wrapper around the array.
/// Exposed as a separate type so `parse_classifier_response` can deserialize
/// straight into it instead of going through `serde_json::Value`.
#[derive(Debug, Deserialize)]
struct ClassifierResponse {
    metrics: Vec<ClassifierMetric>,
}

pub fn metrics_path(forum_dir: &Path) -> PathBuf {
    substrate::round_dir(forum_dir, ROUND_0_INDEX).join(METRICS_FILENAME)
}

/// Build the classifier prompt. The dissent-axis requirement is baked in —
/// the parser enforces it, but the prompt states it explicitly so the LLM
/// doesn't have to infer it from schema.
pub fn build_classifier_prompt(topic: &str, context: Option<&str>) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You are the Fire Keeper for a structured multi-agent deliberation. \
         Your job right now is a pre-round classification pass: pick the 5-10 \
         metrics that best capture what participants should be evaluated on \
         for THIS specific question, plus one mandatory dissent axis.\n\n",
    );
    prompt.push_str("## Forum Topic\n\n");
    prompt.push_str(topic.trim());
    prompt.push_str("\n\n");

    if let Some(ctx) = context.map(str::trim).filter(|c| !c.is_empty()) {
        prompt.push_str("## Supplementary Context\n\n");
        prompt.push_str(ctx);
        prompt.push_str("\n\n");
    }

    prompt.push_str(
        "## Instructions\n\n\
         Pick 5 to 10 metrics that are specific to the question above. Avoid \
         generic catch-alls (\"quality\", \"goodness\") — each metric should \
         name a concrete axis on which answers to THIS question meaningfully \
         differ. Each metric needs:\n\n\
         - `id`: snake_case identifier, unique within the set, [a-z0-9_] only.\n\
         - `name`: short human label (Title Case).\n\
         - `scale`: integer 2-10 (upper bound of the score range, 1..=scale).\n\
         - `description`: one sentence explaining what the metric captures.\n\n\
         ## Mandatory Dissent Axis\n\n\
         In addition to the 5-10 topic metrics you choose, include exactly one \
         extra metric with `id: \"dissent_axis\"`, a descriptive name like \
         \"Dissent\" or \"Unresolved Disagreement\", `scale: 10`, and \
         `\"mandatory\": true`. This axis tracks how much genuine disagreement \
         persists through the deliberation — it is non-optional because \
         preserving dissent is a core goal of this tool.\n\n\
         ## Output Format\n\n\
         Output ONLY a JSON object with a single `metrics` array. No prose, no \
         markdown fences, no trailing commentary. Shape:\n\n\
         {\n  \"metrics\": [\n    { \"id\": \"…\", \"name\": \"…\", \"scale\": 10, \"description\": \"…\" },\n    …\n    { \"id\": \"dissent_axis\", \"name\": \"Dissent\", \"scale\": 10, \"description\": \"…\", \"mandatory\": true }\n  ]\n}\n",
    );
    prompt
}

/// Parse a classifier response into a validated metrics vector. Tolerates
/// ` ```json … ``` ` code fences. Enforces one `dissent_axis`
/// (`mandatory: true`) plus 5-10 topic metrics, unique snake_case ids, and
/// scale in `2..=MAX_SCALE`.
pub fn parse_classifier_response(raw: &str) -> Result<Vec<ClassifierMetric>> {
    let cleaned = strip_code_fences(raw.trim());
    let response: ClassifierResponse = serde_json::from_str(cleaned)
        .with_context(|| "Classifier response was not valid JSON with a `metrics` array")?;
    validate_metrics(&response.metrics)?;
    Ok(response.metrics)
}

fn strip_code_fences(s: &str) -> &str {
    let trimmed = s.trim();
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    body.trim().strip_suffix("```").unwrap_or(body).trim()
}

fn validate_metrics(metrics: &[ClassifierMetric]) -> Result<()> {
    let dissent_count = metrics.iter().filter(|m| m.id == DISSENT_AXIS_ID).count();
    if dissent_count == 0 {
        bail!(
            "Classifier output missing mandatory `{}` metric",
            DISSENT_AXIS_ID
        );
    }
    if dissent_count > 1 {
        bail!(
            "Classifier output has {} `{}` entries, expected exactly 1",
            dissent_count,
            DISSENT_AXIS_ID
        );
    }

    let topic_count = metrics.len() - 1;
    if !(MIN_TOPIC_METRICS..=MAX_TOPIC_METRICS).contains(&topic_count) {
        bail!(
            "Classifier produced {} topic metrics, expected {}-{}",
            topic_count,
            MIN_TOPIC_METRICS,
            MAX_TOPIC_METRICS
        );
    }

    let mut seen_ids: HashSet<&str> = HashSet::with_capacity(metrics.len());
    for m in metrics {
        if m.id.is_empty() {
            bail!("Classifier metric has empty id");
        }
        if !m
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            bail!("Metric id `{}` must be snake_case [a-z0-9_]", m.id);
        }
        if !seen_ids.insert(&m.id) {
            bail!("Classifier emitted duplicate metric id `{}`", m.id);
        }
        if m.name.trim().is_empty() {
            bail!("Metric `{}` has empty name", m.id);
        }
        if !(2..=MAX_SCALE).contains(&m.scale) {
            bail!(
                "Metric `{}` has scale {}, expected 2..={}",
                m.id,
                m.scale,
                MAX_SCALE,
            );
        }
        if m.id == DISSENT_AXIS_ID && m.mandatory != Some(true) {
            bail!("`{}` must have `mandatory: true`", DISSENT_AXIS_ID);
        }
    }

    Ok(())
}

/// Write `metrics.json` atomically under `<forum_dir>/round-0/`. Uses the same
/// `.tmp` + rename + fsync dance as `dashboard_state::write_state` so readers
/// never observe a torn file and the snapshot survives a power loss.
pub fn write_metrics(forum_dir: &Path, file: &ClassifierMetricsFile) -> Result<()> {
    let dir = substrate::create_round_dir(forum_dir, ROUND_0_INDEX)?;
    let final_path = dir.join(METRICS_FILENAME);
    let tmp_path = final_path.with_extension("json.tmp");

    let mut body = serde_json::to_vec_pretty(file)
        .with_context(|| "Failed to serialize ClassifierMetricsFile")?;
    body.push(b'\n');

    {
        let mut tmp = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp_path)
            .with_context(|| format!("Failed to open temp metrics file: {}", tmp_path.display()))?;
        tmp.write_all(&body).with_context(|| {
            format!("Failed to write temp metrics file: {}", tmp_path.display())
        })?;
        tmp.sync_all().with_context(|| {
            format!("Failed to fsync temp metrics file: {}", tmp_path.display())
        })?;
    }

    fs::rename(&tmp_path, &final_path).with_context(|| {
        format!(
            "Failed to rename {} -> {}",
            tmp_path.display(),
            final_path.display()
        )
    })?;

    // fsync the containing directory so the rename itself is durable.
    // Best-effort on platforms where directory fsync is a no-op.
    if let Ok(dir_file) = File::open(&dir) {
        let _ = dir_file.sync_all();
    }
    Ok(())
}

pub fn read_metrics(forum_dir: &Path) -> Result<Option<ClassifierMetricsFile>> {
    let path = metrics_path(forum_dir);
    if !path.exists() {
        return Ok(None);
    }
    let body = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read metrics.json: {}", path.display()))?;
    let file: ClassifierMetricsFile = serde_json::from_str(&body)
        .with_context(|| format!("Failed to parse metrics.json: {}", path.display()))?;
    Ok(Some(file))
}

fn classifier_metrics_payload(metrics: &[ClassifierMetric]) -> Value {
    serde_json::json!({ "metrics": metrics })
}

fn emit_classifier_event(forum_dir: &Path, file: &ClassifierMetricsFile) -> Result<()> {
    let seq = events::next_seq(forum_dir)?;
    let payload = classifier_metrics_payload(&file.metrics);
    let event = DashboardEvent::new(
        seq,
        file.forum_id.clone(),
        EventType::ClassifierMetrics,
        payload,
    );
    events::append_event(forum_dir, &event)
}

/// Whether `ensure_classifier` ran the LLM (`Fresh`) or reused a prior run's
/// `metrics.json` (`Resumed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassifierOutcome {
    Fresh,
    Resumed,
}

/// If `metrics.json` already exists, reads it and returns `Resumed` — no LLM
/// call, no duplicate event. Otherwise invokes `invoke` with the classifier
/// prompt, parses the response, writes `metrics.json`, appends the
/// `classifier_metrics` event, and returns `Fresh`. The closure is injected so
/// tests can stub the LLM.
pub fn ensure_classifier<F>(
    forum_dir: &Path,
    forum_id: &str,
    topic: &str,
    context: Option<&str>,
    model: &str,
    invoke: F,
) -> Result<(ClassifierMetricsFile, ClassifierOutcome)>
where
    F: FnOnce(&str) -> Result<String>,
{
    if let Some(existing) = read_metrics(forum_dir)? {
        return Ok((existing, ClassifierOutcome::Resumed));
    }

    let prompt = build_classifier_prompt(topic, context);
    let raw = invoke(&prompt).with_context(|| "Classifier LLM call failed")?;
    let metrics = parse_classifier_response(&raw)?;

    let file = ClassifierMetricsFile {
        version: METRICS_VERSION,
        forum_id: forum_id.to_string(),
        generated_at: Utc::now(),
        model: model.to_string(),
        metrics,
    };

    write_metrics(forum_dir, &file)?;
    emit_classifier_event(forum_dir, &file)?;
    Ok((file, ClassifierOutcome::Fresh))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("ting-classifier-{}-{}-{}", label, pid, n));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_llm_response() -> String {
        json!({
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
        .to_string()
    }

    // ---- prompt tests ------------------------------------------------------

    #[test]
    fn prompt_includes_topic_and_dissent_instruction() {
        let prompt = build_classifier_prompt("Is the sky falling?", None);
        assert!(prompt.contains("Is the sky falling?"));
        assert!(prompt.contains("dissent_axis"));
        assert!(prompt.contains("mandatory"));
        assert!(prompt.contains("5 to 10"));
    }

    #[test]
    fn prompt_includes_context_when_present() {
        let prompt = build_classifier_prompt("Topic", Some("Additional context here."));
        assert!(prompt.contains("Additional context here."));
        assert!(prompt.contains("Supplementary Context"));
    }

    #[test]
    fn prompt_omits_context_when_empty() {
        let prompt = build_classifier_prompt("Topic", Some("   "));
        assert!(!prompt.contains("Supplementary Context"));
    }

    // ---- parse tests -------------------------------------------------------

    #[test]
    fn parse_well_formed_response() {
        let metrics = parse_classifier_response(&sample_llm_response()).unwrap();
        assert_eq!(metrics.len(), 6);
        assert!(metrics
            .iter()
            .any(|m| m.id == "dissent_axis" && m.mandatory == Some(true)));
    }

    #[test]
    fn parse_tolerates_code_fences() {
        let wrapped = format!("```json\n{}\n```", sample_llm_response());
        let metrics = parse_classifier_response(&wrapped).unwrap();
        assert_eq!(metrics.len(), 6);
    }

    #[test]
    fn parse_tolerates_bare_fences() {
        let wrapped = format!("```\n{}\n```", sample_llm_response());
        let metrics = parse_classifier_response(&wrapped).unwrap();
        assert_eq!(metrics.len(), 6);
    }

    #[test]
    fn parse_rejects_missing_dissent_axis() {
        let bad = json!({
            "metrics": [
                { "id": "feasibility", "name": "F", "scale": 10 },
                { "id": "cost", "name": "C", "scale": 5 },
                { "id": "risk", "name": "R", "scale": 10 },
                { "id": "speed", "name": "S", "scale": 10 },
                { "id": "user_value", "name": "U", "scale": 10 }
            ]
        })
        .to_string();
        let err = parse_classifier_response(&bad).unwrap_err().to_string();
        assert!(err.contains("dissent_axis"), "got: {err}");
    }

    #[test]
    fn parse_rejects_dissent_axis_without_mandatory() {
        let bad = json!({
            "metrics": [
                { "id": "feasibility", "name": "F", "scale": 10 },
                { "id": "cost", "name": "C", "scale": 5 },
                { "id": "risk", "name": "R", "scale": 10 },
                { "id": "speed", "name": "S", "scale": 10 },
                { "id": "user_value", "name": "U", "scale": 10 },
                { "id": "dissent_axis", "name": "Dissent", "scale": 10 }
            ]
        })
        .to_string();
        let err = parse_classifier_response(&bad).unwrap_err().to_string();
        assert!(err.contains("mandatory"), "got: {err}");
    }

    #[test]
    fn parse_rejects_too_few_topic_metrics() {
        let bad = json!({
            "metrics": [
                { "id": "feasibility", "name": "F", "scale": 10 },
                { "id": "dissent_axis", "name": "Dissent", "scale": 10, "mandatory": true }
            ]
        })
        .to_string();
        let err = parse_classifier_response(&bad).unwrap_err().to_string();
        assert!(err.contains("topic metrics"), "got: {err}");
    }

    #[test]
    fn parse_rejects_scale_out_of_range() {
        let bad = json!({
            "metrics": [
                { "id": "feasibility", "name": "F", "scale": 1 },
                { "id": "cost", "name": "C", "scale": 5 },
                { "id": "risk", "name": "R", "scale": 10 },
                { "id": "speed", "name": "S", "scale": 10 },
                { "id": "user_value", "name": "U", "scale": 10 },
                { "id": "dissent_axis", "name": "Dissent", "scale": 10, "mandatory": true }
            ]
        })
        .to_string();
        let err = parse_classifier_response(&bad).unwrap_err().to_string();
        assert!(err.contains("scale"), "got: {err}");
    }

    #[test]
    fn parse_rejects_duplicate_ids() {
        let bad = json!({
            "metrics": [
                { "id": "feasibility", "name": "F", "scale": 10 },
                { "id": "feasibility", "name": "F2", "scale": 10 },
                { "id": "risk", "name": "R", "scale": 10 },
                { "id": "speed", "name": "S", "scale": 10 },
                { "id": "user_value", "name": "U", "scale": 10 },
                { "id": "dissent_axis", "name": "Dissent", "scale": 10, "mandatory": true }
            ]
        })
        .to_string();
        let err = parse_classifier_response(&bad).unwrap_err().to_string();
        assert!(err.contains("duplicate"), "got: {err}");
    }

    #[test]
    fn parse_rejects_non_snake_case_id() {
        let bad = json!({
            "metrics": [
                { "id": "Feasibility", "name": "F", "scale": 10 },
                { "id": "cost", "name": "C", "scale": 5 },
                { "id": "risk", "name": "R", "scale": 10 },
                { "id": "speed", "name": "S", "scale": 10 },
                { "id": "user_value", "name": "U", "scale": 10 },
                { "id": "dissent_axis", "name": "Dissent", "scale": 10, "mandatory": true }
            ]
        })
        .to_string();
        let err = parse_classifier_response(&bad).unwrap_err().to_string();
        assert!(err.contains("snake_case"), "got: {err}");
    }

    #[test]
    fn parse_rejects_invalid_json() {
        let err = parse_classifier_response("not json at all")
            .unwrap_err()
            .to_string();
        assert!(err.contains("valid JSON"), "got: {err}");
    }

    // ---- write/read round trip --------------------------------------------

    fn sample_file() -> ClassifierMetricsFile {
        ClassifierMetricsFile {
            version: METRICS_VERSION,
            forum_id: "ting-2026-04-19-abcd1234".to_string(),
            generated_at: Utc::now(),
            model: "claude-opus-4-6".to_string(),
            metrics: parse_classifier_response(&sample_llm_response()).unwrap(),
        }
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = tmp_dir("round-trip");
        let file = sample_file();
        write_metrics(&dir, &file).unwrap();
        let back = read_metrics(&dir).unwrap().unwrap();
        assert_eq!(back, file);
    }

    #[test]
    fn read_missing_returns_none() {
        let dir = tmp_dir("read-missing");
        assert!(read_metrics(&dir).unwrap().is_none());
    }

    #[test]
    fn write_cleans_up_tmp() {
        let dir = tmp_dir("cleanup");
        write_metrics(&dir, &sample_file()).unwrap();
        let tmp = metrics_path(&dir).with_extension("json.tmp");
        assert!(!tmp.exists());
        assert!(metrics_path(&dir).exists());
    }

    // ---- schema golden ----------------------------------------------------

    fn load_metrics_validator() -> jsonschema::Validator {
        let schema_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("schemas")
            .join("metrics.schema.json");
        let body = fs::read_to_string(&schema_path).unwrap();
        let schema: Value = serde_json::from_str(&body).unwrap();
        jsonschema::validator_for(&schema).unwrap()
    }

    #[test]
    fn file_validates_against_metrics_schema() {
        let validator = load_metrics_validator();
        let file = sample_file();
        let value = serde_json::to_value(&file).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&value)
            .map(|e| e.to_string())
            .collect();
        assert!(
            errors.is_empty(),
            "file failed schema validation: {:#?}\nvalue: {}",
            errors,
            serde_json::to_string_pretty(&value).unwrap()
        );
    }

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
        let file = sample_file();
        let payload = classifier_metrics_payload(&file.metrics);
        let event = DashboardEvent::new(1, &file.forum_id, EventType::ClassifierMetrics, payload);
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

    // ---- integration: ensure_classifier ------------------------------------

    #[test]
    fn ensure_fresh_invokes_llm_writes_file_and_event() {
        let dir = tmp_dir("ensure-fresh");
        let called = Cell::new(0usize);
        let invoke = |prompt: &str| -> Result<String> {
            called.set(called.get() + 1);
            assert!(prompt.contains("Is the sky falling?"));
            assert!(prompt.contains("dissent_axis"));
            Ok(sample_llm_response())
        };

        let (file, outcome) = ensure_classifier(
            &dir,
            "ting-2026-04-19-abcd1234",
            "Is the sky falling?",
            None,
            "claude-opus-4-6",
            invoke,
        )
        .unwrap();

        assert_eq!(outcome, ClassifierOutcome::Fresh);
        assert_eq!(called.get(), 1);
        assert_eq!(file.forum_id, "ting-2026-04-19-abcd1234");
        assert_eq!(file.metrics.len(), 6);
        assert!(metrics_path(&dir).exists());

        // Event log has exactly one classifier_metrics line.
        let log = fs::read_to_string(events::event_log_path(&dir)).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 1);
        let event: DashboardEvent = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(event.event_type, EventType::ClassifierMetrics);
        assert_eq!(event.seq, 1);
    }

    #[test]
    fn ensure_resume_reads_disk_and_does_not_invoke_llm() {
        let dir = tmp_dir("ensure-resume");
        // Seed the directory with a prior run's metrics.json.
        let seeded = sample_file();
        write_metrics(&dir, &seeded).unwrap();

        let invoke = |_: &str| -> Result<String> {
            panic!("Classifier should not invoke LLM on resume");
        };

        let (file, outcome) = ensure_classifier(
            &dir,
            &seeded.forum_id,
            "Is the sky falling?",
            None,
            "claude-opus-4-6",
            invoke,
        )
        .unwrap();

        assert_eq!(outcome, ClassifierOutcome::Resumed);
        assert_eq!(file, seeded);

        // No event appended on resume — event came from the fresh run previously.
        assert!(!events::event_log_path(&dir).exists());
    }

    #[test]
    fn ensure_surfaces_llm_error() {
        let dir = tmp_dir("ensure-llm-error");
        let invoke = |_: &str| -> Result<String> { anyhow::bail!("network down") };
        let err = ensure_classifier(&dir, "fid", "topic", None, "model", invoke)
            .unwrap_err()
            .to_string();
        assert!(err.contains("Classifier LLM"), "got: {err}");
        assert!(!metrics_path(&dir).exists());
    }

    #[test]
    fn ensure_surfaces_parse_error_and_writes_nothing() {
        let dir = tmp_dir("ensure-parse-error");
        let invoke = |_: &str| -> Result<String> { Ok("not json".to_string()) };
        let err = ensure_classifier(&dir, "fid", "topic", None, "model", invoke)
            .unwrap_err()
            .to_string();
        assert!(err.contains("JSON"), "got: {err}");
        assert!(!metrics_path(&dir).exists());
        assert!(!events::event_log_path(&dir).exists());
    }
}
