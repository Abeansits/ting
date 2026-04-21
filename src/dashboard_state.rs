//! Periodic compacted snapshot of the dashboard state.
//!
//! Phase 1A contract: the append-only JSONL event log is the source of truth;
//! `dashboard-state.json` is a compaction of the log so late joiners (SSE
//! clients, Go TUIs) can cheaply render the current state without replaying
//! every event. The protocol rewrites the snapshot atomically after each event
//! append (or debounced if profiling shows cost); consumers only ever see a
//! fully-written file. Full reader/writer rules live at `schemas/CONTRACT.md`.
//!
//! Durability: `write_state` fsyncs the temp file before rename and the
//! containing directory after, so a successful call means the bytes survive
//! a power loss, not just a process crash.
//!
//! Like `events`, nothing here is wired into `protocol.rs` yet. See the module
//! docs on `events` for the phase 1A / 1B split rationale.
#![allow(dead_code)]

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Current snapshot schema version. Bump on breaking changes to the top-level
/// shape (not when adding fields inside `metrics` / `rounds` payloads).
pub const STATE_VERSION: u32 = 1;

/// Filename for the compacted snapshot inside a forum directory.
pub const STATE_FILENAME: &str = "dashboard-state.json";

/// Per-round summary recorded in the snapshot.
///
/// Payloads (`synthesis`, `metric_scores`) are kept loosely-typed at this layer
/// because their precise shape lives in the matching JSON Schema and is
/// specific to downstream consumers. Phase 1B / 1C can tighten these once the
/// classifier and metric-scoring contracts are nailed down.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoundSummary {
    pub round: u32,
    pub stage: String,
    pub participants_responded: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synthesis: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_scores: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub convergence_score: Option<f64>,
}

/// Top-level snapshot written to `dashboard-state.json`.
///
/// Holds just enough to render an initial dashboard frame without replaying
/// the event log. `latest_seq` pairs with the log tailer: consumers read the
/// snapshot, then resume from `latest_seq + 1` forward.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DashboardState {
    pub version: u32,
    pub forum_id: String,
    pub topic: String,
    pub participants: Vec<String>,
    pub max_rounds: u32,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub latest_seq: u64,
    pub status: ForumStatus,
    pub rounds: Vec<RoundSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classifier_metrics: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub convergence_score: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForumStatus {
    Pending,
    InProgress,
    Completed,
}

impl DashboardState {
    /// Construct an empty state for a freshly-created forum.
    pub fn new(
        forum_id: impl Into<String>,
        topic: impl Into<String>,
        participants: Vec<String>,
        max_rounds: u32,
    ) -> Self {
        let now = Utc::now();
        Self {
            version: STATE_VERSION,
            forum_id: forum_id.into(),
            topic: topic.into(),
            participants,
            max_rounds,
            created: now,
            updated: now,
            latest_seq: 0,
            status: ForumStatus::Pending,
            rounds: Vec::new(),
            classifier_metrics: None,
            convergence_score: None,
        }
    }
}

/// Path to the snapshot file inside a forum directory.
pub fn state_path(forum_dir: &Path) -> PathBuf {
    forum_dir.join(STATE_FILENAME)
}

