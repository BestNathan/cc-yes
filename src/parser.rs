use std::path::Path;
use std::path::PathBuf;
use crate::config::ExtractedItems;

/// Parse a bash command string and extract items across all 5 dimensions.
pub fn parse_bash(command: &str, cwd: &Path) -> ExtractedItems {
    let mut items = ExtractedItems::default();

    // Split command into segments by &&, ||, ;, |
    let segments = split_commands(command);

    for segment in &segments {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Extract env vars: VAR=value or export VAR=value
        extract_env_from_line(trimmed, &mut items.env);

        // Extract the executable (first token after stripping env assignments)
        let clean = strip_env_assignments(trimmed);
        let tokens: Vec<&str> = clean.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }

        let exec = tokens[0];
        let cmd_str = tokens.join(" ");

        // Classify the executable
        if is_url_like(exec) {
            // curl/wget URL
            items.cmd.push(exec.to_string());
            extract_urls(&cmd_str, &mut items.url);
        } else if is_script(exec) {
            // bash script.sh, python script.py, node script.js
            items.cmd.push(exec.to_string());
            let script_path = if tokens.len() > 1 { Some(tokens[1]) } else { None };
            if let Some(sp) = script_path {
                items.files.push(sp.to_string());
                deep_parse_script(cwd, sp, &mut items);
            }
            // Check remaining tokens for files/urls
            for t in &tokens[2..] {
                if is_url_like(t) {
                    items.url.push(t.to_string());
                }
            }
        } else {
            // Regular command
            items.cmd.push(exec.to_string());
            // Check remaining tokens for file-like args and URLs
            for t in &tokens[1..] {
                if is_url_like(t) {
                    items.url.push(t.to_string());
                } else if looks_like_file(t) {
                    items.files.push(t.to_string());
                }
            }
        }
    }

    dedup_vec(&mut items.cmd);
    dedup_vec(&mut items.files);
    dedup_vec(&mut items.url);
    dedup_vec(&mut items.imports);
    dedup_vec(&mut items.env);

    items
}

/// Split a bash command string into individual commands by operators.
fn split_commands(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if i + 1 < chars.len()
            && (chars[i] == '&' || chars[i] == '|')
            && chars[i] == chars[i + 1]
        {
            segments.push(current.trim().to_string());
            current = String::new();
            i += 2;
        } else if chars[i] == ';' {
            segments.push(current.trim().to_string());
            current = String::new();
            i += 1;
        } else if chars[i] == '|' {
            // Pipe: include the pipe in the segment (both sides matter)
            current.push(chars[i]);
            i += 1;
        } else {
            current.push(chars[i]);
            i += 1;
        }
    }
    if !current.trim().is_empty() {
        segments.push(current.trim().to_string());
    }

    segments
}

fn extract_env_from_line(line: &str, env_vars: &mut Vec<String>) {
    // Match "VAR=value" or "export VAR=value" patterns
    for word in line.split_whitespace().filter(|w| *w != "export") {
        if let Some(eq_pos) = word.find('=') {
            let var_name = &word[..eq_pos];
            // Only match valid env var names (alphanumeric + underscore)
            if var_name.chars().all(|c| c.is_alphanumeric() || c == '_') && !var_name.is_empty() {
                env_vars.push(var_name.to_string());
            }
        }
    }
    // Also match $VAR references (not assignments)
    for word in line.split_whitespace() {
        if word.starts_with('$') && word.len() > 1 {
            let var = &word[1..];
            let clean: String = var.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !clean.is_empty() {
                env_vars.push(clean);
            }
        }
    }
}

fn strip_env_assignments(line: &str) -> String {
    let words: Vec<&str> = line.split_whitespace()
        .filter(|w| *w != "export" && !w.contains('='))
        .collect();
    words.join(" ")
}

fn is_url_like(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://") || s.starts_with("ftp://")
}

fn is_script(exec: &str) -> bool {
    matches!(exec, "python" | "python3" | "bash" | "sh" | "node" | "ts-node" | "deno")
}

fn looks_like_file(s: &str) -> bool {
    // Heuristic: contains a dot and doesn't start with -
    !s.starts_with('-') && s.contains('.') && !s.contains("://")
}

fn extract_urls(cmd: &str, urls: &mut Vec<String>) {
    for word in cmd.split_whitespace() {
        if is_url_like(word) {
            urls.push(word.to_string());
        }
    }
}

/// Deep parse a script file for its internal imports, commands, files, and URLs.
fn deep_parse_script(cwd: &Path, script_path: &str, items: &mut ExtractedItems) {
    let full_path = if Path::new(script_path).is_relative() {
        cwd.join(script_path)
    } else {
        PathBuf::from(script_path)
    };

    // Check file size limit (20KB)
    let metadata = match std::fs::metadata(&full_path) {
        Ok(m) => m,
        Err(_) => return, // Can't read → skip
    };
    if metadata.len() > 20 * 1024 {
        return; // Too large → skip, delegate upstream
    }

    let content = match std::fs::read_to_string(&full_path) {
        Ok(c) => c,
        Err(_) => return, // Can't read → skip
    };

    // Check line limit (500 lines)
    if content.lines().count() > 500 {
        return; // Too many lines → skip
    }

    // Check for dynamic constructs that make parsing unreliable
    let has_dynamic = content.contains("import(")
        || (content.contains("require(") && (content.contains("+") || content.contains("${")) )
        || content.contains("eval(");
    if has_dynamic {
        return; // Dynamic imports → skip
    }

    let ext = Path::new(script_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    match ext {
        "py" => parse_python_script(&content, items),
        "sh" => parse_bash_script(&content, items),
        "js" | "ts" => parse_js_script(&content, items),
        _ => {} // Unknown extension → skip deep parse
    }
}

fn parse_python_script(content: &str, items: &mut ExtractedItems) {
    for line in content.lines() {
        let trimmed = line.trim();
        // import X, from X import Y
        if trimmed.starts_with("import ") {
            let module = trimmed.strip_prefix("import ").unwrap();
            items.imports.push(module.split_whitespace().next().unwrap_or("").trim_matches(',').to_string());
        } else if trimmed.starts_with("from ") {
            // from X import Y
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() > 1 {
                items.imports.push(parts[1].to_string());
            }
        }
        // subprocess.run / os.system / open() file paths
        extract_string_args(trimmed, &mut items.cmd, &mut items.files, &mut items.url);
    }
}

fn parse_bash_script(content: &str, items: &mut ExtractedItems) {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Extract commands and file references
        let sub = parse_bash(trimmed, &std::env::current_dir().unwrap_or_default());
        items.cmd.extend(sub.cmd);
        items.files.extend(sub.files);
        items.url.extend(sub.url);
        items.env.extend(sub.env);
    }
}

