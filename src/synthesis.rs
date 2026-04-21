use crate::config;
use crate::substrate;
use crate::types::*;
use anyhow::Result;
use std::collections::HashMap;
use std::time::Duration;

/// Wall-clock budget for a single Fire Keeper call. Shared with the pre-round
/// classifier (`classifier.rs`) so the two paths can't silently drift.
pub(crate) const FIRE_KEEPER_TIMEOUT: Duration = Duration::from_secs(600);

fn invoke(synth_config: &SynthesisSection, prompt: &str) -> Result<String> {
    let model = config::resolve_model(&synth_config.model);
    substrate::invoke_fire_keeper_model(
        synth_config.command.as_deref(),
        model,
        prompt,
        FIRE_KEEPER_TIMEOUT,
    )
}

pub fn generate_synthesis(
    synth_config: &SynthesisSection,
    topic: &str,
    round: u32,
    stage: &Stage,
    responses: &HashMap<String, String>,
    prior_synthesis: Option<&str>,
    review_mode: bool,
) -> Result<String> {
    let prompt = if review_mode {
        build_review_synthesis_prompt(topic, round, stage, responses, prior_synthesis)
    } else {
        build_synthesis_prompt(topic, round, stage, responses, prior_synthesis)
    };
    invoke(synth_config, &prompt)
}

pub fn generate_claims(
    synth_config: &SynthesisSection,
    topic: &str,
    responses: &HashMap<String, String>,
) -> Result<String> {
    let prompt = build_claims_prompt(topic, responses);
    invoke(synth_config, &prompt)
}

pub fn generate_dissent(
    synth_config: &SynthesisSection,
    topic: &str,
    responses: &HashMap<String, String>,
    key_disagreements: &[String],
) -> Result<String> {
    let prompt = build_dissent_prompt(topic, responses, key_disagreements);
    invoke(synth_config, &prompt)
}

fn build_synthesis_prompt(
    topic: &str,
    round: u32,
    stage: &Stage,
    responses: &HashMap<String, String>,
    prior_synthesis: Option<&str>,
) -> String {
    let mut prompt = format!(
        "You are synthesizing participant responses from a structured deliberation.\n\n\
         Topic: {}\n\
         Round: {} ({})\n",
        topic, round, stage
    );

    if let Some(prior) = prior_synthesis {
        prompt.push_str(&format!("\n## Prior Round Synthesis\n{}\n", prior));
    }

    prompt.push_str("\n## Participant Responses\n");
    let mut names: Vec<&String> = responses.keys().collect();
    names.sort();
    for name in &names {
        prompt.push_str(&format!("\n### {}\n{}\n", name, responses[*name]));
    }

    prompt.push_str(
        "\n---\n\n\
         Create a narrative synthesis that:\n\
         1. Identifies areas of agreement\n\
         2. Highlights key differences and disagreements\n\
         3. Notes the strongest arguments from each position\n\
         4. Is balanced and does not favor any single participant\n\n\
         Write in clear markdown.\n",
    );

    prompt
}

fn build_review_synthesis_prompt(
    topic: &str,
    round: u32,
    stage: &Stage,
    responses: &HashMap<String, String>,
    prior_synthesis: Option<&str>,
) -> String {
    let mut prompt = format!(
        "You are synthesizing a code review from multiple reviewers.\n\n\
         Topic: {}\n\
         Round: {} ({})\n",
        topic, round, stage
    );

    if let Some(prior) = prior_synthesis {
        prompt.push_str(&format!("\n## Prior Round Findings\n{}\n", prior));
    }

    prompt.push_str("\n## Reviewer Responses\n");
    let mut names: Vec<&String> = responses.keys().collect();
    names.sort();
    for name in &names {
        prompt.push_str(&format!("\n### {}\n{}\n", name, responses[*name]));
    }

    prompt.push_str(
        "\n---\n\n\
         Produce a prioritized findings list. For each finding:\n\
         - Severity: [CRITICAL], [HIGH], [MEDIUM], or [LOW]\n\
         - Title and location (file:line if mentioned by reviewers)\n\
         - Description of the issue and its impact\n\
         - **Fix:** Concrete fix instruction\n\
         - **Confidence:** High if all reviewers agree, Medium if contested, \
         Low if only one reviewer raised it\n\n\
         Format each finding as:\n\
         ### [SEVERITY] Issue title — file:line\n\
         Description.\n\
         **Fix:** ...\n\
         **Confidence:** High/Medium/Low (note dissenting views if any)\n\n\
         Sort by severity (CRITICAL first). Only include findings that survived \
         cross-examination — if a finding was raised then refuted and dropped, \
         do not include it. If a finding was contested but not resolved, include \
         it with Confidence: Medium and note the disagreement.\n",
    );

    prompt
}

