//! Read-only prerequisite checks. Never run presets or contact model providers.

use crate::config;
use anyhow::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct Report {
    /// True only means the inspected executables exist; authentication is separate.
    pub ok: bool,
    pub checks: Vec<Check>,
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub name: String,
    pub status: &'static str,
    pub detail: String,
}

/// Inspect prerequisites for default live runs and the requested participants.
pub fn inspect(participants: &[String]) -> Result<Report> {
    let paths: Vec<PathBuf> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    let mut checks = vec![binary_check("claude", &paths), binary_check("sh", &paths)];
    for spec in participants {
        let (name, participant) = config::parse_participant_spec(spec)?;
        if participant.participant_type == "manual" {
            checks.push(Check {
                name,
                status: "manual",
                detail: "Human participant; no executable needed.".into(),
            });
        } else if let Some((_, built_in)) = config::builtin_preset(&name)
            && participant.command.as_deref() == Some(built_in)
        {
            if !checks.iter().any(|check| check.name == name) {
                checks.push(binary_check(&name, &paths));
            }
            if built_in.contains("cat {prompt_file}")
                && !checks.iter().any(|check| check.name == "cat")
            {
                checks.push(binary_check("cat", &paths));
            }
        } else {
            checks.push(Check { name, status: "manual", detail: "Custom shell command: inspect its dependencies and authentication manually. It was not executed.".into() });
        }
    }
    let ok = !checks.iter().any(|check| check.status == "missing");
    Ok(Report {
        ok, checks,
        notes: vec![
            "Default live runs use Claude for synthesis and judging, even with human or local-model participants.".into(),
            "This checks executable files only. Authenticate each CLI and verify its tool permissions before a live run; no commands or network calls were made.".into(),
            "Try `ting demo` without installing or authenticating any model CLI.".into(),
        ],
    })
}

fn binary_check(name: &str, paths: &[PathBuf]) -> Check {
    match paths
        .iter()
        .map(|path| path.join(name))
        .find(|path| executable(path))
    {
        Some(path) => Check {
            name: name.into(),
            status: "found",
            detail: path.display().to_string(),
        },
        None => Check {
            name: name.into(),
            status: "missing",
            detail: format!("Install `{name}` and make sure its directory is on PATH."),
        },
    }
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_executable_files_without_running_them() {
        let dir = std::env::temp_dir().join(format!("ting-doctor-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("fake-cli");
        std::fs::write(&program, "this is deliberately not a valid program").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(
                binary_check("fake-cli", std::slice::from_ref(&dir)).status,
                "missing"
            );
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(
            binary_check("fake-cli", std::slice::from_ref(&dir)).status,
            "found"
        );
        assert_eq!(
            binary_check("absent", std::slice::from_ref(&dir)).status,
            "missing"
        );
        assert!(!executable(&dir));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn custom_commands_are_reported_without_execution() {
        let report = inspect(&["custom:command:exit 99".into(), "person:manual".into()]).unwrap();
        assert!(report.checks.iter().any(|check| check.name == "custom"
            && check.status == "manual"
            && check.detail.contains("not executed")));
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.name == "person" && check.status == "manual")
        );
        assert!(serde_json::to_value(report).unwrap().get("ok").is_some());
    }
}
