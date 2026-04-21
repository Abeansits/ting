//! Dashboard event envelope and append-only JSONL writer.
//!
//! Phase 1A contract: a single-writer JSONL log (`dashboard-events.jsonl`) in
//! the forum directory. Each line is a self-contained `DashboardEvent`. The
//! full reader/writer contract lives at `schemas/CONTRACT.md`; the key rules:
//!
//! - **Single writer.** The Rust protocol is the only writer. Consumers are
//!   read-only tailers. Multi-writer would need a lock file; out of scope for
//!   v0.4.
//! - **`seq` is authoritative ordering.** `timestamp` is informational only.
//! - **Append atomicity.** `write_all` of one JSON-line-plus-`\n` under
//!   `PIPE_BUF` (4096) with `O_APPEND` is atomic on Linux and macOS: readers
//!   see either zero bytes of the new line or the full line — never a split.
//! - **Consumers handle malformed lines gracefully** (skip with warning) and
//!   hold trailing non-newline bytes as in-flight writes until `\n` arrives.
//!
//! Nothing in this module is wired into `protocol.rs` yet — Phase 1A ships the
//! contract and infrastructure, Phase 1B does the integration. The crate-wide
//! `#[allow(dead_code)]` below is intentional for this phase; Phase 1B will
//! remove it when the protocol starts emitting events and will introduce an
//! `EventWriter` that caches the open FD and next seq (see CONTRACT.md).
#![allow(dead_code)]

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Current event envelope version. Bump when the envelope shape changes in a
/// way that breaks older readers (not when adding event types or payload fields).
pub const EVENT_VERSION: u32 = 1;

/// Filename of the append-only event log, relative to the forum directory.
pub const EVENT_LOG_FILENAME: &str = "dashboard-events.jsonl";

/// The kinds of events the Rust protocol emits during a forum run. Names are
/// serialized as snake_case to match the cross-language contract documented in
/// `schemas/dashboard-event.schema.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    ForumStarted,
    RoundStarted,
    ParticipantResponse,
    Synthesis,
    Claims,
    Alignment,
    ClassifierMetrics,
    MetricScores,
    Convergence,
    ForumComplete,
}

/// Envelope written as one line of `dashboard-events.jsonl`.
///
/// The payload is deliberately untyped (`serde_json::Value`) at the envelope
/// layer: each `EventType` has its own payload shape documented in
/// `schemas/dashboard-event.schema.json`. Keeping the envelope generic lets the
/// writer grow new event types without churning this struct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DashboardEvent {
    pub version: u32,
    pub seq: u64,
    pub forum_id: String,
    pub timestamp: DateTime<Utc>,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub payload: Value,
}

impl DashboardEvent {
    /// Construct an event with `version = EVENT_VERSION` and `timestamp = now`.
    pub fn new(
        seq: u64,
        forum_id: impl Into<String>,
        event_type: EventType,
        payload: Value,
    ) -> Self {
        Self {
            version: EVENT_VERSION,
            seq,
            forum_id: forum_id.into(),
            timestamp: Utc::now(),
            event_type,
            payload,
        }
    }
}

/// Path to the event log file inside a forum directory.
pub fn event_log_path(forum_dir: &Path) -> PathBuf {
    forum_dir.join(EVENT_LOG_FILENAME)
}

/// Append a single event to `<forum_dir>/dashboard-events.jsonl` and fsync.
///
/// Serializes the envelope as one JSON object followed by a newline. The open
/// uses `O_APPEND` so concurrent appends from a single writer are atomic at
/// the kernel level; cross-process concurrent writers are not supported (see
/// module docs).
pub fn append_event(forum_dir: &Path, event: &DashboardEvent) -> Result<()> {
    let path = event_log_path(forum_dir);
    let mut line =
        serde_json::to_string(event).with_context(|| "Failed to serialize DashboardEvent")?;
    line.push('\n');

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("Failed to open event log: {}", path.display()))?;
    file.write_all(line.as_bytes())
        .with_context(|| format!("Failed to append event: {}", path.display()))?;
    file.sync_data()
        .with_context(|| format!("Failed to fsync event log: {}", path.display()))?;
    Ok(())
}

/// True if the log contains at least one event of the given type. Used by
/// resume paths to check idempotency ("did the fresh run manage to emit the
/// event before it crashed?") without re-appending duplicates. Malformed
/// lines are skipped; an IO error on open propagates.
pub fn log_contains(forum_dir: &Path, event_type: EventType) -> Result<bool> {
    scan_log(forum_dir, |evt| evt.event_type == event_type)
}

/// True if the log contains an event of the given type whose payload has a
/// `round` field equal to `round`. Used by per-round resume paths (e.g.
/// `metric_scores` is emitted once per round, so idempotency is keyed on
/// `(type, round)` rather than `type` alone).
pub fn log_contains_round(forum_dir: &Path, event_type: EventType, round: u32) -> Result<bool> {
    scan_log(forum_dir, |evt| {
        evt.event_type == event_type
            && evt.payload.get("round").and_then(|v| v.as_u64()) == Some(round as u64)
    })
}

