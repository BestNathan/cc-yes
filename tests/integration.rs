use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Absolute path to the debug binary, resolved at compile time.
fn binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/cc-yes")
}

#[test]
fn test_hook_approve_simple_git() {
    // Setup: create temp project with yes config that allows git
    let tmp = std::env::temp_dir().join("cc-yes-integration-test");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join(".claude")).unwrap();

    // Write settings.local.json with yes config allowing git
    let settings = r#"{"yes":{"cmd":["git","ls"]}}"#;
    std::fs::write(tmp.join(".claude").join("settings.local.json"), settings).unwrap();

    // Set HOME to tmp so global settings is empty (no ~/.claude/settings.json)
    std::env::set_var("HOME", tmp.to_str().unwrap());

    // Hook input: Bash "git status"
    let hook_input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {
            "command": "git status",
            "description": "Check git status"
        },
        "session_id": "test-session-1",
        "cwd": tmp.to_str().unwrap()
    });

    // Binary is already compiled by `cargo test` – no need for explicit build step.

    // Run cc-yes hook with stdin
    let mut child = Command::new(binary_path())
        .arg("hook")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .current_dir(&tmp)
        .spawn()
        .unwrap();

    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(hook_input.to_string().as_bytes()).unwrap();
    }

    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();

    assert_eq!(
        result["hookSpecificOutput"]["permissionDecision"], "allow",
        "Expected allow for 'git status'"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_hook_delegate_unknown_command() {
    let tmp = std::env::temp_dir().join("cc-yes-integration-unknown");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join(".claude")).unwrap();

    // Settings only allow git
    let settings = r#"{"yes":{"cmd":["git"]}}"#;
    std::fs::write(tmp.join(".claude").join("settings.local.json"), settings).unwrap();
    std::env::set_var("HOME", tmp.to_str().unwrap());

    // Hook input: Bash "rm -rf /"
    let hook_input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {
            "command": "rm -rf /",
            "description": "Delete everything"
        },
        "session_id": "test-session-2",
        "cwd": tmp.to_str().unwrap()
    });

    let mut child = Command::new(binary_path())
        .arg("hook")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .current_dir(&tmp)
        .spawn()
        .unwrap();

    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(hook_input.to_string().as_bytes()).unwrap();
    }

    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Delegate: exit 0 with no decision output — normal permission flow applies
    assert!(
        stdout.trim().is_empty(),
        "Expected silent exit (no output) for delegate, got: {}",
        stdout.trim()
    );

    // Verify snapshot was created
    let snapshot_path = std::env::temp_dir().join("cc-yes-test-session-2.json");
    assert!(snapshot_path.exists(), "Snapshot file should exist after delegate");

    // Clean up
    let _ = std::fs::remove_file(&snapshot_path);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_check_command() {
    let tmp = std::env::temp_dir().join("cc-yes-check-test");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join(".claude")).unwrap();

    let settings = r#"{"yes":{"cmd":["git","cargo build"]}}"#;
    std::fs::write(tmp.join(".claude").join("settings.local.json"), settings).unwrap();
    std::env::set_var("HOME", tmp.to_str().unwrap());

    let output = Command::new(binary_path())
        .args(["check", "git pull && rm -rf /"])
        .current_dir(&tmp)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("git → ✅"), "git should match");
    assert!(stdout.contains("rm → ❌"), "rm should not match");
    assert!(stdout.contains("NOT auto-approve"), "Should indicate delegate");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_hook_feishu_not_configured_still_delegates() {
    let tmp = std::env::temp_dir().join("cc-yes-feishu-test");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join(".claude")).unwrap();

    // Settings with feishu block but empty values → should skip feishu
    let settings = r#"{"yes":{"cmd":["git"],"feishu":{"app_id":"","app_secret":"","chat_id":"","timeout_secs":5}}}"#;
    std::fs::write(tmp.join(".claude").join("settings.local.json"), settings).unwrap();
    std::env::set_var("HOME", tmp.to_str().unwrap());

    let hook_input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {
            "command": "rm -rf /",
            "description": "Delete"
        },
        "session_id": "test-feishu-1",
        "cwd": tmp.to_str().unwrap()
    });

    let mut child = Command::new(binary_path())
        .arg("hook")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .current_dir(&tmp)
        .spawn()
        .unwrap();

    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(hook_input.to_string().as_bytes()).unwrap();
    }

    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // With empty feishu config, should fall through to delegate (silent exit)
    assert!(
        stdout.trim().is_empty(),
        "Empty feishu config should not send feishu request, just delegate silently. Got: {}",
        stdout.trim()
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
