# Ting — Multi-Agent Deliberation Tool

> Historical design document. Some proposed policies here were never implemented.
> For current behavior, use [the CLI reference](docs/CLI.md),
> [the event contract](schemas/CONTRACT.md), and [the roadmap](ROADMAP.md).

A substrate-independent deliberation tool where any agent (LLM, CLI tool, or human) can participate in structured, multi-turn discussions using the filesystem as a shared medium.

## Why

Single-model consultations (like midflight) improve quality. Multi-model, multi-turn deliberation improves it more — but nobody has built it without locking you into a framework. Ting's interface is the filesystem: read a file, write a file. That's it.

## Core Concepts

### Participants
Anything that can read and write files:
- Claude Code sessions (via `session send` or file polling)
- Codex, Gemini, GPT (via their respective CLIs)
- Local models (via ollama, llama.cpp)
- Humans (write markdown in your editor)
- Shell scripts, Python scripts, any process

Participants don't know they're in Ting. They see a prompt, they respond.

### Fire Keeper
A standalone Rust binary that orchestrates the forum:
- Writes prompts for each round
- Watches for participant responses (filesystem events)
- Generates synthesis after each round
- Runs convergence checks
- Produces final output

### Substrate
The filesystem. Forum state is a directory tree of markdown and TOML files. Human-readable, inspectable, debuggable. No database required for v0.1, though a SQLite control plane may be added later for state machine correctness.

## Protocol: Modified Delphi

Three-stage rounds, adapted from the Delphi Method with an adversarial cross-examination step (informed by multi-agent debate research).

### Stage Flow

```
SETUP
  Fire keeper creates forum directory and meta.toml
  Fire keeper writes round-1/prompt.md

ROUND 1: INDEPENDENT PROPOSALS (blind)
  Each participant reads prompt.md
  Each writes their response: round-1/{participant}.md
  No participant sees any other's response
  Fire keeper collects all, generates:
    - round-1/synthesis.md (narrative summary)
    - round-1/claims.toml (structured claims with stance per participant)

ROUND 2: CROSS-EXAMINATION (adversarial)
  Fire keeper assigns each participant one other to critique
  Writes round-2/prompt.md with all Round 1 responses + assignments
  Each participant must:
    1. Critique their assigned response (find weaknesses, contradictions)
    2. Defend or revise their own position
  Fire keeper generates synthesis + updated claims

ROUND 3: INFORMED REVISION
  Fire keeper writes round-3/prompt.md with:
    - Original question
    - All Round 2 critiques and defenses
    - Current synthesis + claims
  Each participant writes final revised position
  Fire keeper runs convergence check

CONVERGENCE CHECK
  If converged → write final/synthesis.md + final/claims.toml
  If divergent → optional Round 4+ (revision only) up to max_rounds
  Always write final/dissent.md for positions that never converged

FINAL OUTPUT
  final/synthesis.md    — narrative synthesis for humans
  final/claims.toml     — structured claims, evidence, stances
  final/dissent.md      — unresolved disagreements (first-class, not optional)
```

### Why Three Stages?

Pure Delphi (propose → revise) produces "polite agreement" with LLMs — they converge on shared biases, not truth. The cross-examination round forces adversarial thinking before revision, surfacing genuine disagreements.

Validated empirically: two blind Codex consultations on Ting's own design produced 40% convergence, 30% partial, 30% genuine divergence. The divergences were the most valuable insights.

## Directory Structure

```
~/.ting/sessions/{forum-id}/
  meta.toml                    # forum config
  round-1/
    prompt.md                  # fire keeper → participants
    {participant-a}.md         # participant responses
    {participant-b}.md
    synthesis.md               # fire keeper generated
    claims.toml                # structured claims
  round-2/
    prompt.md                  # includes cross-exam assignments
    {participant-a}.md
    {participant-b}.md
    synthesis.md
    claims.toml
  round-3/
    ...
  final/
    synthesis.md               # final narrative
    claims.toml                # final structured claims
    dissent.md                 # unresolved positions
    meta-summary.toml          # stats: rounds, convergence scores, timing
```

## Configuration

