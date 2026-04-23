use crate::types::*;
use anyhow::{Context, Result};
use std::path::Path;
use std::time::Duration;

pub fn load(path: &Path) -> Result<ForumConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config: {}", path.display()))?;
    let config: ForumConfig =
        toml::from_str(&content).with_context(|| "Failed to parse meta.toml")?;
    validate(&config)?;
    Ok(config)
}

pub fn save(config: &ForumConfig, path: &Path) -> Result<()> {
    let content =
        toml::to_string_pretty(config).with_context(|| "Failed to serialize config")?;
    std::fs::write(path, content)
        .with_context(|| format!("Failed to write config: {}", path.display()))?;
    Ok(())
}

pub fn validate(config: &ForumConfig) -> Result<()> {
    validate_id(&config.forum.id, "forum ID")?;
    if config.participants.names.is_empty() {
        anyhow::bail!("At least one participant required");
    }
    for name in &config.participants.names {
        validate_id(name, "participant name")?;
        if !config.participants.configs.contains_key(name) {
            anyhow::bail!("Missing config for participant: {}", name);
        }
        let pc = &config.participants.configs[name];
        if pc.participant_type == "command" && pc.command.is_none() {
            anyhow::bail!("Command participant '{}' requires a command", name);
        }
        if !["command", "manual"].contains(&pc.participant_type.as_str()) {
            anyhow::bail!(
                "Unknown participant type '{}' for '{}'",
                pc.participant_type,
                name
            );
        }
    }
    // Reject reserved filenames as participant names
    for name in &config.participants.names {
        if ["prompt", "synthesis", "claims"].contains(&name.as_str()) {
            anyhow::bail!(
                "Participant name '{}' conflicts with reserved filename",
                name
            );
        }
    }
    if config.forum.max_rounds == 0 {
        anyhow::bail!("max_rounds must be > 0");
    }
    if config.convergence.threshold == 0 || config.convergence.threshold > 10 {
        anyhow::bail!("convergence threshold must be 1-10");
    }
    Ok(())
}

/// Validate an identifier (participant name or forum ID) to prevent path traversal.
/// Must match [a-z0-9_-], max 64 chars, no path separators.
fn validate_id(id: &str, label: &str) -> Result<()> {
    if id.is_empty() {
        anyhow::bail!("{} cannot be empty", label);
    }
    if id.len() > 64 {
        anyhow::bail!("{} too long (max 64 chars): {}", label, id);
    }
    if id.contains('/') || id.contains('\\') || id.contains("..") || id.contains('\0') {
        anyhow::bail!("{} contains illegal characters: {}", label, id);
    }
    if id == "." || id == ".." {
        anyhow::bail!("{} cannot be '.' or '..'", label);
    }
    // Participant names have stricter rules (used as filenames)
    if label == "participant name"
        && !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        anyhow::bail!(
            "{} must be [a-z0-9_-] only: {}",
            label,
            id
        );
    }
    Ok(())
}

pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    if let Some(mins) = s.strip_suffix('m') {
        let n: u64 = mins
            .parse()
            .with_context(|| format!("Invalid duration: {}", s))?;
        Ok(Duration::from_secs(n * 60))
    } else if let Some(secs) = s.strip_suffix('s') {
        let n: u64 = secs
            .parse()
            .with_context(|| format!("Invalid duration: {}", s))?;
        Ok(Duration::from_secs(n))
    } else if let Some(hours) = s.strip_suffix('h') {
        let n: u64 = hours
            .parse()
            .with_context(|| format!("Invalid duration: {}", s))?;
        Ok(Duration::from_secs(n * 3600))
    } else {
        anyhow::bail!(
            "Invalid duration format '{}' (expected e.g. '5m', '30s', '1h')",
            s
        );
    }
}

/// Built-in presets for common model CLIs.
fn builtin_preset(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "codex" => Some(("command", "codex exec --full-auto --skip-git-repo-check -c model_reasoning_effort=medium -")),
        "gemini" => Some(("command", "cat {prompt_file} | gemini -p ' '")),
        "claude" => Some(("command", "cat {prompt_file} | claude -p - --model claude-opus-4-6")),
        "opencode" => Some(("command", "opencode run -m opencode-go/kimi-k2.5")),
        "ollama" => Some(("command", "cat {prompt_file} | ollama run llama3")),
        _ => None,
    }
}

