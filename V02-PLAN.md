# Ting v0.2 Implementation Plan

> Historical planning notes. Current work and completion evidence live in
> [ROADMAP.md](ROADMAP.md) and [the project review](docs/PROJECT-REVIEW.md).

Based on 3 Ting forum sessions (v0.2 roadmap, divergence mode design, posting strategy) + Sebastian's additions.

## Phase 1: Foundation (do first — everything else depends on this)

### 1.1 Fix Gemini Preset
- **What**: Gemini keeps failing (`--prompt` expects an argument, stdin handling broken)
- **How**: Test `echo "prompt" | gemini -p " "` vs `cat file | gemini -p "$(cat file)"` vs piping. Find what actually works reliably.
- **Done when**: Gemini participates in 3 consecutive forums without failure
- **Effort**: 1-2 hours

### 1.2 "Bring Your Own Model" (BYOM) Documentation + Validation
- **What**: Document the generic script participant pattern. Any stdin→stdout script is a participant.
- **How**:
  - Add `docs/bring-your-own-model.md` with examples:
    - API wrapper (curl + jq)
    - Local model wrapper (ollama)
    - Filtered wrapper (prepend system prompt, pipe to any CLI)
    - Python wrapper (langchain, litellm, etc.)
  - Add `ting preset add <name> <command>` for user-defined persistent presets (stored in `~/.ting/config.toml`)
  - Add `ting preset list` to show all built-in + custom presets
  - Validate on startup: check if preset command exists on PATH, warn if not
- **Done when**: A new user can add their own model in < 2 minutes
- **Effort**: 3-4 hours

### 1.3 New Presets: Cursor + Pi
- **What**: Add built-in presets for Cursor CLI and Pi
- **How**:
  - Cursor: test `cursor` CLI for non-interactive mode flags
  - Pi: test `pi` CLI or identify the right invocation
  - Add to preset registry in `config.rs`
- **Depends on**: 1.2 (preset infrastructure)
- **Effort**: 1-2 hours

---

## Phase 2: Evaluation Harness (the existential question)

### 2.1 A/B Test Framework
- **What**: `ting eval` command that runs the same question through Ting AND a single model, then compares
- **How**:
  - `ting eval "question" --baseline claude --forum codex,gemini,claude,opencode`
  - Baseline: run question through single model, save response
  - Forum: run normal Ting forum, save synthesis
  - Output both side-by-side in an eval report
  - Store results in `~/.ting/evals/{eval-id}/`
- **Schema**:
  ```
  evals/{eval-id}/
    meta.toml          # question, baseline model, forum participants, timestamps
    baseline.md        # single-model response
    forum/             # symlink or copy of forum session
    comparison.md      # generated side-by-side analysis
    scores.toml        # blind scoring results
  ```
- **Done when**: `ting eval` runs end-to-end and produces a comparison report
- **Effort**: 6-8 hours

### 2.2 Blind Scoring
- **What**: Have a judge model score both outputs without knowing which is Ting vs baseline
- **How**:
  - Randomize order (A/B, not always baseline-first)
  - Judge scores on: completeness, counterarguments surfaced, actionability, blind spots identified
  - Output structured scores to `scores.toml`
  - Use different model family as judge (e.g., if forum uses Claude+Codex, judge with Gemini)
- **Depends on**: 2.1
- **Effort**: 3-4 hours

### 2.3 Structured Event Logging
- **What**: Append-only JSONL event log for every forum action (the implicit control plane)
- **How**:
  - `~/.ting/events.jsonl` — global event log
  - Events: `forum_created`, `round_started`, `participant_invoked`, `response_received`, `participant_failed`, `synthesis_generated`, `convergence_checked`, `forum_completed`
  - Each event: timestamp, forum_id, round, participant, duration, token estimate
  - `ting stats` command to query the log (total forums, avg convergence score, participant reliability, timing)
- **Why now**: The eval harness needs this data. Building it separately later means rebuilding half the harness.
- **Depends on**: Nothing — can start immediately
- **Effort**: 4-5 hours

---

## Phase 3: Divergence Mode

### 3.1 Protocol Trait Refactor
- **What**: Extract Modified Delphi into a `Protocol` trait so divergence can plug in
- **How**:
  - `trait Protocol { fn rounds(&self) -> Vec<RoundType>; fn synthesize(...); fn check_completion(...); }`
  - `DelphiCrossExam` implements convergence-seeking behavior
  - `Diverge` implements divergence-seeking behavior
  - `--protocol delphi` (default) or `--protocol diverge`
  - Fire keeper delegates round logic to the protocol implementation
- **Done when**: Existing Delphi works unchanged through the trait, and a stub diverge protocol compiles
- **Effort**: 4-5 hours

