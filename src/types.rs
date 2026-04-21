use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level forum configuration from meta.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumConfig {
    pub forum: ForumSection,
    pub participants: ParticipantsSection,
    #[serde(default)]
    pub timing: TimingSection,
    #[serde(default)]
    pub convergence: ConvergenceSection,
    #[serde(default)]
    pub synthesis: SynthesisSection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForumSection {
    pub id: String,
    pub topic: String,
    pub created: String,
    pub max_rounds: u32,
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub output_format: Option<String>,
}

fn default_protocol() -> String {
    "delphi-crossexam".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipantsSection {
    pub names: Vec<String>,
    #[serde(flatten)]
    pub configs: HashMap<String, ParticipantConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipantConfig {
    #[serde(rename = "type")]
    pub participant_type: String,
    pub command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingSection {
    #[serde(default = "default_round_timeout")]
    pub round_timeout: String,
    #[serde(default = "default_participant_timeout")]
    pub participant_timeout: String,
    #[serde(default)]
    pub quorum: u32,
    #[serde(default = "default_late_policy")]
    pub late_policy: String,
}

impl Default for TimingSection {
    fn default() -> Self {
        Self {
            round_timeout: default_round_timeout(),
            participant_timeout: default_participant_timeout(),
            quorum: 0,
            late_policy: default_late_policy(),
        }
    }
}

fn default_round_timeout() -> String {
    "5m".into()
}
fn default_participant_timeout() -> String {
    "2m".into()
}
fn default_late_policy() -> String {
    "include_next".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceSection {
    #[serde(default = "default_policy")]
    pub policy: String,
    #[serde(default = "default_judge_model")]
    pub judge_model: String,
    #[serde(default)]
    pub judge_command: Option<String>,
    #[serde(default = "default_threshold")]
    pub threshold: u32,
    #[serde(default = "default_min_rounds")]
    pub min_rounds: u32,
}

impl Default for ConvergenceSection {
    fn default() -> Self {
        Self {
            policy: default_policy(),
            judge_model: default_judge_model(),
            judge_command: None,
            threshold: default_threshold(),
            min_rounds: default_min_rounds(),
        }
    }
}

fn default_policy() -> String {
    "llm-judge".into()
}
fn default_judge_model() -> String {
    "claude-opus".into()
}
fn default_threshold() -> u32 {
    7
}
fn default_min_rounds() -> u32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisSection {
    #[serde(default = "default_synthesis_model")]
    pub model: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default = "default_max_prior_context")]
    pub max_prior_context: u32,
}

impl Default for SynthesisSection {
    fn default() -> Self {
        Self {
            model: default_synthesis_model(),
            command: None,
            max_prior_context: default_max_prior_context(),
        }
    }
}

fn default_synthesis_model() -> String {
    "claude-opus".into()
}
fn default_max_prior_context() -> u32 {
    4000
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    Proposal,
    CrossExam,
    Revision,
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stage::Proposal => write!(f, "proposal"),
            Stage::CrossExam => write!(f, "cross-examination"),
            Stage::Revision => write!(f, "revision"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ConvergenceResult {
    Converged { score: f32, summary: String },
    Divergent { score: f32, key_disagreements: Vec<String> },
}

impl ConvergenceResult {
    pub fn score(&self) -> f32 {
        match self {
            Self::Converged { score, .. } | Self::Divergent { score, .. } => *score,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RoundData {
    pub number: u32,
    pub stage: Stage,
    pub responses: HashMap<String, String>,
    pub synthesis: Option<String>,
    pub claims: Option<String>,
}