/// Write `state` to `<forum_dir>/dashboard-state.json` atomically and durably.
///
/// Uses the standard `.tmp` + rename pattern (mirrors `substrate::write_atomic`)
/// so a concurrent reader never observes a partially-written file. JSON is
/// pretty-printed to make the snapshot human-readable during debugging.
///
/// Durability: we `fsync` the temp file before rename and the containing
/// directory after rename. Without both, a crash right after this call can
/// leave the snapshot empty, corrupted, or missing the rename — even though
/// POSIX rename is atomic *from a reader's perspective*, its on-disk
/// visibility after a crash is not guaranteed until the directory is fsynced.
pub fn write_state(forum_dir: &Path, state: &DashboardState) -> Result<()> {
    let path = state_path(forum_dir);
    let tmp_path = path.with_extension("json.tmp");

    let mut body =
        serde_json::to_vec_pretty(state).with_context(|| "Failed to serialize DashboardState")?;
    body.push(b'\n');

    // Write + fsync the temp file so its bytes are on disk before the rename
    // makes it visible under the final name.
    {
        let mut tmp = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp_path)
            .with_context(|| format!("Failed to open temp snapshot: {}", tmp_path.display()))?;
        tmp.write_all(&body)
            .with_context(|| format!("Failed to write temp snapshot: {}", tmp_path.display()))?;
        tmp.sync_all()
            .with_context(|| format!("Failed to fsync temp snapshot: {}", tmp_path.display()))?;
    }

    fs::rename(&tmp_path, &path).with_context(|| {
        format!(
            "Failed to rename {} -> {}",
            tmp_path.display(),
            path.display()
        )
    })?;

    // fsync the containing directory so the rename itself is durable.
    // Best-effort on platforms where directory fsync is a no-op or unsupported.
    if let Ok(dir) = File::open(forum_dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Read the snapshot, or `Ok(None)` if the file does not exist yet.
///
/// A malformed file is reported as an error (not `None`), because once the
/// snapshot exists it should always be a complete valid document — that's the
/// whole point of the atomic-rename writer.
pub fn read_state(forum_dir: &Path) -> Result<Option<DashboardState>> {
    let path = state_path(forum_dir);
    if !path.exists() {
        return Ok(None);
    }
    let body = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read snapshot: {}", path.display()))?;
    let state: DashboardState = serde_json::from_str(&body)
        .with_context(|| format!("Failed to parse snapshot JSON: {}", path.display()))?;
    Ok(Some(state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("ting-state-{}-{}-{}", label, pid, n));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_state() -> DashboardState {
        let mut s = DashboardState::new(
            "ting-2026-04-19-abcd1234",
            "Is the sky falling?",
            vec!["codex".into(), "gemini".into(), "claude".into()],
            3,
        );
        s.status = ForumStatus::InProgress;
        s.latest_seq = 7;
        s.rounds.push(RoundSummary {
            round: 1,
            stage: "proposal".into(),
            participants_responded: vec!["codex".into(), "gemini".into()],
            synthesis: None,
            metric_scores: None,
            convergence_score: None,
        });
        s
    }

    #[test]
    fn read_missing_returns_none() {
        let dir = tmp_dir("read-missing");
        assert!(read_state(&dir).unwrap().is_none());
    }

    #[test]
    fn write_then_read_round_trip() {
        let dir = tmp_dir("round-trip");
        let state = sample_state();
        write_state(&dir, &state).unwrap();
        let back = read_state(&dir).unwrap().unwrap();
        assert_eq!(back, state);
    }

    fn load_state_validator() -> jsonschema::Validator {
        let schema_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("schemas")
            .join("dashboard-state.schema.json");
        let body = fs::read_to_string(&schema_path).unwrap();
        let schema: Value = serde_json::from_str(&body).unwrap();
        jsonschema::validator_for(&schema).unwrap()
    }

    #[test]
    fn state_validates_against_committed_schema() {
        let validator = load_state_validator();

        let mut state = sample_state();
        state.classifier_metrics = Some(json!({
            "metrics": [
                { "id": "dissent_axis", "name": "Dissent", "scale": 10, "mandatory": true }
            ]
        }));
        state.convergence_score = Some(6.1);
        state.rounds[0].synthesis = Some(json!({ "word_count": 220 }));
        state.rounds[0].convergence_score = Some(6.1);

        let value = serde_json::to_value(&state).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&value)
            .map(|e| e.to_string())
            .collect();
        assert!(
            errors.is_empty(),
            "state failed schema validation: {:#?}\nvalue: {}",
            errors,
            serde_json::to_string_pretty(&value).unwrap()
        );
    }

    #[test]
    fn minimal_fresh_state_validates() {
        let validator = load_state_validator();
        let state =
            DashboardState::new("ting-2026-04-19-abcd1234", "Topic", vec!["codex".into()], 2);
        let value = serde_json::to_value(&state).unwrap();
        let errors: Vec<String> = validator
            .iter_errors(&value)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "minimal state rejected: {:?}", errors);
    }

    #[test]
    fn state_version_matches_schema_contract() {
        // Trip-wire: the committed schema pins v0.4 Phase 1A to version=1.
        assert_eq!(STATE_VERSION, 1);
    }

    #[test]
    fn write_cleans_up_tmp_and_produces_valid_json() {
        let dir = tmp_dir("cleanup");
        let state = sample_state();
        write_state(&dir, &state).unwrap();
        let path = state_path(&dir);
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());

        // JSON is well-formed and re-parseable as a generic Value too.
        let body = fs::read_to_string(&path).unwrap();
        let _: Value = serde_json::from_str(&body).unwrap();
    }

    /// The reader must never see a torn/partial snapshot while a writer is
    /// racing it. We exploit the atomic rename: at every instant the file
    /// either does not exist or is a complete valid document.
    #[test]
    fn concurrent_reader_never_sees_torn_write() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        use std::time::Instant;

        let dir = Arc::new(tmp_dir("concurrent"));
        let stop = Arc::new(AtomicBool::new(false));

        // Seed a first version so the reader has something to find.
        write_state(&dir, &sample_state()).unwrap();

        let reader_dir = Arc::clone(&dir);
        let reader_stop = Arc::clone(&stop);
        let reader = thread::spawn(move || {
            let mut reads: u64 = 0;
            while !reader_stop.load(Ordering::Relaxed) {
                // Any read must either be None or a fully-parseable state.
                match read_state(&reader_dir) {
                    Ok(Some(s)) => {
                        assert_eq!(s.forum_id, "ting-2026-04-19-abcd1234");
                    }
                    Ok(None) => {}
                    Err(e) => panic!("torn read observed: {e}"),
                }
                reads += 1;
            }
            reads
        });

        // Writer: rewrite the snapshot with varying latest_seq for ~150ms.
        let start = Instant::now();
        let mut writes: u64 = 0;
        while start.elapsed().as_millis() < 150 {
            let mut s = sample_state();
            s.latest_seq = writes;
            write_state(&dir, &s).unwrap();
            writes += 1;
        }
        stop.store(true, Ordering::Relaxed);
        let reads = reader.join().unwrap();
        assert!(writes > 0, "expected at least one write");
        assert!(reads > 0, "expected at least one read");
    }
}
