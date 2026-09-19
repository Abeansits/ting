# Ting project review

Reviewed: 2026-09-18 against commit `a674164` (v0.4.1).

Goal: make Ting trustworthy, easy to try, and welcoming to open-source contributors. This is a working backlog, not a record of completed fixes. See [the roadmap](../ROADMAP.md) for sequencing.

## Review evidence

- Reviewed the Rust CLI, orchestration, dashboard source, Go TUI, documentation, GitHub workflows, and release setup.
- All 137 Rust tests passed with `cargo test --locked -q`.
- Go tests passed with `go test -race ./...` from `tui/`.
- `cargo fmt --check` failed.
- No paid model calls or visual browser testing were performed. Findings below are based on source inspection unless stated otherwise.
- GitHub showed no declared license and no attached binary assets for v0.4.1.

## Suggestion [Cleanup] ✨ — Correctness and reliability

### FIX-01 · P1 · Preserve dissent after convergence

- [x] Retain the judge's disagreements independently of its convergence score.
- [x] Generate the dissent document even when enough agreement exists to stop.
- [x] Carry hollow-consensus warnings into final dissent; remove the hardcoded unanimity assertion.

**Implementation:** the final dissent pass now runs for both converged and divergent forums, with regression coverage for minority objections and hollow-consensus warnings. This adds one model call to converged forums. Generated text still requires model-quality evaluation; the code no longer asserts unanimity from a score.

**Evidence:** `src/convergence.rs::parse_judge_response` discards disagreements on the converged branch. `src/protocol.rs::write_final_output` then writes “No unresolved disagreements.”

**Done when:** a converged fixture containing a minority objection preserves that objection in the final result. Agreement sufficient to stop does not imply unanimity.

### FIX-02 · P1 · Enforce internal model timeouts

- [x] Route default Claude calls through a bounded process runner.
- [x] Cancel timed-out invocations and clean up their process groups.
- [x] Identify the failed model/command in errors.

**Implementation:** default Claude and custom commands share the deadline runner. Tests cover a stalled process, descendant cleanup, zero deadlines, literal arguments, and error details. User-initiated whole-forum cancellation remains part of milestone 0.6.

**Evidence:** `src/substrate.rs::invoke_fire_keeper_model` accepts a timeout, but its `invoke_claude` fallback uses an unbounded `.output()` call.

**Done when:** a deliberately stalled fake model is terminated within the configured deadline and leaves no child process running.

### FIX-03 · P1 · Give each participant an explicit identity

- [x] Add participant-specific identity and instructions for selecting their critique assignment.
- [x] Support aliases and multiple participants using the same underlying model.

**Implementation:** personalized inputs are saved under `round-N/prompts/<name>.md`, sent to command participants, and linked in human-participant instructions. A regression exercises two aliases sharing the same command plus a human participant.

**Evidence:** `src/protocol.rs::invoke_participants` sends everyone the same prompt; the cross-examination prompt asks each to “Find YOUR name.”

**Done when:** captured prompts unambiguously identify each participant and its target, including two aliases of the same model.

### FIX-04 · P1 · Wire dashboard lifecycle events

- [x] Emit forum-start, round-start, and participant-response events during execution.
- [x] Provide initial metadata through events.
- [x] Verify reconnect/replay produces the same state as observing the run live.

**Implementation:** a fake-model forum regression validates emitted events against the schema; browser reducer tests cover log replay and snapshot prefixes. Manual response notifications occur as files are collected. Response-write errors propagate before an event can claim that a command participant responded.

**Evidence:** `src/protocol.rs` emits only part of the existing event contract. The runtime does not write the snapshot that could otherwise initialize missing metadata.

**Done when:** a fresh forum displays its topic, participants, round, and response progress without handcrafted fixtures.

### FIX-05 · P1 · Reject invalid judgments

- [x] Reject missing, malformed, non-finite, and out-of-range convergence scores.
- [x] Validate alignment scores as well.
- [x] Use bounded retries and an explicit unavailable/error state instead of invented scores.

