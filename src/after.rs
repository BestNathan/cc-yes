use std::io::{self, Read};
use std::path::Path;
use crate::config::{HookInput, YesConfig};
use crate::parser;
use crate::settings;
use crate::matcher;

/// Run the PostToolUse auto-learn logic.
/// Reads HookInput from stdin, checks if user clicked "Always allow"
/// by comparing current permissions.allow with snapshot, and if so,
/// learns missing items into yes config.
pub fn run_after() -> Result<(), String> {
    // Check auto-learn env var
    if std::env::var("CC_YES_AUTO_LEARN").as_deref() == Ok("0") {
        return Ok(());
    }

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

    let (_, local_path) = settings::load_merged(&cwd)?;

    // Read snapshot
    let session_id = match &input.session_id {
        Some(id) => id,
        None => return Ok(()), // No session id → can't check snapshot
    };
    let snapshot_path = std::env::temp_dir()
        .join(format!("cc-yes-{}.json", session_id));

    let snapshot: Vec<String> = match std::fs::read_to_string(&snapshot_path) {
        Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
        Err(_) => return Ok(()), // No snapshot → hook approved, nothing to learn
    };

    // Clean up snapshot file
    let _ = std::fs::remove_file(&snapshot_path);

    // Read current permissions.allow
    let current = settings::read_permissions_allow(&local_path).unwrap_or_default();

    // Detect new entries
    let new_entries: Vec<&String> = current.iter().filter(|e| !snapshot.contains(e)).collect();

    if new_entries.is_empty() {
        return Ok(()); // No new permissions → user clicked "Yes" (one-time), not "Always allow"
    }

    // User clicked "Always allow" — learn from the command
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

    if extracted.is_empty() {
        return Ok(()); // Nothing to learn
    }

    // Build a YesConfig from the extracted items that are NOT already in config
    let (config, _) = settings::load_merged(&cwd)?;
    let mut to_learn = YesConfig::default();

    for cmd in &extracted.cmd {
        if !matcher::match_single(cmd, &config.cmd) {
            to_learn.cmd.push(cmd.clone());
        }
    }
    for file in &extracted.files {
        if !matcher::match_single(file, &config.files) {
            to_learn.files.push(file.clone());
        }
    }
    for url in &extracted.url {
        if !matcher::match_single(url, &config.url) {
            to_learn.url.push(url.clone());
        }
    }
    for import in &extracted.imports {
        if !matcher::match_single(import, &config.imports) {
            to_learn.imports.push(import.clone());
        }
    }
    for env in &extracted.env {
        if !matcher::match_single(env, &config.env) {
            to_learn.env.push(env.clone());
        }
    }

    if !to_learn.is_empty() {
        settings::write_to_local(&local_path, &to_learn)?;
    }

    Ok(())
}
