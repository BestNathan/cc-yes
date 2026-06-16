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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoyes: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feishu: Option<FeishuConfig>,
}

impl YesConfig {
    /// Returns true if all five dimensions are empty AND autoyes is not enabled.
    pub fn is_empty(&self) -> bool {
        self.cmd.is_empty()
            && self.files.is_empty()
            && self.url.is_empty()
            && self.imports.is_empty()
            && self.env.is_empty()
            && self.autoyes != Some(true)
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

/// Output for PreToolUse hook approval.
/// Wrapped in {"hookSpecificOutput": ...} before writing to stdout.
#[derive(Debug, Serialize)]
pub struct HookSpecificOutput {
    #[serde(rename = "hookEventName")]
    pub hook_event_name: String,
    #[serde(rename = "permissionDecision")]
    pub permission_decision: String,
    #[serde(rename = "permissionDecisionReason")]
    pub permission_decision_reason: String,
}

/// Output for PermissionRequest hook — uses nested decision.behavior format
/// required by Claude Code's PermissionRequest hook protocol.
/// Wrapped in {"hookSpecificOutput": ...} before writing to stdout.
#[derive(Debug, Serialize)]
pub struct PermissionRequestOutput {
    #[serde(rename = "hookEventName")]
    pub hook_event_name: String,
    pub decision: PermissionDecision,
}

#[derive(Debug, Serialize)]
pub struct PermissionDecision {
    pub behavior: String,
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

/// Feishu bot configuration for remote approval.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeishuConfig {
    pub app_id: String,
    pub app_secret: String,
    pub chat_id: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 {
    30
}

impl FeishuConfig {
    /// Returns true if all required fields are present.
    pub fn is_configured(&self) -> bool {
        !self.app_id.is_empty() && !self.app_secret.is_empty() && !self.chat_id.is_empty()
    }
}

/// Result of a feishu approval request.
#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalResult {
    Allow,
    Deny,
    Timeout,
}
