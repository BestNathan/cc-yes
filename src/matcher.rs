use crate::config::{YesConfig, ExtractedItems};

/// Check if all extracted items match the yes rules.
/// Returns true if everything matches → approve.
/// Returns false if anything doesn't match → delegate.
pub fn matches_all(extracted: &ExtractedItems, config: &YesConfig) -> bool {
    // If nothing extracted, delegate (we can't parse this)
    if extracted.is_empty() {
        return false;
    }

    // Check every extracted cmd against cmd rules
    for cmd in &extracted.cmd {
        if !match_item(cmd, &config.cmd) {
            return false;
        }
    }

    // Check every extracted file against files rules
    for file in &extracted.files {
        if !match_item(file, &config.files) {
            return false;
        }
    }

    // Check every extracted url against url rules
    for url in &extracted.url {
        if !match_item(url, &config.url) {
            return false;
        }
    }

    // Check every extracted import against imports rules
    for import in &extracted.imports {
        if !match_item(import, &config.imports) {
            return false;
        }
    }

    // Check every extracted env var against env rules
    for env in &extracted.env {
        if !match_exact(env, &config.env) {
            return false;
        }
    }

    true
}

/// Match an item against a list of rules.
/// Rules support three syntaxes:
///   "git"         → prefix match: "git status" matches "git"
///   "cargo build" → exact match: "cargo build" matches "cargo build", not "cargo check"
///   "npm run dev:*" → glob match: "npm run dev:build" matches "npm run dev:*"
fn match_item(item: &str, rules: &[String]) -> bool {
    if rules.is_empty() {
        return false; // No rules configured → nothing matched
    }

    rules.iter().any(|rule| {
        if rule.contains('*') || rule.contains('?') {
            // Glob pattern matching
            glob_match(rule, item)
        } else if rule.contains(' ') {
            // Multi-word rule → check if item starts with the rule (exact prefix).
            // e.g. "cargo build" matches "cargo build --release" but not "cargo check"
            item == rule
                || item.starts_with(&format!("{} ", rule))
        } else {
            // Single-word rule → prefix match
            item == rule || item.starts_with(&format!("{} ", rule))
        }
    })
}

/// Exact match for env vars and imports (no glob support, exact string comparison).
fn match_exact(item: &str, rules: &[String]) -> bool {
    if rules.is_empty() {
        return false;
    }
    rules.iter().any(|rule| rule == item)
}

/// Check if a single item matches any rule in the list.
/// Used by auto-learn to determine if an item is already covered.
pub fn match_single(item: &str, rules: &[String]) -> bool {
    match_item(item, rules)
}

/// Simple glob matching supporting * and ? wildcards.
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();
    glob_match_recursive(&pattern_chars, &text_chars, 0, 0)
}

fn glob_match_recursive(pattern: &[char], text: &[char], pi: usize, ti: usize) -> bool {
    let mut pi = pi;
    let mut ti = ti;

    while pi < pattern.len() {
        match pattern[pi] {
            '*' => {
                // * matches zero or more characters
                if pi + 1 >= pattern.len() {
                    return true; // trailing * matches everything
                }
                // Try matching the rest of the pattern at each position in text
                for next_ti in ti..=text.len() {
                    if glob_match_recursive(pattern, text, pi + 1, next_ti) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                // ? matches exactly one character
                if ti >= text.len() {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
            ch => {
                if ti >= text.len() || text[ti] != ch {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
        }
    }

    // Reached end of pattern → must also be at end of text
    ti == text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_match() {
        assert!(match_item("git status", &["git".to_string()]));
        assert!(match_item("git", &["git".to_string()]));
        assert!(!match_item("npm run build", &["git".to_string()]));
    }

    #[test]
    fn test_exact_match() {
        assert!(match_item("cargo build", &["cargo build".to_string()]));
        assert!(match_item("cargo build --release", &["cargo build".to_string()]));
        assert!(!match_item("cargo check", &["cargo build".to_string()]));
    }

    #[test]
    fn test_glob_match() {
        assert!(match_item("npm run dev:build", &["npm run dev:*".to_string()]));
        assert!(match_item("npm run dev:watch", &["npm run dev:*".to_string()]));
        assert!(!match_item("npm run prod:build", &["npm run dev:*".to_string()]));
    }

    #[test]
    fn test_file_glob() {
        assert!(match_item("src/main.rs", &["src/**".to_string()]));
        assert!(match_item("src/lib.rs", &["src/**".to_string()]));
        assert!(!match_item("tests/main.rs", &["src/**".to_string()]));
    }

    #[test]
    fn test_url_glob() {
        assert!(match_item("https://docs.rs/serde", &["https://docs.rs/*".to_string()]));
        assert!(!match_item("https://crates.io/serde", &["https://docs.rs/*".to_string()]));
    }

    #[test]
    fn test_env_exact() {
        assert!(match_exact("RUST_LOG", &["RUST_LOG".to_string(), "PATH".to_string()]));
        assert!(!match_exact("AWS_KEY", &["RUST_LOG".to_string()]));
    }

    #[test]
    fn test_full_match_all() {
        let config = YesConfig {
            cmd: vec!["git".to_string(), "cargo build".to_string()],
            files: vec!["*.rs".to_string()],
            url: vec![],
            imports: vec![],
            env: vec!["RUST_LOG".to_string()],
        };

        let extracted = ExtractedItems {
            cmd: vec!["git".to_string(), "cargo build".to_string()],
            files: vec!["src/main.rs".to_string()],
            url: vec![],
            imports: vec![],
            env: vec![],
        };

        assert!(matches_all(&extracted, &config));
    }

    #[test]
    fn test_mismatch_causes_delegate() {
        let config = YesConfig {
            cmd: vec!["git".to_string()],
            files: vec![],
            url: vec![],
            imports: vec![],
            env: vec![],
        };

        let extracted = ExtractedItems {
            cmd: vec!["rm".to_string()],
            files: vec![],
            url: vec![],
            imports: vec![],
            env: vec![],
        };

        assert!(!matches_all(&extracted, &config));
    }

    #[test]
    fn test_empty_extraction_delegates() {
        let config = YesConfig {
            cmd: vec!["git".to_string()],
            files: vec![],
            url: vec![],
            imports: vec![],
            env: vec![],
        };

        let extracted = ExtractedItems::default();
        assert!(!matches_all(&extracted, &config));
    }
}
