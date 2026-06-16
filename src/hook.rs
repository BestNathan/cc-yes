use std::io::{self, Read};
use std::path::Path;
use crate::config::{HookInput, HookSpecificOutput, ApprovalResult};
use crate::log;
use crate::parser;
use crate::matcher;
use crate::settings;
use crate::feishu;

/// Run the PreToolUse hook logic.
/// 1. Check yes rules → approve if all match.
/// 2. If no match and feishu is configured → send card, wait for approval.
/// 3. Otherwise → delegate to normal permission flow.
pub fn run_hook() -> Result<(), String> {
    // Read stdin
    let mut input_json = String::new();
    io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|e| format!("Failed to read stdin: {}", e))?;

    let input: HookInput = serde_json::from_str(&input_json)
        .map_err(|e| format!("Failed to parse hook input: {}", e))?;

    let cwd = match &input.cwd {
        Some(dir) => Path::new(dir).to_path_buf(),
        None => std::env::current_dir().map_err(|e| format!("No cwd: {}", e))?,
    };

    // Load merged yes config
    let Ok((config, local_path)) = settings::load_merged(&cwd) else {
        return Ok(()); // Can't load config → silent exit, normal flow
    };

    // If no yes config at all, exit silently
    if config.is_empty() {
        return Ok(());
    }

    // Autoyes: if enabled, approve everything
    if config.autoyes == Some(true) {
        let command_str = match input.tool_name.as_str() {
            "Bash" => input.tool_input.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            "Write" | "Edit" | "NotebookEdit" => input.tool_input.get("file_path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            "WebFetch" => input.tool_input.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            _ => format!("{:?}", input.tool_input),
        };

        log::log_decision(&input.tool_name, &command_str, "allow", "autoyes enabled");

        // Send feishu notification if configured
        if let Some(ref feishu_config) = config.feishu {
            if feishu_config.is_configured() {
                feishu::send_autoyes_notification(feishu_config, &input, &command_str);
            }
        }

        let output = HookSpecificOutput {
            hook_event_name: "PreToolUse".to_string(),
            permission_decision: "allow".to_string(),
            permission_decision_reason: "Auto-allowed (autoyes enabled)".to_string(),
        };
        let wrapper = serde_json::json!({
            "hookSpecificOutput": output,
        });
        println!("{}", serde_json::to_string(&wrapper).unwrap());
        return Ok(());
    }

    // Extract items from tool input
    let extracted = match input.tool_name.as_str() {
        "Bash" => {
            let command = input.tool_input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            parser::parse_bash(command, &cwd)
        }
        other => parser::parse_tool(other, &input.tool_input, &cwd),
    };

    // If nothing extractable, exit silently
    if extracted.is_empty() {
        return Ok(());
    }

    let command_str = match input.tool_name.as_str() {
        "Bash" => input.tool_input.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        "Write" | "Edit" | "NotebookEdit" => input.tool_input.get("file_path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        "WebFetch" => input.tool_input.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        _ => format!("{:?}", input.tool_input),
    };

    // Step 1: Check all extracted items against yes rules
    if matcher::matches_all(&extracted, &config) {
        log::log_decision(&input.tool_name, &command_str, "allow", "all dimensions matched");
        let output = HookSpecificOutput {
            hook_event_name: "PreToolUse".to_string(),
            permission_decision: "allow".to_string(),
            permission_decision_reason: "All commands, files, URLs, imports, and env vars match yes rules".to_string(),
        };
        let wrapper = serde_json::json!({
            "hookSpecificOutput": output,
        });
        println!("{}", serde_json::to_string(&wrapper).unwrap());
        return Ok(());
    }

    // Step 2: Not in yes rules — try feishu if configured
    if let Some(ref feishu_config) = config.feishu {
        if feishu_config.is_configured() {
            match feishu::request_approval(feishu_config, &input, &command_str) {
                ApprovalResult::Allow => {
                    log::log_decision(&input.tool_name, &command_str, "allow", "approved via feishu");
                    let output = HookSpecificOutput {
                        hook_event_name: "PreToolUse".to_string(),
                        permission_decision: "allow".to_string(),
                        permission_decision_reason: "Approved via feishu".to_string(),
                    };
                    let wrapper = serde_json::json!({"hookSpecificOutput": output});
                    println!("{}", serde_json::to_string(&wrapper).unwrap());

                    // Auto-learn into yes rules
                    if !extracted.is_empty() {
                        let mut to_learn = crate::config::YesConfig::default();
                        for cmd in &extracted.cmd {
                            to_learn.cmd.push(cmd.clone());
                        }
                        for file in &extracted.files {
                            to_learn.files.push(file.clone());
                        }
                        if !to_learn.is_empty() {
                            let _ = settings::write_to_local(&local_path, &to_learn);
                        }
                    }
                    return Ok(());
                }
                ApprovalResult::Deny => {
                    log::log_decision(&input.tool_name, &command_str, "delegate", "denied via feishu");
                    return Ok(()); // Delegate to Claude prompt
                }
                ApprovalResult::Timeout => {
                    log::log_decision(&input.tool_name, &command_str, "delegate", "feishu timeout");
                    // Fall through to delegate
                }
            }
        }
    }

    // Step 3: Delegate — not in yes rules and no feishu approval
    log::log_decision(&input.tool_name, &command_str, "delegate", "some items not in allowlist");
    if let Some(session_id) = &input.session_id {
        snapshot_permissions(&local_path, session_id);
    }

    Ok(())
}

/// Write current permissions.allow to a temp file for PostToolUse comparison.
fn snapshot_permissions(local_path: &Path, session_id: &str) {
    let allow = settings::read_permissions_allow(local_path).unwrap_or_default();
    let snapshot_path = std::env::temp_dir()
        .join(format!("cc-yes-{}.json", session_id));
    let json = serde_json::to_string(&allow).unwrap_or_default();
    let _ = std::fs::write(&snapshot_path, json);
}