### meta.toml

```toml
[forum]
id = "ting-2026-03-27-001"
topic = "Should we use Pipecat or Vapi for voice?"
created = "2026-03-27T00:30:00Z"
max_rounds = 5
protocol = "delphi-crossexam"   # extensible

[participants]
names = ["v0", "codex", "gemini"]

[participants.v0]
type = "command"                 # or "file-poll", "manual"
command = "agent-deck session send conductor-ops '{prompt}' --wait -q"

[participants.codex]
type = "command"
command = "codex exec --full-auto -m '{prompt}'"

[participants.gemini]
type = "command"
command = "gemini '{prompt}'"

# Human participants use type = "manual" — they write files directly

[timing]
round_timeout = "5m"
participant_timeout = "2m"
quorum = 0                      # 0 = all required, N = proceed with N
late_policy = "include_next"    # "include_next" or "discard"

[convergence]
policy = "llm-judge"            # trait-based, swappable
judge_model = "claude-sonnet"   # model for convergence evaluation
threshold = 7                   # 1-10, above = converged
min_rounds = 2                  # never stop before this many rounds

[synthesis]
model = "claude-sonnet"         # model for generating synthesis
max_prior_context = 4000        # max tokens of prior rounds to include (prevents bloat)
```

## Participant Interface Contract

### Input
Read `round-N/prompt.md`. Contains:
- The original forum topic
- Round-specific instructions (propose / cross-examine / revise)
- Prior round context (summarized, not raw dumps — prevents prompt bloat)

### Output
Write `round-N/{participant-name}.md`. Contents:
- The participant's response in markdown
- Must be written atomically: write to `{name}.md.tmp`, rename to `{name}.md`

### Filename Contract
- Participant IDs: `[a-z0-9_-]+`, max 32 chars
- Reserved filenames: `prompt.md`, `synthesis.md`, `claims.toml`
- Max response size: 50KB (configurable)
- Encoding: UTF-8

## Convergence Detection

### Trait Interface

```rust
pub enum ConvergenceResult {
    Converged { score: f32, summary: String },
    Divergent { score: f32, key_disagreements: Vec<String> },
}

pub trait ConvergencePolicy: Send + Sync {
    fn evaluate(&self, round: &Round) -> Result<ConvergenceResult>;
}
```

### Built-in Policies

**LlmJudge (default):**
- Sends all participant responses to a judge model
- Asks: "Rate agreement 1-10. List remaining disagreements."
- Randomizes response order to prevent position bias
- Judge model should differ from participant models when possible

**ExplicitVote (planned):**
- After synthesis, each participant writes agree/disagree/modify
- Converged when all agree or quorum reached

**ClaimsDiff (planned):**
- Compare claims.toml across rounds
- Converged when claim stances stop changing

### Anti-Bias Measures
- Randomize participant order in prompts (prevent position bias)
- Use different model family for judge vs participants when possible
- Track claim stability across rounds, not just agreement score
- Preserve dissent as first-class output

## Timing & Synchronization

### Filesystem Watching
- `notify` crate for cross-platform events (FSEvents macOS, inotify Linux)
- Fire keeper watches round directory for expected filenames
- Atomic writes prevent partial-read issues

### Timeout Handling
```
participant writes response
    → within participant_timeout? → accept
    → exceeded? → mark as "late"
        → late_policy = "include_next"? → fold into next round's context
        → late_policy = "discard"? → exclude, note in synthesis

all expected responses received OR round_timeout hit
    → proceed to synthesis
    → note missing participants in claims.toml
```

### Quorum Mode
When `quorum > 0`, fire keeper proceeds when N participants have responded, without waiting for the rest. Late arrivals handled per `late_policy`.

## CLI Interface

```bash
# Create and start a forum
ting new "Should we use Pipecat or Vapi?" \
  --participant v0:command:"agent-deck session send conductor-ops '{prompt}' --wait -q" \
  --participant codex:command:"codex exec --full-auto -m '{prompt}'" \
  --participant human:manual \
  --timeout 5m

# Check status
ting status {forum-id}

# List all forums
ting list

# Read final synthesis
ting result {forum-id}

# Resume / add rounds to existing forum
ting continue {forum-id} --rounds 2

# Manually submit a response (for human participants)
ting respond {forum-id} --round 2 --participant human --file response.md
```

