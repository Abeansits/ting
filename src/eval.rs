use crate::{config, protocol, substrate, types::*};
use anyhow::{Context, Result};
use rand::seq::SliceRandom;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct EvalConfig {
    pub topic: String,
    pub context: Option<String>,
    pub baseline_preset: String,
    pub forum_presets: Vec<String>,
    pub judge_preset: String,
    pub timeout: String,
    pub max_rounds: u32,
}

#[allow(dead_code)]
pub struct EvalResult {
    pub eval_dir: PathBuf,
    pub baseline_first: bool, // true = baseline was Response A
    pub scores: Scores,
    pub comparison: String,
}

#[derive(Debug, Clone)]
pub struct Scores {
    pub baseline: ScoreSet,
    pub forum: ScoreSet,
}

#[derive(Debug, Clone)]
pub struct ScoreSet {
    pub completeness: f32,
    pub counterarguments: f32,
    pub actionability: f32,
    pub blind_spots: f32,
    pub precision: f32,
    pub overall: f32,
}

impl ScoreSet {
    fn avg(&self) -> f32 {
        (self.completeness + self.counterarguments + self.actionability
            + self.blind_spots + self.precision + self.overall) / 6.0
    }
}

pub fn evals_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".ting").join("evals")
}

/// Run a full eval: baseline response, forum deliberation, blind judge comparison
pub fn run_eval(cfg: &EvalConfig) -> Result<EvalResult> {
    let eval_id = format!(
        "eval-{}-{}",
        chrono::Utc::now().format("%Y-%m-%d"),
        &uuid::Uuid::new_v4().to_string()[..8],
    );
    let eval_dir = evals_dir().join(&eval_id);
    std::fs::create_dir_all(&eval_dir)?;

    // Write eval meta.toml with resolved model IDs
    let mut meta = format!(
        "[eval]\n\
         id = \"{eval_id}\"\n\
         topic = \"{topic}\"\n\
         baseline = \"{baseline}\"\n\
         forum = [{forum}]\n\
         judge = \"{judge}\"\n\
         created = \"{created}\"\n",
        eval_id = eval_id,
        topic = cfg.topic.replace('"', "\\\""),
        baseline = cfg.baseline_preset,
        forum = cfg.forum_presets.iter().map(|p| format!("\"{}\"", p)).collect::<Vec<_>>().join(", "),
        judge = cfg.judge_preset,
        created = chrono::Utc::now().to_rfc3339(),
    );
    meta.push_str("\n[models]\n");
    meta.push_str(&format!(
        "baseline = \"{}\"\n",
        config::resolve_model_id(&cfg.baseline_preset)
    ));
    meta.push_str(&format!(
        "judge = \"{}\"\n",
        config::resolve_model_id(&cfg.judge_preset)
    ));
    for p in &cfg.forum_presets {
        meta.push_str(&format!(
            "{} = \"{}\"\n",
            p,
            config::resolve_model_id(p)
        ));
    }
    std::fs::write(eval_dir.join("meta.toml"), &meta)?;

    // Step 1: Get baseline response
    eprintln!("\n=== Baseline: {} ===", cfg.baseline_preset);
    let baseline_response = run_baseline(cfg)?;
    substrate::write_atomic(&eval_dir.join("baseline.md"), &baseline_response)?;
    eprintln!("  Baseline response saved ({} chars)", baseline_response.len());

    // Step 2: Run forum
    eprintln!("\n=== Forum: {} ===", cfg.forum_presets.join(", "));
    let forum_config = build_forum_config(cfg)?;
    let forum_path = substrate::create_forum_dir(&forum_config.forum.id)?;
    config::save(&forum_config, &forum_path.join("meta.toml"))?;

    // Symlink forum into eval dir
    #[cfg(unix)]
    std::os::unix::fs::symlink(&forum_path, eval_dir.join("forum"))?;
    #[cfg(not(unix))]
    std::fs::write(eval_dir.join("forum.txt"), forum_path.display().to_string())?;

    protocol::run_forum(&forum_config, &forum_path, &protocol::RunOptions::default())?;

    // Read forum synthesis
    let forum_synthesis = substrate::read_file(&forum_path.join("final").join("synthesis.md"))
        .with_context(|| "Forum did not produce a final synthesis")?;

    // Step 3: Blind judge comparison
    eprintln!("\n=== Blind Judging: {} ===", cfg.judge_preset);
    let (baseline_first, comparison, scores) =
        blind_judge(cfg, &baseline_response, &forum_synthesis)?;

    // Write outputs
    substrate::write_atomic(&eval_dir.join("comparison.md"), &comparison)?;
    write_scores_toml(&eval_dir.join("scores.toml"), &scores, baseline_first, cfg)?;

    eprintln!("\n=== Eval complete: {} ===", eval_dir.display());
    eprintln!(
        "  Baseline avg: {:.1}  |  Forum avg: {:.1}",
        scores.baseline.avg(),
        scores.forum.avg(),
    );

    Ok(EvalResult {
        eval_dir,
        baseline_first,
        scores,
        comparison,
    })
}

