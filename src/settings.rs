use std::path::{Path, PathBuf};
use std::env;
use crate::config::{SettingsFile, YesConfig};

/// Load and merge settings from 3 layers: global -> project -> local.
/// Returns the merged YesConfig and the path to settings.local.json.
pub fn load_merged(cwd: &Path) -> Result<(YesConfig, PathBuf), String> {
    let home = env::var("HOME").unwrap_or_default();
    let global_path = PathBuf::from(home).join(".claude").join("settings.json");
    let project_path = cwd.join(".claude").join("settings.json");
    let local_path = cwd.join(".claude").join("settings.local.json");

    let mut merged = YesConfig::default();

    // Layer 1: ~/.claude/settings.json (lowest priority)
    if let Ok(settings) = read_settings(&global_path) {
        if let Some(yes) = settings.yes {
            merge_into(&mut merged, &yes);
        }
    }

    // Layer 2: .claude/settings.json (project)
    if let Ok(settings) = read_settings(&project_path) {
        if let Some(yes) = settings.yes {
            merge_into(&mut merged, &yes);
        }
    }

    // Layer 3: .claude/settings.local.json (highest priority)
    if let Ok(settings) = read_settings(&local_path) {
        if let Some(yes) = settings.yes {
            merge_into(&mut merged, &yes);
        }
    }

    Ok((merged, local_path))
}

fn read_settings(path: &Path) -> Result<SettingsFile, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
    serde_json::from_str::<SettingsFile>(&content)
        .map_err(|e| format!("Invalid JSON in {}: {}", path.display(), e))
}

/// Deep merge: append arrays from `other` into `base`, deduplicating.
fn merge_into(base: &mut YesConfig, other: &YesConfig) {
    base.cmd.extend(other.cmd.iter().cloned());
    base.files.extend(other.files.iter().cloned());
    base.url.extend(other.url.iter().cloned());
    base.imports.extend(other.imports.iter().cloned());
    base.env.extend(other.env.iter().cloned());
    // feishu: higher-priority layer overrides
    if other.feishu.is_some() {
        base.feishu = other.feishu.clone();
    }
    // autoyes: higher-priority layer overrides
    if other.autoyes.is_some() {
        base.autoyes = other.autoyes;
    }

    dedup(&mut base.cmd);
    dedup(&mut base.files);
    dedup(&mut base.url);
    dedup(&mut base.imports);
    dedup(&mut base.env);
}

fn dedup(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|item| seen.insert(item.clone()));
}

/// Read permissions.allow from settings.local.json.
pub fn read_permissions_allow(local_path: &Path) -> Result<Vec<String>, String> {
    match read_settings(local_path) {
        Ok(settings) => {
            Ok(settings.permissions
                .map(|p| p.allow)
                .unwrap_or_default())
        }
        Err(_) => Ok(Vec::new()),
    }
}

/// Write YesConfig to settings.local.json, merging with existing content.
pub fn write_to_local(local_path: &Path, new_yes: &YesConfig) -> Result<(), String> {
    // Read existing file (or start fresh)
    let mut existing: SettingsFile = match std::fs::read_to_string(local_path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => SettingsFile::default(),
    };

    // Merge new_yes into existing yes block
    let mut merged_yes = existing.yes.unwrap_or_default();
    merge_into(&mut merged_yes, new_yes);
    existing.yes = Some(merged_yes);

    // Ensure parent directory exists
    if let Some(parent) = local_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create dir {}: {}", parent.display(), e))?;
    }

    let json = serde_json::to_string_pretty(&existing)
        .map_err(|e| format!("Cannot serialize: {}", e))?;
    std::fs::write(local_path, json)
        .map_err(|e| format!("Cannot write {}: {}", local_path.display(), e))
}

/// Remove a specific rule from a dimension in settings.local.json.
pub fn remove_from_local(local_path: &Path, dimension: &str, rule: &str) -> Result<(), String> {
    let mut existing: SettingsFile = match std::fs::read_to_string(local_path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => return Ok(()), // File doesn't exist, nothing to remove
    };

    let mut yes = existing.yes.unwrap_or_default();
    match dimension {
        "cmd" => yes.cmd.retain(|r| r != rule),
        "files" => yes.files.retain(|r| r != rule),
        "url" => yes.url.retain(|r| r != rule),
        "imports" => yes.imports.retain(|r| r != rule),
        "env" => yes.env.retain(|r| r != rule),
        _ => return Err(format!("Unknown dimension: {}", dimension)),
    }
    existing.yes = if yes.is_empty() { None } else { Some(yes) };

    let json = serde_json::to_string_pretty(&existing)
        .map_err(|e| format!("Cannot serialize: {}", e))?;
    std::fs::write(local_path, json)
        .map_err(|e| format!("Cannot write {}: {}", local_path.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn test_three_layer_merge() {
        let tmp = std::env::temp_dir().join("cc-yes-test-merge");
        let _ = std::fs::remove_dir_all(&tmp);

        // Layer 1: global
        write_temp_file(&tmp, "home/.claude/settings.json", r#"{"yes":{"cmd":["git","ls"]}}"#);

        // Layer 2: project
        write_temp_file(&tmp, "project/.claude/settings.json", r#"{"yes":{"cmd":["cargo"],"files":["*.rs"]}}"#);

        // Layer 3: local
        write_temp_file(&tmp, "project/.claude/settings.local.json", r#"{"yes":{"cmd":["npm"],"url":["https://api.github.com/*"]}}"#);

        // Override HOME for this test
        std::env::set_var("HOME", tmp.join("home").to_str().unwrap());

        let (merged, _) = load_merged(&tmp.join("project")).unwrap();
        assert!(merged.cmd.contains(&"git".to_string()));
        assert!(merged.cmd.contains(&"ls".to_string()));
        assert!(merged.cmd.contains(&"cargo".to_string()));
        assert!(merged.cmd.contains(&"npm".to_string()));
        assert!(merged.files.contains(&"*.rs".to_string()));
        assert!(merged.url.contains(&"https://api.github.com/*".to_string()));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_empty_config() {
        let tmp = std::env::temp_dir().join("cc-yes-test-empty");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join(".claude")).unwrap();
        std::env::set_var("HOME", tmp.to_str().unwrap());

        let (merged, _) = load_merged(&tmp).unwrap();
        assert!(merged.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
