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

- [ ] Reject missing, malformed, non-finite, and out-of-range convergence scores.
- [ ] Validate alignment scores as well.
- [ ] Use bounded retries and an explicit unavailable/error state instead of invented scores.

**Evidence:** `src/convergence.rs` defaults malformed or missing convergence scores to 5 and accepts finite scores outside 1–10; alignment parsing also supplies default scores.

**Done when:** malformed model output cannot silently become a valid convergence decision or alignment measurement.

### FIX-06 · P2 · Make completion explicit and durable

- [ ] Persist running, completed, failed, and interrupted states.
- [ ] Mark completion only after all required final artifacts succeed.
- [ ] Make `status`, `list`, `result`, and dashboard state agree.

**Evidence:** `src/substrate.rs::is_completed` checks for `final/synthesis.md`, but `write_final_output` writes it before dissent and metadata finish.

**Done when:** an injected finalization failure cannot appear as a completed forum.

### FIX-07 · P2 · Protect participant records

- [ ] Propagate response-write failures instead of ignoring them.
- [ ] Reject duplicate participant names before creating a forum or starting commands.

**Evidence:** `src/protocol.rs::invoke_participants` ignores `write_atomic` errors. `src/main.rs::cmd_new` accumulates duplicate names while overwriting their configuration-map entries.

**Done when:** duplicate names fail early and a failed response write cannot be reported as successfully persisted participation.

### FIX-08 · P2 · Fix Unicode topic truncation

- [ ] Truncate display text at character boundaries.
- [ ] Add emoji and non-Latin regression cases.

**Evidence:** `src/main.rs::cmd_list` slices topics with `&topic[..32]`.

**Done when:** listing long Unicode topics never panics.

### FIX-09 · P2 · Make configuration promises accurate

- [ ] Implement or remove unused `quorum`, `late_policy`, convergence `policy`, and `max_prior_context` settings.
- [ ] Reject unsupported values rather than accepting ineffective configuration.
- [ ] Make `--max-rounds` a hard ceiling; expose any automatic extension separately.

**Done when:** documented settings have observable, tested behavior and users can reliably cap a run.

### FIX-10 · P2 · Make reports portable

- [ ] Bundle required rendering assets or render HTML during export so reports work offline.
- [ ] Replace the old `Abeansits/agora` footer link.

**Evidence:** `src/report.rs` loads Markdown rendering and sanitization libraries from a CDN despite the README's self-contained report claim.

**Done when:** an exported report renders correctly without network access and links to Ting.

## Suggestion [Improvement] ✨ — Open-source welcome

### OSS-01 · License and package identity

- [ ] Choose and add a license.
- [ ] Add matching Cargo license metadata and useful package description/repository metadata.

**Done when:** visitors and package consumers can clearly identify the project's reuse terms.

### OSS-02 · Working installation and releases

- [ ] Fix the source-install quick start: clone, enter the repository, then `cargo install --path . --locked`.
- [ ] Publish macOS and Linux binaries with checksums.
- [ ] Document supported platforms and the tested minimum Rust version.

**Evidence:** `cargo build --release` in the current quick start does not put `ting` on `PATH`; v0.4.1 has no attached binaries.

**Done when:** a newcomer can follow the documented installation on a clean supported system.

### OSS-03 · No-account demo

- [ ] Bundle a safe, representative sample forum.
- [ ] Add `ting demo` to explore it without model credentials.
- [ ] Include synthesis, meaningful dissent, and changes after cross-examination.

**Done when:** visitors can experience the product before installing or authenticating participant CLIs.

### OSS-04 · README and examples

- [ ] Lead with one sentence, a screenshot or short recording, installation, demo, and first real run.
- [ ] Move detailed architecture and event-contract explanations into linked docs.
- [ ] Document `eval` and review output with examples.
- [ ] Reconcile defaults, preset commands, and dashboard claims with the current CLI.

**Done when:** the first screen explains what Ting does and gives a clear route to trying it.

### OSS-05 · Contributor entry points

- [ ] Add `CONTRIBUTING.md` with setup, architecture pointers, and validation commands.
- [ ] Add issue and PR templates plus support/security reporting guidance.
- [ ] Identify small, independently actionable good-first-issue tasks.

**Done when:** a new contributor can find a task, make a change, and validate it without undocumented maintainer knowledge.

### OSS-06 · CI and toolchain checks

- [ ] Apply formatting and enforce `cargo fmt --check` in CI.
- [ ] Add Clippy and a declared/tested minimum Rust version.
- [ ] Test supported platforms.
- [ ] Add meaningful regression coverage for the reliability findings above.

**Done when:** the documented local checks match CI and platform/toolchain promises are exercised.

### OSS-07 · Execution, privacy, and cost clarity

- [ ] Document actual CLI commands, working directories, and execution permissions.
- [ ] Explain what context is sent to providers and what publishing exposes.
- [ ] Identify the internal model operations and which optional features add calls.
- [ ] Add `ting doctor` to check prerequisites before a paid run.

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