fn build_claims_prompt(topic: &str, responses: &HashMap<String, String>) -> String {
    let mut names: Vec<&String> = responses.keys().collect();
    names.sort();

    let mut prompt = format!(
        "You are extracting structured claims from a deliberation round.\n\n\
         Topic: {}\n\n\
         ## Responses\n",
        topic
    );

    for name in &names {
        prompt.push_str(&format!("\n### {}\n{}\n", name, responses[*name]));
    }

    let stance_example: String = names
        .iter()
        .map(|n| format!("{} = \"support\"  # or \"oppose\" or \"neutral\"", n))
        .collect::<Vec<_>>()
        .join("\n");

    prompt.push_str(&format!(
        "\n---\n\n\
         Extract the key claims made by participants. Output valid TOML.\n\n\
         Use this structure:\n\n\
         [[claims]]\n\
         text = \"The claim text\"\n\
         confidence = \"high\"  # low, medium, or high\n\n\
         [claims.stances]\n\
         {}\n\n\
         Output ONLY the TOML content, no markdown fences or extra text.\n",
        stance_example
    ));

    prompt
}

fn build_dissent_prompt(
    topic: &str,
    responses: &HashMap<String, String>,
    key_disagreements: &[String],
) -> String {
    let mut prompt = format!(
        "You are documenting unresolved disagreements from a structured deliberation.\n\n\
         Topic: {}\n\n\
         ## Key Disagreements Identified\n",
        topic
    );

    for d in key_disagreements {
        prompt.push_str(&format!("- {}\n", d));
    }

    prompt.push_str("\n## Final Participant Positions\n");
    let mut names: Vec<&String> = responses.keys().collect();
    names.sort();
    for name in &names {
        prompt.push_str(&format!("\n### {}\n{}\n", name, responses[*name]));
    }

    prompt.push_str(
        "\n---\n\n\
         Write a clear document of the unresolved disagreements.\n\
         For each disagreement:\n\
         1. State the disagreement clearly\n\
         2. Summarize each participant's position\n\
         3. Note why convergence was not reached\n\n\
         Dissent is a first-class output — it represents valuable, genuine differences \
         in perspective, not failure.\n\
         Write in clear markdown.\n",
    );

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_synthesis_prompt_includes_responses() {
        let mut responses = HashMap::new();
        responses.insert("alice".to_string(), "Use Rust".to_string());
        responses.insert("bob".to_string(), "Use Go".to_string());

        let prompt =
            build_synthesis_prompt("Best language?", 1, &Stage::Proposal, &responses, None);
        assert!(prompt.contains("Best language?"));
        assert!(prompt.contains("Use Rust"));
        assert!(prompt.contains("Use Go"));
        assert!(prompt.contains("Round: 1 (proposal)"));
    }

    #[test]
    fn test_synthesis_prompt_includes_prior() {
        let mut responses = HashMap::new();
        responses.insert("alice".to_string(), "Still Rust".to_string());

        let prompt = build_synthesis_prompt(
            "Language?",
            2,
            &Stage::CrossExam,
            &responses,
            Some("Prior synthesis text"),
        );
        assert!(prompt.contains("Prior synthesis text"));
        assert!(prompt.contains("Round: 2 (cross-examination)"));
    }

    #[test]
    fn test_claims_prompt_lists_participants() {
        let mut responses = HashMap::new();
        responses.insert("alice".to_string(), "Claim A".to_string());
        responses.insert("bob".to_string(), "Claim B".to_string());

        let prompt = build_claims_prompt("Topic", &responses);
        assert!(prompt.contains("alice"));
        assert!(prompt.contains("bob"));
        assert!(prompt.contains("[[claims]]"));
        assert!(prompt.contains("[claims.stances]"));
    }

    #[test]
    fn test_dissent_prompt_includes_disagreements() {
        let mut responses = HashMap::new();
        responses.insert("alice".to_string(), "Position A".to_string());

        let disagreements = vec![
            "Architecture choice".to_string(),
            "Timeline".to_string(),
        ];

        let prompt = build_dissent_prompt("Topic", &responses, &disagreements);
        assert!(prompt.contains("Architecture choice"));
        assert!(prompt.contains("Timeline"));
        assert!(prompt.contains("first-class output"));
    }
}
