use crate::config;
use crate::substrate;
use crate::types::*;
use anyhow::{Context, Result};
use rand::seq::SliceRandom;
use std::collections::HashMap;
use std::time::Duration;

const FIRE_KEEPER_TIMEOUT: Duration = Duration::from_secs(600);

/// Retry a failed model call or invalid judgment once; never invent a score.
fn evaluate_with_retry<T>(
    mut invoke: impl FnMut() -> Result<String>,
    parse: impl Fn(&str) -> Result<T>,
    label: &str,
) -> Result<T> {
    for attempt in 1..=2 {
        match invoke().and_then(|output| parse(&output)) {
            Ok(value) => return Ok(value),
            Err(error) if attempt == 1 => eprintln!("  Warning: {label} attempt failed: {error:#}. Retrying once."),
            Err(error) => return Err(error).with_context(|| format!("{label} unavailable after 2 attempts")),
        }
    }
    unreachable!()
}

/// Both convergence and alignment use finite values on the closed 1–10 scale.
fn parse_score(raw: &str) -> Result<f32> {
    let score: f32 = raw.trim().parse().context("Invalid numeric score")?;
    anyhow::ensure!(score.is_finite() && (1.0..=10.0).contains(&score), "Score must be finite and within 1–10");
    Ok(score)
}

fn invoke_judge(convergence_config: &ConvergenceSection, prompt: &str) -> Result<String> {
    let model = config::resolve_model(&convergence_config.judge_model);
    substrate::invoke_fire_keeper_model(
        convergence_config.judge_command.as_deref(),
        model,
        prompt,
        FIRE_KEEPER_TIMEOUT,
    )
}

/// Per-participant alignment scores for a round
pub type AlignmentScores = HashMap<String, f32>;

/// Score each participant's alignment with the synthesis (1-10).
/// Used for the position shift chart in HTML reports.
pub fn evaluate_alignment(
    convergence_config: &ConvergenceSection,
    synthesis: &str,
    responses: &HashMap<String, String>,
) -> Result<AlignmentScores> {
    let mut prompt = format!(
        "You are scoring how well each participant's position aligns with the group synthesis.\n\n\
         ## Synthesis\n{}\n\n\
         ## Participant Responses\n",
        synthesis
    );
    let mut names: Vec<&String> = responses.keys().collect();
    names.sort();
    for name in &names {
        prompt.push_str(&format!("\n### {}\n{}\n", name, responses[*name]));
    }
    prompt.push_str(
        "\n---\n\n\
         Score each participant's alignment with the synthesis on a 1-10 scale.\n\
         1 = completely divergent, 10 = fully aligned.\n\n\
         Respond in EXACTLY this format (one per line):\n\
         ALIGNMENT: participant_name=score participant_name=score ...\n",
    );

    evaluate_with_retry(
        || invoke_judge(convergence_config, &prompt),
        |output| parse_alignment_scores(output, &names),
        "Alignment evaluation",
    )
}

fn parse_alignment_scores(
    output: &str,
    expected: &[&String],
) -> Result<AlignmentScores> {
    let mut scores = AlignmentScores::new();
    let mut lines = output.lines().filter_map(|line| line.trim().strip_prefix("ALIGNMENT:"));
    let line = lines.next().context("Missing ALIGNMENT line")?;
    anyhow::ensure!(lines.next().is_none(), "Duplicate ALIGNMENT lines");

    for token in line.split_whitespace() {
        let (name, value) = token.split_once('=').context("Malformed alignment entry")?;
        anyhow::ensure!(expected.iter().any(|expected| expected.as_str() == name), "Unknown alignment participant: {name}");
        let score = parse_score(value)?;
        anyhow::ensure!(scores.insert(name.to_string(), score).is_none(), "Duplicate alignment participant: {name}");
    }

    for name in expected {
        anyhow::ensure!(scores.contains_key(name.as_str()), "Missing alignment score for {name}");
    }
    Ok(scores)
}

/// Evaluate convergence of participant responses using the configured policy
pub fn evaluate(
    convergence_config: &ConvergenceSection,
    topic: &str,
    responses: &HashMap<String, String>,
    threshold: u32,
) -> Result<ConvergenceResult> {
    let prompt = build_judge_prompt(topic, responses);
    evaluate_with_retry(
        || invoke_judge(convergence_config, &prompt),
        |output| parse_judge_response(output, threshold),
        "Convergence evaluation",
    )
}

fn build_judge_prompt(topic: &str, responses: &HashMap<String, String>) -> String {
    let mut prompt = format!(
        "You are evaluating whether participants in a structured deliberation have reached consensus.\n\n\
         Topic: {}\n\n\
         Participant responses:\n",
        topic
    );

    // Randomize order to prevent position bias
    let mut names: Vec<&String> = responses.keys().collect();
    let mut rng = rand::thread_rng();
    names.shuffle(&mut rng);

    for name in &names {
        prompt.push_str(&format!("\n### {}\n{}\n", name, responses[*name]));
    }

    prompt.push_str(
        "\n---\n\n\
         Rate the level of agreement on a scale of 1-10:\n\
         - 1 = Complete disagreement on all major points\n\
         - 5 = Agreement on some points, significant disagreements remain\n\
         - 7 = Substantial agreement with minor disagreements\n\
         - 10 = Complete consensus\n\n\
         Respond in EXACTLY this format (no other text):\n\
         SCORE: <number>\n\
         SUMMARY: <one paragraph summarizing the state of agreement>\n\
         DISAGREEMENTS:\n\
         - <disagreement 1>\n\
         - <disagreement 2>\n",
    );

    prompt
}