/// Resolve a preset by name: user presets (from ~/.ting/config.toml) override built-ins.
pub fn preset_command(name: &str) -> Option<(String, String)> {
    // Check user presets first
    if let Some(cmd) = load_user_preset(name) {
        return Some(("command".to_string(), cmd));
    }
    // Fall back to built-in
    builtin_preset(name).map(|(t, c)| (t.to_string(), c.to_string()))
}

/// List all presets: built-in + user (user overrides shown with [custom] tag)
pub fn list_all_presets() -> Vec<(String, String, bool)> {
    let builtins = ["codex", "gemini", "claude", "opencode", "ollama"];
    let user_presets = load_user_presets();

    let mut result: Vec<(String, String, bool)> = Vec::new();

    // Built-ins (mark overridden ones)
    for name in &builtins {
        if let Some(cmd) = user_presets.get(*name) {
            result.push((name.to_string(), cmd.clone(), true));
        } else if let Some((_, cmd)) = builtin_preset(name) {
            result.push((name.to_string(), cmd.to_string(), false));
        }
    }

    // User-only presets (not overriding a built-in)
    for (name, cmd) in &user_presets {
        if !builtins.contains(&name.as_str()) {
            result.push((name.clone(), cmd.clone(), true));
        }
    }

    result
}

fn ting_config_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".ting").join("config.toml")
}

fn load_user_presets() -> std::collections::HashMap<String, String> {
    let path = ting_config_path();
    if !path.exists() {
        return std::collections::HashMap::new();
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return std::collections::HashMap::new(),
    };
    let table: toml::Table = match content.parse() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Warning: failed to parse {}: {}", path.display(), e);
            return std::collections::HashMap::new();
        }
    };
    let mut presets = std::collections::HashMap::new();
    if let Some(toml::Value::Table(p)) = table.get("presets") {
        for (name, val) in p {
            if let toml::Value::String(cmd) = val {
                presets.insert(name.clone(), cmd.clone());
            }
        }
    }
    presets
}

fn load_user_preset(name: &str) -> Option<String> {
    load_user_presets().get(name).cloned()
}