**Implementation:** both judgment types retry once and then report evaluation unavailable. Convergence failure aborts the run; alignment failure warns without writing fabricated values. Duplicate scores and missing/unknown alignment participants are also rejected.

**Evidence:** `src/convergence.rs` defaults malformed or missing convergence scores to 5 and accepts finite scores outside 1–10; alignment parsing also supplies default scores.

**Done when:** malformed model output cannot silently become a valid convergence decision or alignment measurement.

### FIX-06 · P2 · Make completion explicit and durable

- [x] Persist running/completed/failed outcomes and recognize a dead runner as interrupted.
- [x] Mark completion only after all required final artifacts succeed.
- [x] Make `status`, `list`, `result`, and browser dashboard use the run outcome.

**Implementation:** `run-status.json` is atomically written and synced. Finalization failure records the error and emits `forum_failed`; terminal and HTML result commands reject incomplete runs. The browser checks the outcome on replay and heartbeat. Unix dead-PID detection reports interrupted runs; it is a local-process check, not a portable session lease (PID reuse and copied sessions remain limitations). The TUI consumes failure events; dead-runner detection is currently in the Rust CLI/browser path.

**Evidence:** `src/substrate.rs::is_completed` checks for `final/synthesis.md`, but `write_final_output` writes it before dissent and metadata finish.

**Done when:** an injected finalization failure cannot appear as a completed forum.

### FIX-07 · P2 · Protect participant records

- [x] Propagate response-write failures instead of ignoring them.
- [x] Reject duplicate participant names before creating a forum or starting commands.

**Implementation:** config validation rejects duplicate names, and the protocol also validates direct callers. A regression forces response persistence to fail and verifies that the round aborts without emitting a response event.

**Evidence:** `src/protocol.rs::invoke_participants` ignores `write_atomic` errors. `src/main.rs::cmd_new` accumulates duplicate names while overwriting their configuration-map entries.

**Done when:** duplicate names fail early and a failed response write cannot be reported as successfully persisted participation.

### FIX-08 · P2 · Fix Unicode topic truncation

- [x] Truncate display text at character boundaries.
- [x] Add emoji and non-Latin regression cases.

**Implementation:** topic lists and preset-command lists use the same UTF-8-safe abbreviation helper. Regression cases cover ASCII boundaries, emoji, non-Latin text, and mixed multibyte strings. Width is measured in Unicode scalar values, not terminal display columns.

**Evidence:** `src/main.rs::cmd_list` slices topics with `&topic[..32]`.

**Done when:** listing long Unicode topics never panics.

### FIX-09 · P2 · Make configuration promises accurate

- [ ] Implement or remove unused `quorum`, `late_policy`, convergence `policy`, and `max_prior_context` settings.
- [ ] Reject unsupported values rather than accepting ineffective configuration.
- [ ] Make `--max-rounds` a hard ceiling; expose any automatic extension separately.

**Done when:** documented settings have observable, tested behavior and users can reliably cap a run.

### FIX-10 · P2 · Make reports portable

- [x] Bundle required rendering assets or render HTML during export so reports work offline.
- [x] Replace the old `Abeansits/agora` footer link.

**Implementation:** forum and evaluation exports embed pinned Marked/DOMPurify builds and upstream notices. A content-security policy blocks automatic network resources; model Markdown images/media/styles are removed. Vendor archives are integrity-checked and file hashes recorded. Chrome smoke checks confirm rendered Markdown and dissent without external script tags.

**Evidence:** `src/report.rs` loads Markdown rendering and sanitization libraries from a CDN despite the README's self-contained report claim.

**Done when:** an exported report renders correctly without network access and links to Ting.

## Suggestion [Improvement] ✨ — Open-source welcome

### OSS-01 · License and package identity

- [ ] Choose and add a license.
- [ ] Add matching Cargo license metadata and useful package description/repository metadata.

**Done when:** visitors and package consumers can clearly identify the project's reuse terms.

### OSS-02 · Working installation and releases

- [x] Fix the source-install quick start: clone, enter the repository, then `cargo install --path . --locked`.
- [ ] Publish macOS and Linux binaries with checksums.
- [ ] Document supported platforms and the tested minimum Rust version.