fn run_baseline(cfg: &EvalConfig) -> Result<String> {
    let (_, cmd) = config::preset_command(&cfg.baseline_preset)
        .ok_or_else(|| anyhow::anyhow!("Unknown preset: {}", cfg.baseline_preset))?;

    let prompt = build_baseline_prompt(&cfg.topic, cfg.context.as_deref());
    let timeout = config::parse_duration(&cfg.timeout)?;

    eprintln!("  Invoking {}...", cfg.baseline_preset);
    substrate::invoke_command(&cmd, &prompt, timeout)
}

fn build_baseline_prompt(topic: &str, context: Option<&str>) -> String {
    let ctx = match context {
        Some(c) => format!("\n## Context\n\n{}\n", c),
        None => String::new(),
    };
    format!(
        "# Question\n\n\
         {topic}{ctx}\n\n\
         ## Instructions\n\n\
         Provide a thorough analysis of the topic above.\n\
         Consider multiple perspectives, tradeoffs, risks, and your specific recommendation.\n\
         Be concrete and actionable.\n\
         Write in clear, structured markdown.\n",
        topic = topic,
        ctx = ctx,
    )
}

fn build_forum_config(cfg: &EvalConfig) -> Result<ForumConfig> {
    let mut names = Vec::new();
    let mut configs: HashMap<String, ParticipantConfig> = HashMap::new();

    for preset in &cfg.forum_presets {
        let (name, pc) = config::parse_participant_spec(preset)?;
        names.push(name.clone());
        configs.insert(name, pc);
    }

    let forum_id = format!(
        "ting-{}-{}",
        chrono::Utc::now().format("%Y-%m-%d"),
        &uuid::Uuid::new_v4().to_string()[..8],
    );

    Ok(ForumConfig {
        forum: ForumSection {
            id: forum_id,
            topic: cfg.topic.clone(),
            created: chrono::Utc::now().to_rfc3339(),
            max_rounds: cfg.max_rounds,
            protocol: "delphi-crossexam".to_string(),
            context: cfg.context.clone(),
            output_format: None,
        },
        participants: ParticipantsSection { names, configs },
        timing: TimingSection {
            round_timeout: cfg.timeout.clone(),
            participant_timeout: cfg.timeout.clone(),
            quorum: 0,
            late_policy: "include_next".to_string(),
        },
        convergence: ConvergenceSection::default(),
        synthesis: SynthesisSection::default(),
    })
}

/// Run blind A/B comparison. Returns (baseline_was_A, comparison_text, scores).
fn blind_judge(
    cfg: &EvalConfig,
    baseline: &str,
    forum_synthesis: &str,
) -> Result<(bool, String, Scores)> {
    // Randomize assignment
    let mut rng = rand::thread_rng();
    let baseline_first = [true, false].choose(&mut rng).copied().unwrap();

    let (response_a, response_b) = if baseline_first {
        (baseline, forum_synthesis)
    } else {
        (forum_synthesis, baseline)
    };

    let prompt = build_judge_prompt(&cfg.topic, response_a, response_b);

    // Invoke judge
    let (_, judge_cmd) = config::preset_command(&cfg.judge_preset)
        .ok_or_else(|| anyhow::anyhow!("Unknown judge preset: {}", cfg.judge_preset))?;
    let timeout = config::parse_duration(&cfg.timeout)?;

    eprintln!("  Invoking judge: {}...", cfg.judge_preset);
    let judge_output = substrate::invoke_command(&judge_cmd, &prompt, timeout)?;

    // Parse scores
    let scores = parse_judge_scores(&judge_output, baseline_first)?;

    // Build comparison report
    let comparison = build_comparison_report(
        &cfg.topic,
        &cfg.baseline_preset,
        &cfg.forum_presets,
        baseline,
        forum_synthesis,
        baseline_first,
        &judge_output,
        &scores,
    );

    Ok((baseline_first, comparison, scores))
}