/// Save a user preset to ~/.ting/config.toml
pub fn save_user_preset(name: &str, command: &str) -> Result<()> {
    let path = ting_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = if path.exists() {
        std::fs::read_to_string(&path).unwrap_or_default()
    } else {
        String::new()
    };

    let mut table: toml::Table = content.parse().unwrap_or_default();
    let presets = table
        .entry("presets")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if let toml::Value::Table(p) = presets {
        p.insert(name.to_string(), toml::Value::String(command.to_string()));
    }

    let output = toml::to_string_pretty(&table)
        .with_context(|| "Failed to serialize config")?;
    // Atomic write: write to tmp then rename
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, &output)
        .with_context(|| format!("Failed to write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("Failed to rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Parse a CLI participant spec. Supports three formats:
///   - "codex"                  → built-in preset (if name matches)
///   - "name:manual"            → manual participant
///   - "name:command:cmd string" → custom command participant
pub fn parse_participant_spec(spec: &str) -> Result<(String, ParticipantConfig)> {
    let parts: Vec<&str> = spec.splitn(3, ':').collect();
    match parts.len() {
        1 => {
            // Bare name — check for preset (user presets override built-in)
            let name = parts[0].to_string();
            if let Some((ptype, cmd)) = preset_command(&name) {
                Ok((
                    name,
                    ParticipantConfig {
                        participant_type: ptype,
                        command: Some(cmd),
                    },
                ))
            } else if name == "human" {
                Ok((
                    name,
                    ParticipantConfig {
                        participant_type: "manual".to_string(),
                        command: None,
                    },
                ))
            } else {
                let all = list_all_presets();
                let names: Vec<String> = all.iter().map(|(n, _, _)| n.clone()).collect();
                let mut available = names.join(", ");
                available.push_str(", human");
                anyhow::bail!(
                    "Unknown preset '{}'. Available: {}. Or use name:command:\"cmd\"",
                    name,
                    available,
                )
            }
        }
        2 => {
            let name = parts[0].to_string();
            let ptype = parts[1].to_string();
            if ptype != "manual" {
                anyhow::bail!(
                    "Participant '{}' of type '{}' requires a command (use name:type:command)",
                    name,
                    ptype
                );
            }
            Ok((
                name,
                ParticipantConfig {
                    participant_type: ptype,
                    command: None,
                },
            ))
        }
        3 => {
            let name = parts[0].to_string();
            let ptype = parts[1].to_string();
            let cmd = parts[2].to_string();
            Ok((
                name,
                ParticipantConfig {
                    participant_type: ptype,
                    command: Some(cmd),
                },
            ))
        }
        _ => anyhow::bail!(
            "Invalid participant spec '{}' (expected preset name, name:manual, or name:command:\"cmd\")",
            spec
        ),
    }
}

/// Map model shorthand to actual model ID
pub fn resolve_model(model: &str) -> &str {
    match model {
        "claude-sonnet" => "claude-sonnet-4-6",
        "claude-opus" => "claude-opus-4-6",
        "claude-haiku" => "claude-haiku-4-5-20251001",
        other => other,
    }
}

/// Best-effort extraction of model ID from a preset name or command string.
/// Tries: known presets → --model flag → "ollama run MODEL" → falls back to command string.
pub fn resolve_model_id(preset_name: &str) -> String {
    // Known model IDs for built-in presets
    match preset_name {
        "claude" => return "claude-opus-4-6".to_string(),
        "codex" => return "gpt-5.3-codex".to_string(),
        "gemini" => return "gemini-2.5-pro".to_string(),
        "opencode" => return "kimi-k2.5".to_string(),
        _ => {}
    }

    // Try to extract from the command string
    if let Some((_, cmd)) = preset_command(preset_name) {
        // --model MODEL
        if let Some(model) = extract_flag_value(&cmd, "--model") {
            return model;
        }
        // ollama run MODEL
        if let Some(pos) = cmd.find("ollama run ") {
            let rest = &cmd[pos + 11..];
            let model = rest.split_whitespace().next().unwrap_or("unknown");
            return model.to_string();
        }
        // Fall back to the full command
        return cmd;
    }

    preset_name.to_string()
}

fn extract_flag_value(cmd: &str, flag: &str) -> Option<String> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    parts.iter().position(|&p| p == flag)
        .and_then(|i| parts.get(i + 1))
        .map(|v| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn test_parse_participant_preset_codex() {
        let (name, config) = parse_participant_spec("codex").unwrap();
        assert_eq!(name, "codex");
        assert_eq!(config.participant_type, "command");
        assert!(config.command.as_ref().unwrap().contains("codex exec"));
    }

    #[test]
    fn test_parse_participant_preset_gemini() {
        let (name, config) = parse_participant_spec("gemini").unwrap();
        assert_eq!(name, "gemini");
        assert_eq!(config.participant_type, "command");
        assert!(config.command.as_ref().unwrap().contains("gemini"));
    }

    #[test]
    fn test_parse_participant_preset_human() {
        let (name, config) = parse_participant_spec("human").unwrap();
        assert_eq!(name, "human");
        assert_eq!(config.participant_type, "manual");
        assert!(config.command.is_none());
    }

    #[test]
    fn test_parse_participant_preset_unknown() {
        assert!(parse_participant_spec("unknownmodel").is_err());
    }

    #[test]
    fn test_parse_participant_spec_manual() {
        let (name, config) = parse_participant_spec("human:manual").unwrap();
        assert_eq!(name, "human");
        assert_eq!(config.participant_type, "manual");
        assert!(config.command.is_none());
    }

    #[test]
    fn test_parse_participant_spec_command() {
        let (name, config) =
            parse_participant_spec("v0:command:claude -p '{prompt}'").unwrap();
        assert_eq!(name, "v0");
        assert_eq!(config.participant_type, "command");
        assert_eq!(config.command.unwrap(), "claude -p '{prompt}'");
    }

    #[test]
    fn test_parse_participant_spec_invalid() {
        assert!(parse_participant_spec("onlyname").is_err());
        assert!(parse_participant_spec("name:command").is_err()); // command type needs 3rd part
    }

    #[test]
    fn test_load_config_from_string() {
        let toml_str = r#"
[forum]
id = "test-001"
topic = "Test topic"
created = "2026-03-27T00:00:00Z"
max_rounds = 3

[participants]
names = ["alice", "bob"]

[participants.alice]
type = "command"
command = "echo 'hello'"

[participants.bob]
type = "manual"
"#;
        let config: ForumConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.forum.id, "test-001");
        assert_eq!(config.forum.topic, "Test topic");
        assert_eq!(config.forum.max_rounds, 3);
        assert_eq!(config.participants.names.len(), 2);
        assert_eq!(
            config.participants.configs["alice"].participant_type,
            "command"
        );
        assert_eq!(
            config.participants.configs["alice"].command.as_deref(),
            Some("echo 'hello'")
        );
        assert_eq!(
            config.participants.configs["bob"].participant_type,
            "manual"
        );
        assert!(config.participants.configs["bob"].command.is_none());
        // Defaults
        assert_eq!(config.timing.round_timeout, "5m");
        assert_eq!(config.convergence.threshold, 7);
        assert_eq!(config.synthesis.model, "claude-opus");
    }

    #[test]
    fn test_resolve_model() {
        assert_eq!(resolve_model("claude-sonnet"), "claude-sonnet-4-6");
        assert_eq!(resolve_model("claude-opus"), "claude-opus-4-6");
        assert_eq!(resolve_model("gpt-4o"), "gpt-4o"); // passthrough
    }

    #[test]
    fn test_resolve_model_id_builtins() {
        assert_eq!(resolve_model_id("claude"), "claude-opus-4-6");
        assert_eq!(resolve_model_id("codex"), "gpt-5.3-codex");
        assert_eq!(resolve_model_id("gemini"), "gemini-2.5-pro");
        assert_eq!(resolve_model_id("opencode"), "kimi-k2.5");
    }

    #[test]
    fn test_resolve_model_id_ollama() {
        assert_eq!(resolve_model_id("ollama"), "llama3");
    }

    #[test]
    fn test_resolve_model_id_unknown() {
        // Unknown preset falls back to the name itself
        assert_eq!(resolve_model_id("unknownpreset"), "unknownpreset");
    }

    #[test]
    fn test_validate_id_rejects_traversal() {
        assert!(validate_id("../etc", "test").is_err());
        assert!(validate_id("foo/../bar", "test").is_err());
        assert!(validate_id(".", "test").is_err());
        assert!(validate_id("..", "test").is_err());
        assert!(validate_id("foo/bar", "test").is_err());
        assert!(validate_id("", "test").is_err());
        assert!(validate_id("foo\0bar", "test").is_err());
    }

    #[test]
    fn test_validate_id_accepts_valid() {
        assert!(validate_id("ting-2026-03-27-001", "forum ID").is_ok());
        assert!(validate_id("alice", "participant name").is_ok());
        assert!(validate_id("agent-v0", "participant name").is_ok());
        assert!(validate_id("model_1", "participant name").is_ok());
    }

    #[test]
    fn test_validate_id_rejects_uppercase_participant() {
        assert!(validate_id("Alice", "participant name").is_err());
        assert!(validate_id("Agent-V0", "participant name").is_err());
    }

    #[test]
    fn test_validate_reserved_participant_names() {
        let toml_str = r#"
[forum]
id = "test-001"
topic = "Test"
created = "2026-03-27T00:00:00Z"
max_rounds = 3

[participants]
names = ["prompt"]

[participants.prompt]
type = "manual"
"#;
        let config: ForumConfig = toml::from_str(toml_str).unwrap();
        assert!(validate(&config).is_err());
    }

    #[test]
    fn test_validate_threshold_zero_rejected() {
        let toml_str = r#"
[forum]
id = "test-001"
topic = "Test"
created = "2026-03-27T00:00:00Z"
max_rounds = 3

[participants]
names = ["alice"]

[participants.alice]
type = "manual"

[convergence]
threshold = 0
"#;
        let config: ForumConfig = toml::from_str(toml_str).unwrap();
        assert!(validate(&config).is_err());
    }
}