fn parse_judge_response(output: &str, threshold: u32) -> Result<ConvergenceResult> {
    let mut score: Option<f32> = None;
    let mut summary = String::new();
    let mut disagreements: Vec<String> = Vec::new();
    let mut in_disagreements = false;

    for line in output.lines() {
        let line = line.trim();
        if let Some(s) = line.strip_prefix("SCORE:") {
            anyhow::ensure!(score.is_none(), "Duplicate SCORE lines");
            score = Some(parse_score(s)?);
            in_disagreements = false;
        } else if let Some(s) = line.strip_prefix("SUMMARY:") {
            summary = s.trim().to_string();
            in_disagreements = false;
        } else if line.starts_with("DISAGREEMENTS:") {
            in_disagreements = true;
        } else if in_disagreements && line.starts_with("- ") {
            disagreements.push(line[2..].to_string());
        }
    }

    let score = score.context("Missing SCORE line")?;

    if score >= threshold as f32 {
        Ok(ConvergenceResult::Converged {
            score,
            summary,
            key_disagreements: disagreements,
        })
    } else {
        Ok(ConvergenceResult::Divergent {
            score,
            key_disagreements: disagreements,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_judge_converged() {
        let output = "\
SCORE: 8.5
SUMMARY: Strong agreement on most points with only minor differences in implementation details.
DISAGREEMENTS:
- Minor difference on timing of rollout";

        let result = parse_judge_response(output, 7).unwrap();
        match result {
            ConvergenceResult::Converged { score, summary, key_disagreements } => {
                assert!((score - 8.5).abs() < 0.01);
                assert!(summary.contains("Strong agreement"));
                assert_eq!(key_disagreements, ["Minor difference on timing of rollout"]);
            }
            _ => panic!("Expected Converged"),
        }
    }

    #[test]
    fn test_parse_judge_divergent() {
        let output = "\
SCORE: 4
SUMMARY: Significant disagreements on fundamental architecture choices.
DISAGREEMENTS:
- Architecture choice: monolith vs microservices
- Timeline: Q1 vs Q3 delivery";

        let result = parse_judge_response(output, 7).unwrap();
        match result {
            ConvergenceResult::Divergent {
                score,
                key_disagreements,
            } => {
                assert!((score - 4.0).abs() < 0.01);
                assert_eq!(key_disagreements.len(), 2);
                assert!(key_disagreements[0].contains("Architecture"));
                assert!(key_disagreements[1].contains("Timeline"));
            }
            _ => panic!("Expected Divergent"),
        }
    }

    #[test]
    fn test_parse_judge_exact_threshold() {
        let output = "SCORE: 7\nSUMMARY: At threshold.\nDISAGREEMENTS:\n- None major";
        let result = parse_judge_response(output, 7).unwrap();
        assert!(matches!(result, ConvergenceResult::Converged { .. }));
    }

    #[test]
    fn invalid_judgments_are_errors_instead_of_plausible_scores() {
        for raw in ["SCORE: nan", "SCORE: inf", "SCORE: -inf", "SCORE: 0", "SCORE: 11", "SCORE: nope", "No score", "SCORE: 7\nSCORE: 8"] {
            assert!(parse_judge_response(raw, 7).is_err(), "accepted {raw:?}");
        }
        for raw in ["SCORE: 1", "SCORE: 10", "SCORE: 7.5"] {
            assert!(parse_judge_response(raw, 7).is_ok());
        }
    }

    #[test]
    fn alignment_requires_exactly_one_valid_score_per_participant() {
        let names = ["alice".to_string(), "bob".to_string()];
        let expected = names.iter().collect::<Vec<_>>();
        let valid = parse_alignment_scores("ALIGNMENT: alice=7.5 bob=1", &expected).unwrap();
        assert_eq!(valid["alice"], 7.5);
        assert_eq!(valid["bob"], 1.0);
        for raw in [
            "No scores", "ALIGNMENT: alice=8", "ALIGNMENT: alice=8 bob=NaN",
            "ALIGNMENT: alice=0 bob=8", "ALIGNMENT: alice=8 bob=11",
            "ALIGNMENT: alice=8 bob=8 charlie=8", "ALIGNMENT: alice=8 alice=9 bob=8",
            "ALIGNMENT: alice=8 bob=8 extra", "ALIGNMENT: alice=8 bob=8\nALIGNMENT: alice=9 bob=9",
        ] {
            assert!(parse_alignment_scores(raw, &expected).is_err(), "accepted {raw:?}");
        }
    }

    #[test]
    fn invalid_judgment_retries_once_then_recovers_or_reports_unavailable() {
        let mut calls = 0;
        let value = evaluate_with_retry(
            || { calls += 1; Ok(if calls == 1 { "invalid" } else { "SCORE: 8" }.to_string()) },
            |output| parse_judge_response(output, 7),
            "Convergence evaluation",
        ).unwrap();
        assert_eq!(calls, 2);
        assert_eq!(value.score(), 8.0);

        calls = 0;
        let error = evaluate_with_retry(
            || { calls += 1; Ok("invalid".to_string()) },
            |output| parse_judge_response(output, 7),
            "Convergence evaluation",
        ).unwrap_err();
        assert_eq!(calls, 2);
        assert!(format!("{error:#}").contains("unavailable after 2 attempts"));
        assert!(format!("{error:#}").contains("Missing SCORE"));
    }

    #[test]
    fn test_build_judge_prompt_contains_all_responses() {
        let mut responses = HashMap::new();
        responses.insert("alice".to_string(), "Alice says yes".to_string());
        responses.insert("bob".to_string(), "Bob says no".to_string());

        let prompt = build_judge_prompt("Test topic", &responses);
        assert!(prompt.contains("Test topic"));
        assert!(prompt.contains("Alice says yes"));
        assert!(prompt.contains("Bob says no"));
        assert!(prompt.contains("SCORE:"));
    }
}
