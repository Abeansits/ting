//! A bundled, explicitly illustrative forum requiring no model credentials.

use crate::{classifier::ClassifierMetric, config, events, run_status, substrate, types::*};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;

#[derive(Deserialize)]
struct Sample {
    topic: String,
    participants: Vec<String>,
    metrics: Vec<ClassifierMetric>,
    rounds: Vec<SampleRound>,
    dissent: String,
}

#[derive(Deserialize)]
struct SampleRound {
    prompt: String,
    responses: HashMap<String, String>,
    synthesis: String,
    claims: String,
    score: f32,
    scores: HashMap<String, f32>,
}

/// Create a new sample directory without overwriting any existing session.
pub fn create(path: &Path, id: &str) -> Result<()> {
    let sample: Sample = serde_json::from_str(include_str!("../examples/demo-forum.json"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir(path).context("Demo destination already exists or cannot be created")?;
    let created = chrono::Utc::now().to_rfc3339();
    let cfg = ForumConfig {
        forum: ForumSection {
            id: id.into(),
            topic: sample.topic.clone(),
            created: created.clone(),
            max_rounds: sample.rounds.len() as u32,
            protocol: "delphi-crossexam".into(),
            context: Some(
                "Hand-written illustrative sample. No live model calls; all scores are examples."
                    .into(),
            ),
            output_format: None,
        },
        participants: ParticipantsSection {
            names: sample.participants.clone(),
            configs: sample
                .participants
                .iter()
                .map(|name| {
                    (
                        name.clone(),
                        ParticipantConfig {
                            participant_type: "manual".into(),
                            command: None,
                        },
                    )
                })
                .collect(),
        },
        timing: TimingSection::default(),
        convergence: ConvergenceSection {
            judge_model: "illustrative-sample".into(),
            ..ConvergenceSection::default()
        },
        synthesis: SynthesisSection {
            model: "illustrative-sample".into(),
            ..SynthesisSection::default()
        },
    };
    config::validate(&cfg)?;
    config::save(&cfg, &path.join("meta.toml"))?;
    run_status::write(path, run_status::Status::Running, None)?;
    events::emit(
        path,
        id,
        events::EventType::ForumStarted,
        json!({
            "topic": sample.topic, "participants": sample.participants, "max_rounds": sample.rounds.len(),
        }),
    )?;
    let metrics_dir = substrate::create_round_dir(path, 0)?;
    substrate::write_atomic(
        &metrics_dir.join("metrics.json"),
        &serde_json::to_string_pretty(&json!({
            "version": 1, "forum_id": id, "generated_at": created, "model": "illustrative-sample", "metrics": sample.metrics,
        }))?,
    )?;
    events::emit(
        path,
        id,
        events::EventType::ClassifierMetrics,
        json!({"metrics": sample.metrics}),
    )?;
    for (index, round) in sample.rounds.iter().enumerate() {
        let number = index as u32 + 1;
        let dir = substrate::create_round_dir(path, number)?;
        substrate::write_atomic(&dir.join("prompt.md"), &round.prompt)?;
        events::emit(
            path,
            id,
            events::EventType::RoundStarted,
            json!({
                "round": number, "stage": if number == 1 { "proposal" } else { "cross-examination" },
            }),
        )?;
        for name in &sample.participants {
            let response = round
                .responses
                .get(name)
                .context("Missing sample response")?;
            substrate::write_atomic(&dir.join(format!("{name}.md")), response)?;
            events::emit(
                path,
                id,
                events::EventType::ParticipantResponse,
                json!({
                    "round": number, "participant": name, "word_count": response.split_whitespace().count(),
                }),
            )?;
        }
        substrate::write_atomic(&dir.join("synthesis.md"), &round.synthesis)?;
        substrate::write_atomic_toml(&dir.join("claims.toml"), &round.claims)?;
        events::emit(
            path,
            id,
            events::EventType::Synthesis,
            json!({"round": number, "word_count": round.synthesis.split_whitespace().count()}),
        )?;
        let scores: Vec<_> = sample
            .metrics
            .iter()
            .map(|metric| json!({"metric_id": metric.id, "score": round.scores[&metric.id]}))
            .collect();
        substrate::write_atomic(
            &dir.join("metric-scores.json"),
            &serde_json::to_string_pretty(&json!({
                "version": 1, "forum_id": id, "round": number, "scored_at": created,
                "model": "illustrative-sample", "scores": scores,
            }))?,
        )?;
        events::emit(
            path,
            id,
            events::EventType::MetricScores,
            json!({"round": number, "scores": scores}),
        )?;
        events::emit(
            path,
            id,
            events::EventType::Convergence,
            json!({"round": number, "score": round.score}),
        )?;
    }
    let last = sample.rounds.last().context("Demo has no rounds")?;
    let final_dir = substrate::create_final_dir(path)?;
    substrate::write_atomic(&final_dir.join("synthesis.md"), &last.synthesis)?;
    substrate::write_atomic(&final_dir.join("dissent.md"), &sample.dissent)?;
    substrate::write_atomic_toml(&final_dir.join("claims.toml"), &last.claims)?;
    substrate::write_atomic_toml(
        &final_dir.join("meta-summary.toml"),
        &format!(
            "[summary]\nstatus = \"converged\"\nfinal_score = {}\ntotal_rounds = {}\nparticipants = {}\ndemo = true\n",
            last.score,
            sample.rounds.len(),
            sample.participants.len(),
        ),
    )?;
    run_status::write(path, run_status::Status::Completed, None)?;
    events::emit(
        path,
        id,
        events::EventType::ForumComplete,
        json!({"rounds_used": sample.rounds.len()}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_is_complete_valid_and_does_not_overwrite_existing_data() {
        let path = std::env::temp_dir().join(format!("ting-demo-test-{}", uuid::Uuid::new_v4()));
        create(&path, "ting-demo-test").unwrap();
        assert!(substrate::is_completed(&path));
        assert!(
            std::fs::read_to_string(path.join("final/dissent.md"))
                .unwrap()
                .contains("still prefers a managed queue")
        );
        let cfg = config::load(&path.join("meta.toml")).unwrap();
        assert!(
            cfg.participants
                .configs
                .values()
                .all(|pc| pc.command.is_none())
        );
        let schema =
            serde_json::from_str(include_str!("../schemas/dashboard-event.schema.json")).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let log = std::fs::read_to_string(events::event_log_path(&path)).unwrap();
        for line in log.lines() {
            assert!(validator.is_valid(&serde_json::from_str(line).unwrap()));
        }
        assert!(create(&path, "another-id").is_err());
        assert_eq!(
            config::load(&path.join("meta.toml")).unwrap().forum.id,
            "ting-demo-test"
        );
        std::fs::remove_dir_all(path).unwrap();
    }
}