fn scan_log<F>(forum_dir: &Path, mut predicate: F) -> Result<bool>
where
    F: FnMut(&DashboardEvent) -> bool,
{
    let path = event_log_path(forum_dir);
    if !path.exists() {
        return Ok(false);
    }
    let file = File::open(&path)
        .with_context(|| format!("Failed to open event log: {}", path.display()))?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line.with_context(|| format!("Failed to read event log: {}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(evt) = serde_json::from_str::<DashboardEvent>(&line)
            && predicate(&evt)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Return the next sequence number to assign: `max(seq) + 1`, or `1` if the log
/// is empty or missing.
///
/// Used on boot/resume so a restarted protocol continues where it left off.
/// Malformed lines are skipped with a warning on stderr rather than failing —
/// the goal is to get the highest known seq and keep going.
pub fn next_seq(forum_dir: &Path) -> Result<u64> {
    let path = event_log_path(forum_dir);
    if !path.exists() {
        return Ok(1);
    }

    let file = File::open(&path)
        .with_context(|| format!("Failed to open event log: {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut max_seq: u64 = 0;
    for (line_no, line) in reader.lines().enumerate() {
        let line =
            line.with_context(|| format!("Failed to read event log line {}", line_no + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<DashboardEvent>(&line) {
            Ok(evt) => {
                if evt.seq > max_seq {
                    max_seq = evt.seq;
                }
            }
            Err(e) => {
                eprintln!(
                    "warning: skipping malformed event at {}:{}: {}",
                    path.display(),
                    line_no + 1,
                    e
                );
            }
        }
    }

    Ok(max_seq + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("ting-events-{}-{}-{}", label, pid, n));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_payload(event_type: EventType) -> Value {
        match event_type {
            EventType::ForumStarted => json!({
                "topic": "Is the sky falling?",
                "participants": ["codex", "gemini", "claude"],
                "max_rounds": 3
            }),
            EventType::RoundStarted => json!({ "round": 1, "stage": "proposal" }),
            EventType::ParticipantResponse => json!({
                "round": 1,
                "participant": "codex",
                "word_count": 412
            }),
            EventType::Synthesis => json!({ "round": 1, "word_count": 220 }),
            EventType::Claims => json!({
                "round": 1,
                "claims": [{ "id": "c1", "text": "Yes." }]
            }),
            EventType::Alignment => json!({
                "round": 1,
                "alignment": { "codex": "agree", "gemini": "disagree" }
            }),
            EventType::ClassifierMetrics => json!({
                "metrics": [
                    { "id": "feasibility", "name": "Feasibility", "scale": 10 },
                    { "id": "dissent_axis", "name": "Dissent", "scale": 10, "mandatory": true }
                ]
            }),
            EventType::MetricScores => json!({
                "round": 2,
                "scores": [
                    { "metric_id": "feasibility", "score": 7.5 },
                    { "metric_id": "dissent_axis", "score": 4.2 }
                ]
            }),
            EventType::Convergence => json!({ "round": 2, "score": 6.1 }),
            EventType::ForumComplete => json!({ "rounds_used": 2 }),
        }
    }

    const ALL_EVENT_TYPES: &[EventType] = &[
        EventType::ForumStarted,
        EventType::RoundStarted,
        EventType::ParticipantResponse,
        EventType::Synthesis,
        EventType::Claims,
        EventType::Alignment,
        EventType::ClassifierMetrics,
        EventType::MetricScores,
        EventType::Convergence,
        EventType::ForumComplete,
    ];

    #[test]
    fn event_types_round_trip_through_json() {
        for &event_type in ALL_EVENT_TYPES {
            let event = DashboardEvent::new(
                1,
                "ting-2026-04-19-abcd1234",
                event_type,
                sample_payload(event_type),
            );
            let line = serde_json::to_string(&event).expect("serialize");
            let decoded: DashboardEvent = serde_json::from_str(&line).expect("deserialize");
            assert_eq!(decoded, event, "round-trip mismatch for {:?}", event_type);
        }
    }

    #[test]
    fn event_type_serializes_as_snake_case() {
        let event = DashboardEvent::new(1, "fid", EventType::ParticipantResponse, json!({}));
        let value: Value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], json!("participant_response"));
    }

    #[test]
    fn append_event_writes_one_line_per_event() {
        let dir = tmp_dir("append-one-line");
        for seq in 1..=3u64 {
            let evt =
                DashboardEvent::new(seq, "fid", EventType::RoundStarted, json!({ "round": seq }));
            append_event(&dir, &evt).unwrap();
        }
        let body = fs::read_to_string(event_log_path(&dir)).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 3);
        for line in lines {
            let parsed: DashboardEvent = serde_json::from_str(line).unwrap();
            assert_eq!(parsed.event_type, EventType::RoundStarted);
        }
    }

    #[test]
    fn next_seq_on_empty_dir_returns_one() {
        let dir = tmp_dir("next-seq-empty");
        assert_eq!(next_seq(&dir).unwrap(), 1);
    }

    #[test]
    fn next_seq_monotonic_across_simulated_restart() {
        let dir = tmp_dir("next-seq-restart");
        // Simulate process 1: write seq 1..=3.
        for seq in 1..=3u64 {
            let evt =
                DashboardEvent::new(seq, "fid", EventType::RoundStarted, json!({ "round": seq }));
            append_event(&dir, &evt).unwrap();
        }
        // Simulate process 2 coming back up and resuming.
        let resumed = next_seq(&dir).unwrap();
        assert_eq!(resumed, 4);

        // Append one more at the resumed seq and verify strict monotonicity.
        let evt = DashboardEvent::new(resumed, "fid", EventType::ForumComplete, json!({}));
        append_event(&dir, &evt).unwrap();
        assert_eq!(next_seq(&dir).unwrap(), 5);
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
    fn every_event_type_validates_against_committed_schema() {
        let validator = load_event_validator();
        for &event_type in ALL_EVENT_TYPES {
            let event = DashboardEvent::new(
                1,
                "ting-2026-04-19-abcd1234",
                event_type,
                sample_payload(event_type),
            );
            let value = serde_json::to_value(&event).unwrap();
            let errors: Vec<String> = validator
                .iter_errors(&value)
                .map(|e| e.to_string())
                .collect();
            assert!(
                errors.is_empty(),
                "event type {:?} failed schema validation: {:#?}\nvalue: {}",
                event_type,
                errors,
                serde_json::to_string_pretty(&value).unwrap()
            );
        }
    }

    #[test]
    fn schema_rejects_unknown_event_type() {
        let validator = load_event_validator();
        let bad = json!({
            "version": 1,
            "seq": 1,
            "forum_id": "f",
            "timestamp": Utc::now().to_rfc3339(),
            "type": "bogus_event",
            "payload": {}
        });
        assert!(!validator.is_valid(&bad));
    }

    #[test]
    fn schema_rejects_missing_envelope_fields() {
        let validator = load_event_validator();
        let bad = json!({ "version": 1, "seq": 1 });
        assert!(!validator.is_valid(&bad));
    }

    #[test]
    fn event_version_matches_schema_contract() {
        // Trip-wire: the committed schema pins v0.4 Phase 1A to version=1.
        // If EVENT_VERSION bumps, the schema must update in lockstep.
        assert_eq!(EVENT_VERSION, 1);
    }

    #[test]
    fn next_seq_skips_malformed_lines() {
        let dir = tmp_dir("next-seq-malformed");
        let path = event_log_path(&dir);
        // Good line at seq=1.
        append_event(
            &dir,
            &DashboardEvent::new(1, "fid", EventType::RoundStarted, json!({})),
        )
        .unwrap();
        // Corrupt line.
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"this is not json\n").unwrap();
        // Good line at seq=2.
        append_event(
            &dir,
            &DashboardEvent::new(2, "fid", EventType::RoundStarted, json!({})),
        )
        .unwrap();

        assert_eq!(next_seq(&dir).unwrap(), 3);
    }

    #[test]
    fn log_contains_round_matches_payload_round() {
        let dir = tmp_dir("log-contains-round");
        // Two metric_scores events for different rounds.
        for r in 1..=2u32 {
            append_event(
                &dir,
                &DashboardEvent::new(
                    r as u64,
                    "fid",
                    EventType::MetricScores,
                    json!({ "round": r, "scores": [] }),
                ),
            )
            .unwrap();
        }
        assert!(log_contains_round(&dir, EventType::MetricScores, 1).unwrap());
        assert!(log_contains_round(&dir, EventType::MetricScores, 2).unwrap());
        assert!(!log_contains_round(&dir, EventType::MetricScores, 3).unwrap());
        // Different type, same round → no match.
        assert!(!log_contains_round(&dir, EventType::Convergence, 1).unwrap());
    }

    #[test]
    fn log_contains_finds_present_and_rejects_missing() {
        let dir = tmp_dir("log-contains");
        assert!(!log_contains(&dir, EventType::ClassifierMetrics).unwrap());
        append_event(
            &dir,
            &DashboardEvent::new(1, "fid", EventType::RoundStarted, json!({})),
        )
        .unwrap();
        assert!(log_contains(&dir, EventType::RoundStarted).unwrap());
        assert!(!log_contains(&dir, EventType::ClassifierMetrics).unwrap());
        append_event(
            &dir,
            &DashboardEvent::new(
                2,
                "fid",
                EventType::ClassifierMetrics,
                json!({ "metrics": [] }),
            ),
        )
        .unwrap();
        assert!(log_contains(&dir, EventType::ClassifierMetrics).unwrap());
    }
}
