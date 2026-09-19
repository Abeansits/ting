# Ting roadmap

Working proposal · 2026-09-18

Direction: trustworthy deliberation, an easy first run, then evidence-backed adaptive orchestration. Versions below are proposed milestones, not release commitments.

The [project review and fix backlog](docs/PROJECT-REVIEW.md) contains evidence, work-item IDs, and completion criteria.

## Suggestion [Simplification] ✨ — Release sequence

### 0.4.2 · Trustworthy runs

- [x] Preserve dissent after convergence (FIX-01; implemented, pending PR merge).
- [x] Bound internal model calls and clean up processes (FIX-02; implemented, pending PR merge).
- [x] Supply participant identities and critique targets (FIX-03; implemented, pending PR merge).
- [ ] Emit dashboard lifecycle events (FIX-04).
- [ ] Validate judgments and represent unavailable evaluations honestly (FIX-05).
- [ ] Correct completion status and response persistence (FIX-06, FIX-07).
- [ ] Fix Unicode topic truncation (FIX-08).

**Release gate:** failure cases produce honest status, and completed results preserve minority objections.

### 0.5 · Public welcome

- [ ] License and complete package metadata (OSS-01).
- [ ] Working installation instructions and downloadable binaries (OSS-02).
- [ ] No-account `ting demo` with a representative sample forum (OSS-03).
- [ ] Concise README, visuals, and real output examples (OSS-04).
- [ ] Contributor guide, templates, and starter issues (OSS-05).
- [ ] Formatting, linting, toolchain, and platform checks (OSS-06).
- [ ] Execution/cost documentation and `ting doctor` (OSS-07).
- [ ] Offline-capable HTML reports (FIX-10).

**Release gate:** a newcomer can explore Ting without credentials and diagnose setup before a paid run.

### 0.6 · Recovery and control

- [ ] Add `ting resume` with checkpoints for completed operations.
- [ ] Persist execution options needed for reproducible recovery.
- [ ] Prevent stale scores from being reused after responses or rubrics change.
- [ ] Add cancellation and distinct stop reasons: converged, stalled, budget exhausted, failed, interrupted.
- [ ] Enforce hard round limits and accurate configuration semantics (FIX-09).
- [ ] Display actual discussion content and configured thresholds in the dashboard (OSS-08).

**Release gate:** an interrupted forum resumes without repeating completed work or mixing old evaluations with new evidence.

### 0.7 · Optional Jev evaluation backend

- [ ] Add descriptive rubric levels to metric definitions.
- [ ] Introduce a small evaluation-provider interface, preserving the existing generative provider.
- [ ] Implement Jev metric scoring and recorded comparison runs.
- [ ] Evaluate participant alignment scoring.
- [ ] Run convergence judgments in comparison mode before allowing Jev to stop a forum.
- [ ] Persist model/rubric versions, raw judgments, latency, usage, and fallback outcomes.
- [ ] Test provider timeouts, malformed responses, and unavailable service behavior.

**Release gate:** representative evaluations support the quality/cost tradeoff, and provider failures have tested fallback behavior.

### Later · Adaptive deliberation

- [ ] Evaluate advisory “another round?” judgments before enabling automatic early exit.
- [ ] Experiment with targeted cross-examination while preserving assignment constraints.
- [ ] Expand human-reviewed evaluation datasets and compare against simpler protocols.

**Release gate:** adaptation improves useful outcomes on representative tasks, rather than merely producing higher agreement scores.

## Suggestion [Improvement] ✨ — Jev integration design

### Keep generation and evaluation separate

Keep generative models responsible for synthesis, claims, dissent explanations, and rubric invention. Use Jev for bounded judgments where structured output is useful. Jev remains optional.

### Recommended implementation order

1. **Per-round metric scoring.** This is already a batched operation whose failures do not abort a forum. Extend `metrics.json` with concrete rubric levels, freeze those definitions for the run, and score them together against shared state.
2. **Participant alignment.** Ting currently makes a separate generative call each round for alignment. Compare batched Scores against that behavior.
3. **Convergence comparison mode.** Save Jev and current-judge results side by side without changing stop decisions. Include examples of superficial agreement, unresolved minority objections, and incomplete participation.
4. **Another-round advice.** Judge whether further revision is likely to add useful information. Initially show the signal without acting on it. A stopped discussion must not automatically be labeled converged.
5. **Semantic cross-exam pairing.** Only pursue this if it improves critiques. The existing shuffled cycle already guarantees coverage and avoids self-pairing; independent Choice answers do not enforce those global constraints. Keep assignment validation in code.

### Rubrics and score mapping

- Jev Scores use zero-based positions in ordered descriptive levels. Ting currently uses 1–10 scores.
- With ten descriptive levels, `ting_score = jev_score + 1` preserves the display range. This does not prove the old stopping threshold has the same quality.
- Use concrete, independently understandable descriptions for levels; numeric labels alone are insufficient.
- Keep questions, thresholds, and rubric versions easy to review together.
- Preserve raw probability distributions alongside display scores.
- Independent questions sharing the same state can be batched; they cannot depend on one another's answers within that batch.

### Evaluation and rollout

- [ ] Assemble saved, human-reviewed forums covering agreement, substantive dissent, stalled discussion, missing responses, and misleading synthesis.
- [ ] Measure premature stopping and missed dissent, alongside score differences, latency, and usage.
- [ ] Treat existing Claude judgments as a comparison baseline, not ground truth.
- [ ] Validate probabilities on Ting's tasks; typed output and high confidence do not establish correctness.
- [ ] Define input-size handling explicitly. If excerpts are used, retain minority evidence and record truncation.
- [ ] Pin or record the resolved model and rubric version for reproducibility.
- [ ] Keep API credentials outside saved forum artifacts; make the additional provider/data transfer explicit.
- [ ] Define fallback policy per operation: optional scoring can remain unavailable; control decisions need a documented safe fallback.

No latency, cost, calibration, or quality improvement has been measured for Ting yet.

### Sources consulted

- [TypeSafe agent skill](https://github.com/typesafe-ai/skills/blob/main/skills/typesafe-ai/SKILL.md)
- [Agent skill documentation](https://docs.typesafe.ai/agent-skill#other-agents)
- [TypeSafe introduction and primitives](https://docs.typesafe.ai/introduction)
- [Score semantics and rubric guidance](https://docs.typesafe.ai/primitives/score)

These were reviewed on 2026-09-18. Recheck the live API contract and relevant cookbook before implementation. Reading the skill for this review did not install it in the repository.

## Suggestion [Simplification] ✨ — Product and maintenance boundaries

- Make the browser dashboard the primary newcomer experience.
- Keep the Go TUI optional and consuming the same tested filesystem contract.
- Keep deterministic rules, assignment constraints, limits, and workflow transitions in code.
- Prefer small provider boundaries over a broad framework refactor.
- Preserve dissent and evidence even when the workflow decides to stop.
