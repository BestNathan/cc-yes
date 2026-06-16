//! PermissionRequest hook — fires when Claude is about to show the
//! permission prompt.  Sends a card to Feishu and waits for approval.

use std::io::{self, Read};
use crate::config::{HookInput, PermissionRequestOutput, PermissionDecision, ApprovalResult};
use crate::feishu;
use crate::log;
use crate::settings;

pub fn run_permission_request() -> Result<(), String> {
    let mut input_json = String::new();
    io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|e| format!("Failed to read stdin: {}", e))?;

    let input: HookInput = serde_json::from_str(&input_json)
        .map_err(|e| format!("Failed to parse input: {}", e))?;

    let cwd = match &input.cwd {
        Some(dir) => std::path::Path::new(dir).to_path_buf(),
        None => std::env::current_dir().map_err(|e| format!("No cwd: {}", e))?,
    };

    let Ok((config, local_path)) = settings::load_merged(&cwd) else {
        return Ok(());
    };

    let Some(ref feishu_config) = config.feishu else {
        return Ok(()); // No feishu config → delegate
    };

    if !feishu_config.is_configured() {
        return Ok(());
    }

    let command_str = match input.tool_name.as_str() {
        "Bash" => input.tool_input.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        "Write" | "Edit" | "NotebookEdit" => input.tool_input.get("file_path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        "WebFetch" => input.tool_input.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        _ => format!("{:?}", input.tool_input),
    };

    match feishu::request_approval(feishu_config, &input, &command_str) {
        ApprovalResult::Allow => {
            log::log_decision(&input.tool_name, &command_str, "allow", "approved via feishu");
            let output = PermissionRequestOutput {
                hook_event_name: "PermissionRequest".to_string(),
                decision: PermissionDecision {
                    behavior: "allow".to_string(),
                },
            };
            let wrapper = serde_json::json!({"hookSpecificOutput": output});
            println!("{}", serde_json::to_string(&wrapper).unwrap());

            // Auto-learn
            let extracted = crate::parser::parse_bash(&command_str, &cwd);
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
        }
        ApprovalResult::Deny => {
            log::log_decision(&input.tool_name, &command_str, "delegate", "denied via feishu");
            // Silent exit = delegate
        }
        ApprovalResult::Timeout => {
            log::log_decision(&input.tool_name, &command_str, "delegate", "feishu timeout");
        }
    }

    Ok(())
}