fn build_judge_prompt(topic: &str, response_a: &str, response_b: &str) -> String {
    format!(
        "You are a blind evaluator comparing two responses to the same question.\n\
         You do NOT know which response came from which system. Evaluate purely on quality.\n\n\
         ## Question\n{topic}\n\n\
         ## Response A\n{response_a}\n\n\
         ## Response B\n{response_b}\n\n\
         ---\n\n\
         Score each response on these dimensions (1-10 scale):\n\
         - **Completeness**: How thoroughly does it cover the topic?\n\
         - **Counterarguments**: Does it surface opposing views and tradeoffs?\n\
         - **Actionability**: How concrete and implementable are the recommendations?\n\
         - **Blind spots**: Does it identify risks, assumptions, or gaps?\n\
         - **Precision**: Of the issues identified, how many are genuine versus false positives, \
         overstated concerns, or misattributed problems? 10 = every finding is accurate and \
         correctly scoped. Penalize responses that report issues that aren't actually bugs or \
         overstate severity.\n\
         - **Overall**: Overall quality of analysis\n\n\
         Respond in EXACTLY this format:\n\
         SCORES_A: completeness=N counterarguments=N actionability=N blind_spots=N precision=N overall=N\n\
         SCORES_B: completeness=N counterarguments=N actionability=N blind_spots=N precision=N overall=N\n\
         REASONING: <your detailed comparison and reasoning>\n",
        topic = topic,
        response_a = response_a,
        response_b = response_b,
    )
}

pub fn parse_judge_scores(output: &str, baseline_first: bool) -> Result<Scores> {
    let scores_a = parse_score_line(output, "SCORES_A:");
    let scores_b = parse_score_line(output, "SCORES_B:");

    let (baseline, forum) = if baseline_first {
        (scores_a, scores_b)
    } else {
        (scores_b, scores_a)
    };

    Ok(Scores { baseline, forum })
}

