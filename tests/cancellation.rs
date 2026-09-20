//! Real signal handling and checkpoint recovery, with no external model calls.
#![cfg(unix)]
use std::{
    fs,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn signals_stop_forums_and_resume_preserves_completed_calls() {
    for (signal, dashboard) in [(libc::SIGINT, false), (libc::SIGTERM, true)] {
        let dir = std::env::temp_dir().join(format!("ting-signal-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let root = dir.display();
        fs::write(
            dir.join("participant.sh"),
            format!("cat >/dev/null\necho called >> '{root}/calls'\necho Answer\n"),
        )
        .unwrap();
        fs::write(dir.join("synth.sh"), format!("cat >/dev/null\nif [ ! -f '{root}/allow' ]; then\n touch '{root}/ready'\n sleep 30 &\n wait\nfi\necho Synthesis\n")).unwrap();
        fs::write(
            dir.join("meta.toml"),
            format!(
                r#"
[forum]
id = "signal-test"
topic = "Signal handling"
created = "2026-09-19T00:00:00Z"
max_rounds = 1
[participants]
names = ["test"]
[participants.test]
type = "command"
command = "sh '{root}/participant.sh'"
[synthesis]
model = "fake"
command = "sh '{root}/synth.sh'"
[convergence]
min_rounds = 1
judge_model = "fake"
judge_command = "printf 'SCORE: 8\\nSUMMARY: Agreement\\nALIGNMENT: test=8\\n'"
"#
            ),
        )
        .unwrap();
        fs::write(
            dir.join("run-options.json"),
            r#"{"version":1,"options":{"classify":false,"score":false,"emit_events":true}}"#,
        )
        .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_ting"));
        command
            .arg("resume")
            .arg(&dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if dashboard {
            command.args(["--dashboard", "--no-open", "--port", "0"]);
        }
        let mut child = command.spawn().unwrap();
        let start = Instant::now();
        while !dir.join("ready").exists() {
            if start.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("fixture failed to become ready: {root}");
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "runner exited early: {root}"
            );
            thread::sleep(Duration::from_millis(20));
        }
        unsafe {
            assert_eq!(libc::kill(child.id() as i32, signal), 0);
        }
        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if start.elapsed() > Duration::from_secs(8) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("cancellation did not clean up command group promptly");
            }
            thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(status.code(), Some(130));
        let status: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("run-status.json")).unwrap())
                .unwrap();
        assert_eq!(status["status"], "interrupted");
        let log = fs::read_to_string(dir.join("dashboard-events.jsonl")).unwrap();
        assert!(log.contains("forum_interrupted"));
        assert!(!log.contains("forum_failed"));
        assert!(dir.join("round-1/.checkpoints/test.md.json").exists());
        fs::write(dir.join("allow"), "").unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_ting"))
            .arg("resume")
            .arg(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(dir.join("calls"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert!(dir.join("final/meta-summary.toml").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
