use serde::{Deserialize, Serialize};

/// The top-level `yes` object in settings.json.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct YesConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cmd: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub url: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
}

impl YesConfig {
    /// Returns true if all five dimensions are empty (no rules configured).
    pub fn is_empty(&self) -> bool {
        self.cmd.is_empty()
            && self.files.is_empty()
            && self.url.is_empty()
            && self.imports.is_empty()
            && self.env.is_empty()
    }
}

/// Items extracted from a tool invocation, organized by the five dimensions.
#[derive(Debug, Clone, Default)]
pub struct ExtractedItems {
    pub cmd: Vec<String>,
    pub files: Vec<String>,
    pub url: Vec<String>,
    pub imports: Vec<String>,
    pub env: Vec<String>,
}

impl ExtractedItems {
    /// Returns true if all five dimensions are empty (nothing extracted).
    pub fn is_empty(&self) -> bool {
        self.cmd.is_empty()
            && self.files.is_empty()
            && self.url.is_empty()
            && self.imports.is_empty()
            && self.env.is_empty()
    }
}

/// Hook input JSON received via stdin from Claude Code.
#[derive(Debug, Deserialize)]
pub struct HookInput {
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
}

/// Decision output written to stdout.
#[derive(Debug, Serialize)]
pub struct Decision {
    pub decision: String, // "approve" or "delegate"
}

/// Top-level wrapper for settings.json containing the optional `yes` block.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SettingsFile {
    #[serde(default)]
    pub yes: Option<YesConfig>,
    #[serde(default)]
    pub permissions: Option<PermissionsSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionsSection {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
}