fn parse_js_script(content: &str, items: &mut ExtractedItems) {
    for line in content.lines() {
        let trimmed = line.trim();
        // require('X') or import 'X'
        if trimmed.contains("require(") {
            extract_require_modules(trimmed, &mut items.imports);
        }
        if trimmed.contains("import ") {
            extract_import_modules(trimmed, &mut items.imports);
        }
        // fs.readFile / fs.writeFile paths
        extract_string_args(trimmed, &mut items.cmd, &mut items.files, &mut items.url);
    }
}

fn extract_require_modules(line: &str, imports: &mut Vec<String>) {
    // Simple: require('module-name')
    for cap in line.split("require(").skip(1) {
        if let Some(end) = cap.find(')') {
            let inner = &cap[..end];
            let module = inner.trim_matches(|c| c == '\'' || c == '"' || c == '`');
            if !module.is_empty() && !module.starts_with('.') && !module.starts_with('/') {
                imports.push(module.to_string());
            }
        }
    }
}

fn extract_import_modules(line: &str, imports: &mut Vec<String>) {
    // import { X } from 'module'  or  import 'module'
    for cap in line.split("from ").skip(1) {
        let module = cap.trim_matches(|c| c == '\'' || c == '"' || c == '`' || c == ';');
        if !module.is_empty() && !module.starts_with('.') && !module.starts_with('/') {
            imports.push(module.to_string());
        }
    }
    // import 'module'
    if line.starts_with("import ") && !line.contains("from ") {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() > 1 {
            let module = parts[1].trim_matches(|c| c == '\'' || c == '"' || c == '`' || c == ';');
            if !module.is_empty() && !module.starts_with('.') && !module.starts_with('/') {
                imports.push(module.to_string());
            }
        }
    }
}

fn extract_string_args(line: &str, _cmds: &mut Vec<String>, files: &mut Vec<String>, urls: &mut Vec<String>) {
    // Extract quoted strings that look like file paths or URLs
    let mut in_string = false;
    let mut quote_char = '"';
    let mut current = String::new();

    for ch in line.chars() {
        if !in_string && (ch == '"' || ch == '\'' || ch == '`') {
            in_string = true;
            quote_char = ch;
        } else if in_string && ch == quote_char {
            in_string = false;
            if is_url_like(&current) {
                urls.push(current.clone());
            } else if current.contains('.') || current.contains('/') {
                files.push(current.clone());
            }
            current.clear();
        } else if in_string {
            current.push(ch);
        }
    }
}

fn dedup_vec(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|item| seen.insert(item.clone()));
}

/// Parse tool input for non-Bash tools (Write, Edit, WebFetch, WebSearch, NotebookEdit).
pub fn parse_tool(tool_name: &str, tool_input: &serde_json::Value, _cwd: &Path) -> ExtractedItems {
    let mut items = ExtractedItems::default();

    match tool_name {
        "Write" | "Edit" | "NotebookEdit" => {
            if let Some(path) = tool_input.get("file_path").and_then(|v| v.as_str()) {
                items.files.push(path.to_string());
            }
        }
        "WebFetch" => {
            if let Some(url) = tool_input.get("url").and_then(|v| v.as_str()) {
                items.url.push(url.to_string());
            }
        }
        "WebSearch" => {
            // No extractable dimensions → items stays empty → delegate
            return items;
        }
        _ => {
            // Unknown tool → empty → delegate
            return items;
        }
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_git() {
        let items = parse_bash("git status", Path::new("/tmp"));
        assert!(items.cmd.contains(&"git".to_string()));
    }

    #[test]
    fn test_parse_with_env() {
        let items = parse_bash("RUST_LOG=debug cargo build", Path::new("/tmp"));
        assert!(items.cmd.contains(&"cargo".to_string()));
        assert!(items.env.contains(&"RUST_LOG".to_string()));
    }

    #[test]
    fn test_parse_compound_command() {
        let items = parse_bash("git pull && cargo build", Path::new("/tmp"));
        assert!(items.cmd.contains(&"git".to_string()));
        assert!(items.cmd.contains(&"cargo".to_string()));
    }

    #[test]
    fn test_parse_with_url() {
        let items = parse_bash("curl -s https://api.example.com/data", Path::new("/tmp"));
        assert!(items.cmd.contains(&"curl".to_string()));
        assert!(items.url.contains(&"https://api.example.com/data".to_string()));
    }

    #[test]
    fn test_parse_python_script() {
        let items = parse_bash("python train.py --epochs 100", Path::new("/tmp"));
        assert!(items.cmd.contains(&"python".to_string()));
        assert!(items.files.iter().any(|f| f.contains("train.py")));
    }
}