**Evidence:** `cargo build --release` in the current quick start does not put `ting` on `PATH`; v0.4.1 has no attached binaries.

**Done when:** a newcomer can follow the documented installation on a clean supported system.

### OSS-03 · No-account demo

- [x] Bundle a safe, representative sample forum.
- [x] Add `ting demo` to explore it without model credentials.
- [x] Include synthesis, meaningful dissent, and changes after cross-examination.

**Implementation:** the embedded, hand-written sample is labeled illustrative and creates a standard session. It prints synthesis/dissent and serves the dashboard, or exits with `--no-serve`. Smoke-tested with an empty executable PATH; schema and overwrite tests run in the Rust suite.

**Done when:** visitors can experience the product before installing or authenticating participant CLIs.

### OSS-04 · README and examples

- [x] Lead with one sentence, a screenshot or short recording, installation, demo, and first real run.
- [x] Move detailed architecture and event-contract explanations into linked docs.
- [x] Document `eval` and review output with examples.
- [x] Reconcile defaults, preset commands, and dashboard claims with the current CLI.

**Implementation:** a concise README uses an actual demo screenshot and links to a CLI reference, architecture guide, and generated sample output. Sample data is explicitly illustrative. Current auto-extension, reserved config fields, and binary-only doctor checks are documented rather than implied to be stronger guarantees.

**Done when:** the first screen explains what Ting does and gives a clear route to trying it.

### OSS-05 · Contributor entry points

- [x] Add `CONTRIBUTING.md` with setup, architecture pointers, and validation commands.
- [x] Add issue and PR templates plus support/security reporting guidance.
- [x] Identify small, independently actionable good-first-issue tasks ([#23](https://github.com/Abeansits/ting/issues/23), [#24](https://github.com/Abeansits/ting/issues/24)).

**Done when:** a new contributor can find a task, make a change, and validate it without undocumented maintainer knowledge.

### OSS-06 · CI and toolchain checks

- [x] Apply formatting and enforce `cargo fmt --check` in CI.
- [x] Add Clippy and a declared/tested minimum Rust version.
- [x] Test supported platforms (Linux/macOS matrix passed in #21).
- [x] Add meaningful regression coverage for the reliability findings above.

**Implementation:** all 152 Rust tests pass locally on Rust 1.88.0. Style checks use Rust 1.92.0; formatting and Clippy pass. CI includes Linux minimum/stable Rust, macOS stable Rust, Node reducer tests, and Go race tests on both platforms. Workflows also run for stacked PRs.

**Done when:** the documented local checks match CI and platform/toolchain promises are exercised.

### OSS-07 · Execution, privacy, and cost clarity

- [x] Document actual CLI commands, working directories, and execution permissions.
- [x] Explain what context is sent to providers and what publishing exposes.
- [x] Identify the internal model operations and which optional features add calls.
- [x] Add `ting doctor` to check prerequisites before a paid run.

**Implementation:** `docs/EXECUTION.md` documents commands, privacy, and call overhead. Doctor checks known executable files without running them; custom shell commands and authentication are explicitly left for manual inspection. JSON output and missing-binary exit behavior are smoke-tested.

**Done when:** users can understand the execution model and resolve common setup problems before starting deliberation.

### OSS-08 · Read the discussion in the dashboard

- [ ] Display actual synthesis and participant response text.
- [ ] Show failures, interruption, and stop reasons clearly.
- [ ] Reflect the configured convergence threshold rather than a hardcoded value.

**Evidence:** `src/static/dashboard.js` currently displays synthesis word counts and file-location guidance; its convergence gauge uses a hardcoded threshold of 7.

**Done when:** users can follow and understand a forum without returning to session files.

## Suggested starting order

1. FIX-01: preserve dissent, Ting's central promise.
2. FIX-02: bound internal model execution.
3. OSS-03: make the experience accessible without credentials, alongside the dashboard event fixes it needs.

License selection and quick-start corrections are small enough to handle alongside these work items.