fn parse_score_line(output: &str, prefix: &str) -> ScoreSet {
    let line = output
        .lines()
        .find(|l| l.trim().starts_with(prefix))
        .unwrap_or("");

    let get = |key: &str| -> f32 {
        line.split_whitespace()
            .find(|s| s.starts_with(&format!("{}=", key)))
            .and_then(|s| s.split('=').nth(1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(5.0)
    };

    ScoreSet {
        completeness: get("completeness"),
        counterarguments: get("counterarguments"),
        actionability: get("actionability"),
        blind_spots: get("blind_spots"),
        precision: get("precision"),
        overall: get("overall"),
    }
}

fn build_comparison_report(
    topic: &str,
    baseline_name: &str,
    forum_names: &[String],
    baseline_response: &str,
    forum_synthesis: &str,
    baseline_first: bool,
    judge_reasoning: &str,
    scores: &Scores,
) -> String {
    let (a_label, b_label) = if baseline_first {
        (
            format!("Baseline ({})", baseline_name),
            format!("Forum ({})", forum_names.join(", ")),
        )
    } else {
        (
            format!("Forum ({})", forum_names.join(", ")),
            format!("Baseline ({})", baseline_name),
        )
    };

    let reasoning = judge_reasoning
        .lines()
        .skip_while(|l| !l.starts_with("REASONING:"))
        .collect::<Vec<_>>()
        .join("\n")
        .replacen("REASONING:", "", 1)
        .trim()
        .to_string();

    format!(
        "# Eval Comparison: Single Model vs. Ting Forum\n\n\
         **Topic:** {topic}\n\
         **Baseline:** {baseline_name} (single model)\n\
         **Forum:** {forum}\n\n\
         ## Scores\n\n\
         | Dimension | Baseline | Forum | Delta |\n\
         |-----------|----------|-------|-------|\n\
         | Completeness | {bc:.1} | {fc:.1} | {dc:+.1} |\n\
         | Counterarguments | {bca:.1} | {fca:.1} | {dca:+.1} |\n\
         | Actionability | {ba:.1} | {fa:.1} | {da:+.1} |\n\
         | Blind spots | {bbs:.1} | {fbs:.1} | {dbs:+.1} |\n\
         | Precision | {bp:.1} | {fp:.1} | {dp:+.1} |\n\
         | **Overall** | **{bo:.1}** | **{fo:.1}** | **{do_:+.1}** |\n\
         | **Average** | **{bavg:.1}** | **{favg:.1}** | **{davg:+.1}** |\n\n\
         ## Judge Assignment\n\n\
         - Response A = {a_label}\n\
         - Response B = {b_label}\n\n\
         ## Judge Reasoning\n\n\
         {reasoning}\n\n\
         ---\n\n\
         ## Baseline Response ({baseline_name})\n\n\
         {baseline_response}\n\n\
         ---\n\n\
         ## Forum Synthesis ({forum})\n\n\
         {forum_synthesis}\n",
        topic = topic,
        baseline_name = baseline_name,
        forum = forum_names.join(", "),
        bc = scores.baseline.completeness,
        fc = scores.forum.completeness,
        dc = scores.forum.completeness - scores.baseline.completeness,
        bca = scores.baseline.counterarguments,
        fca = scores.forum.counterarguments,
        dca = scores.forum.counterarguments - scores.baseline.counterarguments,
        ba = scores.baseline.actionability,
        fa = scores.forum.actionability,
        da = scores.forum.actionability - scores.baseline.actionability,
        bbs = scores.baseline.blind_spots,
        fbs = scores.forum.blind_spots,
        dbs = scores.forum.blind_spots - scores.baseline.blind_spots,
        bp = scores.baseline.precision,
        fp = scores.forum.precision,
        dp = scores.forum.precision - scores.baseline.precision,
        bo = scores.baseline.overall,
        fo = scores.forum.overall,
        do_ = scores.forum.overall - scores.baseline.overall,
        bavg = scores.baseline.avg(),
        favg = scores.forum.avg(),
        davg = scores.forum.avg() - scores.baseline.avg(),
        a_label = a_label,
        b_label = b_label,
        reasoning = reasoning,
        baseline_response = baseline_response,
        forum_synthesis = forum_synthesis,
    )
}

fn write_scores_toml(path: &Path, scores: &Scores, baseline_first: bool, cfg: &EvalConfig) -> Result<()> {
    let content = format!(
        "[assignment]\n\
         response_a = \"{a}\"\n\
         response_b = \"{b}\"\n\n\
         [models]\n\
         baseline = \"{bmodel}\"\n\
         judge = \"{jmodel}\"\n\
         forum = [{fmodels}]\n\n\
         [baseline]\n\
         completeness = {bc:.1}\n\
         counterarguments = {bca:.1}\n\
         actionability = {ba:.1}\n\
         blind_spots = {bbs:.1}\n\
         precision = {bpr:.1}\n\
         overall = {bo:.1}\n\
         average = {bavg:.1}\n\n\
         [forum]\n\
         completeness = {fc:.1}\n\
         counterarguments = {fca:.1}\n\
         actionability = {fa:.1}\n\
         blind_spots = {fbs:.1}\n\
         precision = {fpr:.1}\n\
         overall = {fo:.1}\n\
         average = {favg:.1}\n",
        a = if baseline_first { "baseline" } else { "forum" },
        b = if baseline_first { "forum" } else { "baseline" },
        bmodel = config::resolve_model_id(&cfg.baseline_preset),
        jmodel = config::resolve_model_id(&cfg.judge_preset),
        fmodels = cfg.forum_presets.iter()
            .map(|p| format!("\"{}\"", config::resolve_model_id(p)))
            .collect::<Vec<_>>().join(", "),
        bc = scores.baseline.completeness,
        bca = scores.baseline.counterarguments,
        ba = scores.baseline.actionability,
        bbs = scores.baseline.blind_spots,
        bpr = scores.baseline.precision,
        bo = scores.baseline.overall,
        bavg = scores.baseline.avg(),
        fc = scores.forum.completeness,
        fca = scores.forum.counterarguments,
        fa = scores.forum.actionability,
        fbs = scores.forum.blind_spots,
        fpr = scores.forum.precision,
        fo = scores.forum.overall,
        favg = scores.forum.avg(),
    );
    substrate::write_atomic_toml(path, &content)
}

/// Generate HTML report for an eval
pub fn generate_eval_html(eval_dir: &Path) -> Result<String> {
    let meta = substrate::read_file(&eval_dir.join("meta.toml"))?;
    let baseline = substrate::read_file(&eval_dir.join("baseline.md"))?;
    let comparison = substrate::read_file(&eval_dir.join("comparison.md"))?;
    let scores_raw = substrate::read_file(&eval_dir.join("scores.toml"))?;

    // Parse key fields from meta
    let topic = extract_toml_string(&meta, "topic");
    let baseline_name = extract_toml_string(&meta, "baseline");
    let judge = extract_toml_string(&meta, "judge");
    let eval_id = extract_toml_string(&meta, "id");

    // Read forum synthesis
    let forum_synthesis = if eval_dir.join("forum").join("final").join("synthesis.md").exists() {
        substrate::read_file(&eval_dir.join("forum").join("final").join("synthesis.md"))?
    } else {
        "(Forum synthesis not available)".to_string()
    };

    // Parse scores
    let get_score = |section: &str, key: &str| -> String {
        scores_raw
            .lines()
            .skip_while(|l| !l.starts_with(&format!("[{}]", section)))
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split('=').nth(1))
            .map(|v| v.trim().to_string())
            .unwrap_or_else(|| "—".to_string())
    };

    let b_avg = get_score("baseline", "average");
    let f_avg = get_score("forum", "average");
    let b_overall = get_score("baseline", "overall");
    let f_overall = get_score("forum", "overall");

    let winner = {
        let ba: f32 = b_avg.parse().unwrap_or(0.0);
        let fa: f32 = f_avg.parse().unwrap_or(0.0);
        if (fa - ba).abs() < 0.5 {
            "Tie".to_string()
        } else if fa > ba {
            "Forum".to_string()
        } else {
            "Baseline".to_string()
        }
    };

    let winner_color = match winner.as_str() {
        "Forum" => "#4ade80",
        "Baseline" => "#facc15",
        _ => "#8b949e",
    };

    let esc = |s: &str| -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    };

    Ok(format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Ting Eval — {topic}</title>
<style>
:root {{
  --bg: #0d1117; --surface: #161b22; --surface-2: #1c2333;
  --border: #30363d; --text: #e6edf3; --text-dim: #8b949e;
  --accent: #58a6ff; --green: #4ade80; --yellow: #facc15; --red: #f87171;
  --font: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
  --mono: "SF Mono", "Fira Code", Menlo, Consolas, monospace;
}}
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{ background: var(--bg); color: var(--text); font-family: var(--font); font-size: 15px; line-height: 1.6; }}
.container {{ max-width: 1000px; margin: 0 auto; padding: 40px 24px; }}
.logo {{ font-size: 13px; font-weight: 700; letter-spacing: 4px; color: var(--accent); margin-bottom: 8px; }}
.eval-badge {{ background: #1f3a5f; color: var(--accent); font-size: 12px; padding: 2px 10px; border-radius: 10px; }}
h1 {{ font-size: 22px; margin: 16px 0; line-height: 1.3; }}
h2 {{ font-size: 17px; margin: 24px 0 12px; }}
.meta {{ color: var(--text-dim); font-size: 13px; margin-bottom: 24px; }}
.winner-row {{ display: flex; align-items: center; gap: 12px; margin: 20px 0; }}
.winner-badge {{ font-size: 18px; font-weight: 700; padding: 8px 20px; border-radius: 10px; color: var(--bg); }}
.scores-table {{ width: 100%; border-collapse: collapse; margin: 16px 0; font-size: 14px; }}
.scores-table th, .scores-table td {{ border: 1px solid var(--border); padding: 10px 14px; text-align: center; }}
.scores-table th {{ background: var(--surface-2); color: var(--text); font-weight: 600; }}
.scores-table td {{ color: var(--text-dim); }}
.scores-table .positive {{ color: var(--green); font-weight: 600; }}
.scores-table .negative {{ color: var(--red); font-weight: 600; }}
.panel {{ background: var(--surface); border: 1px solid var(--border); border-radius: 12px; padding: 24px; margin: 16px 0; }}
.panel-accent {{ border-color: #1f3a5f; border-width: 2px; }}
.side-by-side {{ display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }}
@media (max-width: 700px) {{ .side-by-side {{ grid-template-columns: 1fr; }} }}
.side-label {{ font-size: 12px; font-weight: 600; text-transform: uppercase; letter-spacing: 1px; color: var(--text-dim); margin-bottom: 8px; }}
.md-render {{ font-size: 14px; line-height: 1.7; }}
.md-src {{ display: none; }}
.md-render h1,.md-render h2,.md-render h3 {{ margin: 16px 0 8px; }}
.md-render h2 {{ font-size: 16px; }} .md-render h3 {{ font-size: 14px; }}
.md-render p {{ margin: 6px 0; }} .md-render ul,.md-render ol {{ padding-left: 20px; }}
.md-render code {{ background: var(--surface-2); border: 1px solid var(--border); border-radius: 4px; padding: 1px 5px; font-family: var(--mono); font-size: 13px; }}
.md-render pre {{ background: var(--surface-2); border-radius: 8px; padding: 12px; overflow-x: auto; margin: 10px 0; }}
.md-render pre code {{ background: none; border: none; padding: 0; font-size: 12px; }}
.md-render table {{ border-collapse: collapse; width: 100%; margin: 10px 0; font-size: 13px; }}
.md-render th,.md-render td {{ border: 1px solid var(--border); padding: 6px 10px; text-align: left; }}
.md-render th {{ background: var(--surface-2); font-weight: 600; }}
.md-render blockquote {{ border-left: 3px solid var(--accent); padding: 4px 14px; color: var(--text-dim); margin: 10px 0; }}
.md-render hr {{ border: none; border-top: 1px solid var(--border); margin: 16px 0; }}
footer {{ margin-top: 32px; padding-top: 16px; border-top: 1px solid var(--border); font-size: 12px; color: var(--text-dim); display: flex; justify-content: space-between; }}
</style>
</head>
<body>
<div class="container">

<div class="logo">TING <span class="eval-badge">EVAL</span></div>
<h1>{topic}</h1>
<div class="meta">
  Baseline: <strong>{baseline_name}</strong> &nbsp;|&nbsp;
  Judge: <strong>{judge}</strong> &nbsp;|&nbsp;
  {eval_id}
</div>

<div class="winner-row">
  <span class="winner-badge" style="background:{winner_color}">{winner}</span>
  <span style="color:var(--text-dim)">Baseline avg: {b_avg} &nbsp;|&nbsp; Forum avg: {f_avg}</span>
</div>

<h2>Scores</h2>
<table class="scores-table">
<tr><th>Dimension</th><th>Baseline</th><th>Forum</th></tr>
<tr><td>Completeness</td><td>{b_comp}</td><td>{f_comp}</td></tr>
<tr><td>Counterarguments</td><td>{b_counter}</td><td>{f_counter}</td></tr>
<tr><td>Actionability</td><td>{b_action}</td><td>{f_action}</td></tr>
<tr><td>Blind spots</td><td>{b_blind}</td><td>{f_blind}</td></tr>
<tr><td>Precision</td><td>{b_precision}</td><td>{f_precision}</td></tr>
<tr><td><strong>Overall</strong></td><td><strong>{b_overall}</strong></td><td><strong>{f_overall}</strong></td></tr>
</table>

<h2>Judge Reasoning</h2>
<div class="panel md-render"><textarea class="md-src">{comparison_escaped}</textarea></div>

<h2>Responses</h2>
<div class="side-by-side">
  <div class="panel">
    <div class="side-label">Baseline ({baseline_name})</div>
    <div class="md-render"><textarea class="md-src">{baseline_escaped}</textarea></div>
  </div>
  <div class="panel panel-accent">
    <div class="side-label">Forum Synthesis</div>
    <div class="md-render"><textarea class="md-src">{forum_escaped}</textarea></div>
  </div>
</div>

<footer>
  <span>Generated by Ting Eval v{version}</span>
</footer>

</div>
<script src="https://cdn.jsdelivr.net/npm/marked@15/marked.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/dompurify@3/dist/purify.min.js"></script>
<script>
document.querySelectorAll('.md-render').forEach(el => {{
  const src = el.querySelector('.md-src');
  if (src) {{ el.innerHTML = DOMPurify.sanitize(marked.parse(src.value)); }}
}});
</script>
</body>
</html>"##,
        topic = esc(&topic),
        baseline_name = esc(&baseline_name),
        judge = esc(&judge),
        eval_id = esc(&eval_id),
        winner = winner,
        winner_color = winner_color,
        b_avg = b_avg,
        f_avg = f_avg,
        b_comp = get_score("baseline", "completeness"),
        f_comp = get_score("forum", "completeness"),
        b_counter = get_score("baseline", "counterarguments"),
        f_counter = get_score("forum", "counterarguments"),
        b_action = get_score("baseline", "actionability"),
        f_action = get_score("forum", "actionability"),
        b_blind = get_score("baseline", "blind_spots"),
        f_blind = get_score("forum", "blind_spots"),
        b_precision = get_score("baseline", "precision"),
        f_precision = get_score("forum", "precision"),
        b_overall = b_overall,
        f_overall = f_overall,
        comparison_escaped = esc(&comparison),
        baseline_escaped = esc(&baseline),
        forum_escaped = esc(&forum_synthesis),
        version = env!("CARGO_PKG_VERSION"),
    ))
}

fn extract_toml_string(content: &str, key: &str) -> String {
    content
        .lines()
        .find(|l| l.starts_with(&format!("{} = ", key)))
        .and_then(|l| l.split('=').nth(1))
        .map(|v| v.trim().trim_matches('"').to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_judge_scores_baseline_first() {
        let output = "\
SCORES_A: completeness=8 counterarguments=7 actionability=9 blind_spots=6 precision=7 overall=8
SCORES_B: completeness=9 counterarguments=9 actionability=8 blind_spots=8 precision=9 overall=9
REASONING: Response B was more thorough.";

        let scores = parse_judge_scores(output, true).unwrap();
        // A=baseline, B=forum
        assert!((scores.baseline.completeness - 8.0).abs() < 0.01);
        assert!((scores.baseline.precision - 7.0).abs() < 0.01);
        assert!((scores.baseline.overall - 8.0).abs() < 0.01);
        assert!((scores.forum.completeness - 9.0).abs() < 0.01);
        assert!((scores.forum.precision - 9.0).abs() < 0.01);
        assert!((scores.forum.overall - 9.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_judge_scores_forum_first() {
        let output = "\
SCORES_A: completeness=9 counterarguments=9 actionability=8 blind_spots=8 precision=8 overall=9
SCORES_B: completeness=7 counterarguments=6 actionability=7 blind_spots=5 precision=6 overall=7
REASONING: Response A was better.";

        let scores = parse_judge_scores(output, false).unwrap();
        // A=forum (baseline_first=false), B=baseline
        assert!((scores.forum.completeness - 9.0).abs() < 0.01);
        assert!((scores.baseline.completeness - 7.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_score_line_defaults_on_missing() {
        let scores = parse_score_line("nothing here", "SCORES_A:");
        assert!((scores.completeness - 5.0).abs() < 0.01);
        assert!((scores.overall - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_score_set_avg() {
        let s = ScoreSet {
            completeness: 8.0,
            counterarguments: 6.0,
            actionability: 10.0,
            blind_spots: 4.0,
            precision: 7.0,
            overall: 7.0,
        };
        // (8+6+10+4+7+7)/6 = 42/6 = 7.0
        assert!((s.avg() - 7.0).abs() < 0.01);
    }

    #[test]
    fn test_build_baseline_prompt_with_context() {
        let prompt = build_baseline_prompt("test topic", Some("extra context"));
        assert!(prompt.contains("test topic"));
        assert!(prompt.contains("extra context"));
        assert!(prompt.contains("## Context"));
    }

    #[test]
    fn test_build_baseline_prompt_without_context() {
        let prompt = build_baseline_prompt("test topic", None);
        assert!(prompt.contains("test topic"));
        assert!(!prompt.contains("## Context"));
    }

    #[test]
    fn test_build_judge_prompt_contains_both_responses() {
        let prompt = build_judge_prompt("topic", "response A text", "response B text");
        assert!(prompt.contains("Response A"));
        assert!(prompt.contains("Response B"));
        assert!(prompt.contains("response A text"));
        assert!(prompt.contains("response B text"));
        assert!(prompt.contains("SCORES_A:"));
    }

    #[test]
    fn test_extract_toml_string() {
        let content = "id = \"eval-123\"\ntopic = \"test question\"";
        assert_eq!(extract_toml_string(content, "id"), "eval-123");
        assert_eq!(extract_toml_string(content, "topic"), "test question");
        assert_eq!(extract_toml_string(content, "missing"), "");
    }
}
