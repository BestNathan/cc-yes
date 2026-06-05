use std::io::{self, Read};
use std::path::Path;
use crate::config::{HookInput, Decision};
use crate::parser;
use crate::matcher;
use crate::settings;

/// Run the PreToolUse hook logic.
/// Reads HookInput from stdin, outputs Decision to stdout.
/// On delegate, writes a snapshot of permissions.allow for later auto-learn detection.
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
    let (config, local_path) = settings::load_merged(&cwd)?;

    // If no yes config at all, delegate
    if config.is_empty() {
        let decision = Decision { decision: "delegate".to_string() };
        println!("{}", serde_json::to_string(&decision).unwrap());
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

    // If nothing extractable, delegate
    if extracted.is_empty() {
        let decision = Decision { decision: "delegate".to_string() };
        println!("{}", serde_json::to_string(&decision).unwrap());
        return Ok(());
    }

    // Check all extracted items against rules
    if matcher::matches_all(&extracted, &config) {
        let decision = Decision { decision: "approve".to_string() };
        println!("{}", serde_json::to_string(&decision).unwrap());
    } else {
        // Delegate — but first snapshot permissions.allow for auto-learn detection
        if let Some(session_id) = &input.session_id {
            snapshot_permissions(&local_path, session_id);
        }
        let decision = Decision { decision: "delegate".to_string() };
        println!("{}", serde_json::to_string(&decision).unwrap());
    }

    Ok(())
}

/// Write current permissions.allow to a temp file for PostToolUse comparison.
fn snapshot_permissions(local_path: &Path, session_id: &str) {
    let allow = settings::read_permissions_allow(local_path).unwrap_or_default();
    let snapshot_path = std::path::PathBuf::from("/tmp")
        .join(format!("cc-yes-{}.json", session_id));
    let json = serde_json::to_string(&allow).unwrap_or_default();
    let _ = std::fs::write(&snapshot_path, json);
}