## v0.1 Scope

### In Scope
- [ ] Fire keeper binary (Rust, single binary)
- [ ] Modified Delphi protocol (propose → cross-exam → revise)
- [ ] Filesystem substrate with atomic writes
- [ ] `notify`-based file watching
- [ ] LLM judge convergence (single policy)
- [ ] Structured claims output (claims.toml)
- [ ] Dissent preservation
- [ ] Command-based and manual participant types
- [ ] CLI: `new`, `status`, `list`, `result`, `respond`
- [ ] Prompt summarization (prevent context bloat across rounds)
- [ ] Configurable timing: timeouts, quorum, late policy

### Out of Scope (v0.2+)
- **Divergence protocol** — "creativity mode" where the goal is breadth, not consensus. Inspired by De Bono's Six Thinking Hats. Fire keeper assigns lenses (risk, optimism, wild ideas, facts, etc.) to participants. No convergence check — output is the spread of perspectives, not agreement. Useful for brainstorming, creative work, retrospectives.
- **Additional CLI presets** — Cursor (`cursor`), Pi (`pi`), and others as they become available. Presets are cheap to add.
- **"Bring your own model" guide** — Document the generic script participant pattern. Any bash script that reads stdin and writes stdout is a valid participant. Include examples: API wrapper, local model wrapper, filtered/prompted wrapper. Make the custom command format a first-class documented feature, not a power-user escape hatch.
- SQLite control plane / event log
- Deterministic replay
- Security hardening (signed outputs, path sanitization, trust boundaries)
- Cost/token budgets per participant
- Additional convergence policies (ExplicitVote, ClaimsDiff)
- Evaluation harness with baselines
- Web UI / dashboard
- Remote participants (network transport)
- Fault injection testing

## Prior Art & References

### Academic
- Du et al. 2023 — "Improving Factuality and Reasoning in Language Models through Multiagent Debate"
- Irving et al. 2018 — "AI Safety via Debate"
- Zheng et al. 2023 — LLM judge bias and reliability
- Wang et al. 2023 — Self-Consistency (related convergence ideas)
- Yao et al. 2023 — Tree of Thoughts

### Systems
- Blackboard architectures (Hearsay-II, 1977) — shared knowledge store, multiple specialist agents
- KQML/FIPA ACL — early agent communication standards
- A2A / MCP — modern agent interop protocols (transport-level, not deliberation-level)
- Delphi Method — RAND Corporation, 1960s forecasting methodology

### What Ting Does Differently
- Substrate-independent (filesystem, not framework-locked)
- Any participant type (LLM, human, script — all first-class)
- Adversarial cross-examination built into the protocol
- Structured claim tracking alongside narrative synthesis
- Dissent as first-class output, not failure

## Architecture

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│ Participant  │     │ Participant  │     │ Participant  │
│ (Claude)     │     │ (Codex)      │     │ (Human)      │
└──────┬───────┘     └──────┬───────┘     └──────┬───────┘
       │ write               │ write               │ write
       ▼                     ▼                     ▼
┌──────────────────────────────────────────────────────────┐
│                    Filesystem Substrate                    │
│  sessions/{id}/round-N/{participant}.md                   │
└──────────────────────────┬───────────────────────────────┘
                           │ watch (notify)
                           ▼
                    ┌──────────────┐
                    │  Fire Keeper  │
                    │  (Rust CLI)   │
                    ├──────────────┤
                    │ • Orchestrate │
                    │ • Synthesize  │
                    │ • Converge    │
                    │ • Output      │
                    └──────────────┘
```

## Origin Story

Born from a conversation between Sebastian and Vigil (V0) on 2026-03-26. Started as "what if midflight but multi-turn?" and evolved through live Codex consultations that themselves demonstrated the value: two blind runs on the same design produced 40% convergence and 60% complementary divergence. The divergences contained the best insights.

The tool's first real test will be naming itself.
