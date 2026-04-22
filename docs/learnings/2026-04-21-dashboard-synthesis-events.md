# 2026-04-21 — Dashboard synthesis / convergence / forum_complete events

## Gotchas

- `serde_json::json!({ "score": f })` panics on `NaN` / `inf`. The existing
  `parse_judge_response` in `convergence.rs` used
  `parse::<f32>().unwrap_or(5.0)`, but `"nan".parse::<f32>()` returns
  `Ok(NaN)` — so an LLM that emits `SCORE: nan` would crash the protocol
  as soon as the convergence event tried to serialize. Codex caught this on
  mid-flight review. Fix: fold non-finite values into the 5.0 fallback at
  the parse site.
- `ForumCompletePayload.rounds_used` was `minimum: 1` in the committed
  schema, which forced skipping the event on zero-round forums. That leaves
  the dashboard stream open forever on degenerate runs. Relaxed to
  `minimum: 0` and emit unconditionally when `--dashboard` is on.

## Subtle design calls

- `RunOptions::emit_events` is `== dashboard`, *not* derived via
  `classify && !no_classifier`. Rationale: the Convergence + Latest
  Synthesis widgets are independent of the classifier, so
  `--dashboard --no-classifier` should still populate them.
- The `events::emit` helper was hoisted into `src/events.rs` rather than
  left local to `protocol.rs`. `classifier.rs` and `metric_scoring.rs`
  each have their own in-module `emit_*_event` copies — those are future
  consolidation work, intentionally left out of scope here.

## Helpful commands

- Quick SSE tail: `curl -sSN -H "Accept: text/event-stream"
  http://127.0.0.1:<port>/api/events`
- Forum event-type histogram:
  `jq -r '.type' dashboard-events.jsonl | sort | uniq -c`

## Dead ends

- Considered capturing `forum_path` + `forum_id` in a closure to shave
  the repeated emit arguments. Borrow plumbing wasn't worth the 4 lines
  saved.
- Considered sanitizing NaN at the emit site instead of the parse site.
  Parse-site fix is better — it also protects pre-existing consumers
  (report.rs, dashboard_state.rs) that may stringify the score.
