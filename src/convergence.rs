use crate::config;
use crate::substrate;
use crate::types::*;
use anyhow::Result;
use rand::seq::SliceRandom;
use std::collections::HashMap;
use std::time::Duration;

const FIRE_KEEPER_TIMEOUT: Duration = Duration::from_secs(600);

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

    let output = invoke_judge(convergence_config, &prompt)?;
    parse_alignment_scores(&output, &names)
}

fn parse_alignment_scores(
    output: &str,
    expected: &[&String],
) -> Result<AlignmentScores> {
    let mut scores = AlignmentScores::new();
    let line = output
        .lines()
        .find(|l| l.trim().starts_with("ALIGNMENT:"))
        .unwrap_or("");

    for token in line.split_whitespace() {
        if let Some((name, val)) = token.split_once('=') {
            if let Ok(score) = val.parse::<f32>() {
                scores.insert(name.to_string(), score);
            }
        }
    }

    // Fill missing with 5.0
    for name in expected {
        scores.entry(name.to_string()).or_insert(5.0);
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
    let output = invoke_judge(convergence_config, &prompt)?;
    parse_judge_response(&output, threshold)
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
            // Filter NaN/inf — serde_json panics on non-finite numbers when
            // the score later lands in a dashboard event payload.
            score = Some(
                s.trim()
                    .parse::<f32>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .unwrap_or(5.0),
            );
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

    let score = score.unwrap_or(5.0);

    if score >= threshold as f32 {
        Ok(ConvergenceResult::Converged { score, summary })
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
            ConvergenceResult::Converged { score, summary } => {
                assert!((score - 8.5).abs() < 0.01);
                assert!(summary.contains("Strong agreement"));
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
    fn test_parse_judge_non_finite_score_falls_back_to_5() {
        for raw in ["SCORE: nan\nSUMMARY: x", "SCORE: inf\nSUMMARY: x", "SCORE: -inf\nSUMMARY: x"] {
            let result = parse_judge_response(raw, 7).unwrap();
            let score = match result {
                ConvergenceResult::Converged { score, .. }
                | ConvergenceResult::Divergent { score, .. } => score,
            };
            assert!(score.is_finite(), "score from {raw:?} should be finite, got {score}");
            assert!((score - 5.0).abs() < 0.01, "expected 5.0 fallback, got {score}");
        }
    }

    #[test]
    fn test_parse_judge_malformed_defaults_to_5() {
        let output = "This model didn't follow the format at all.";
        let result = parse_judge_response(output, 7).unwrap();
        match result {
            ConvergenceResult::Divergent { score, .. } => {
                assert!((score - 5.0).abs() < 0.01);
            }
            _ => panic!("Malformed output should default to divergent with score 5"),
        }
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