### 3.2 Diverge Protocol Implementation
- **What**: Four-round divergence protocol (from forum design session)
- **Rounds**:
  1. **Explore** — blind proposals with assigned lenses (user picks lens pack or uses default)
  2. **Cross-Pollinate** — participants see anonymized idea cards from others, extend/invert/combine
  3. **Expand** — participants see cluster map, deepen one cluster + open one adjacent frontier
  4. **Harvest** — nominate frontiers, flag what deserves deeper exploration
- **Output artifacts**:
  - `ideas.toml` — structured idea cards with lens, category, novelty estimate
  - `clusters.toml` — idea families with bridges between them
  - `fragments.md` — half-formed sparks (first-class, not discarded)
  - `frontiers.md` — what deserves follow-up (Delphi forum, research, prototype)
- **Lens packs** (built-in):
  - `default`: Optimist, Critic, Pragmatist, Wildcard
  - `sixhats`: White (facts), Red (intuition), Black (caution), Yellow (benefits), Green (creative), Blue (process)
  - `startup`: Customer, Engineer, Investor, Competitor
  - Custom via `--lenses "role1,role2,role3"`
- **Completion check**: Saturation metric (are new ideas still appearing?) replaces convergence
- **Depends on**: 3.1
- **Effort**: 8-10 hours

---

## Phase 4: Polish for Public Launch

### 4.1 Run 5 A/B Tests
- **What**: The proof content for posting
- **Test cases** (from forum recommendations):
  1. **Code review**: PR diff — single model vs Ting (measure: bugs found, false positives)
  2. **Architecture decision**: "JSONL vs SQLite" — already done, document formally
  3. **Planning**: "v0.2 roadmap" — already done, document formally
  4. **Creative**: "Name this tool" — run diverge mode, compare to single-model brainstorm
  5. **Analysis**: Real-world problem (pick from HN "Ask HN" or industry decision)
- **Output**: Blog-ready comparison for each, with blind scores
- **Depends on**: 2.1, 2.2
- **Effort**: 4-5 hours (mostly waiting for forums to run)

### 4.2 HTML Report Polish
- **What**: Make the HTML report demo-worthy
- **How**: Clean up styling, ensure markdown renders perfectly, add convergence score visualization, make it look good in screenshots
- **Effort**: 2-3 hours

### 4.3 README + Landing Content
- **What**: README that sells, not just documents
- **How**: Hook → demo GIF → install → quick start → "how it works" visual → example output
- **Depends on**: 4.1 (need real results to showcase)
- **Effort**: 2-3 hours

### 4.4 Posting Sequence
- **Post 1**: Observation post — "I noticed something: when I ask 4 AI models the same question independently, they disagree 60% of the time. The disagreements are more valuable than the agreements."
- **Post 2**: The proof — side-by-side A/B result. "Left: Claude alone. Right: 4 models debating. Spot the difference." Link to HTML report.
- **Post 3**: The self-review story — "I had the tool review its own code. It found 11 real bugs." Screenshot of the findings.
- **Post 4**: Repo drop — "Open source. Rust. cargo install. Works with any AI model." Link to GitHub.
- **Post 5+**: Numbered series — `Ting Session #N` with interesting forum results

---

## Sequencing Summary

```
Phase 1 (1 week)     Phase 2 (1-2 weeks)     Phase 3 (1-2 weeks)     Phase 4 (1 week)
├─ Fix Gemini        ├─ Event logging         ├─ Protocol trait       ├─ Run A/B tests
├─ BYOM docs         ├─ A/B framework         ├─ Diverge protocol     ├─ Polish HTML
├─ Cursor/Pi         ├─ Blind scoring         ├─ Lens packs           ├─ README
│                    │                        ├─ Saturation metric    ├─ Post sequence
```

Phases 1-2 can partially overlap. Phase 3 is independent of Phase 2. Phase 4 needs 1+2 done.

## Delegation Plan

| Task | Who | Why |
|------|-----|-----|
| 1.1 Gemini fix | ting-v01 session | Quick code fix |
| 1.2 BYOM docs | ting-v01 session | Knows the codebase |
| 1.3 New presets | ting-v01 session | Trivial addition |
| 2.3 Event logging | ting-v01 session | Infrastructure code |
| 2.1-2.2 Eval harness | New session | Fresh context, complex feature |
| 3.1-3.2 Diverge mode | New session | Major feature, fresh context |
| 4.1 A/B tests | V0 (conductor) | Orchestration, not coding |
| 4.2-4.3 Polish | ting-v01 session | Knows the HTML template |
| 4.4 Posting | Sebastian + V0 | Creative, needs human voice |
