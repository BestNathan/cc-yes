use std::io::{self, Read};
use std::path::Path;
use crate::config::{HookInput, HookSpecificOutput};
use crate::log;
use crate::parser;
use crate::matcher;
use crate::settings;

/// Run the PreToolUse hook logic.
/// Reads HookInput from stdin, outputs hookSpecificOutput for approve,
/// or exits silently (exit 0) to delegate to normal permission flow.
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

    // Check all extracted items against rules
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
    } else {
        // Delegate — not in yes rules, let Claude show permission prompt
        log::log_decision(&input.tool_name, &command_str, "delegate", "some items not in allowlist");
        if let Some(session_id) = &input.session_id {
            snapshot_permissions(&local_path, session_id);
        }
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
