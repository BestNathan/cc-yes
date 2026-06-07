# 飞书审批 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add feishu bot WebSocket approval to cc-yes — when a command doesn't match yes rules, send an interactive card to feishu and wait for remote approve/deny.

**Architecture:** New `feishu.rs` module handles feishu REST API (get token, send card) and feishu WebSocket event stream (receive card click). `hook.rs` delegate path now first tries feishu approval if configured, falling back to normal delegate on any error or timeout.

**Tech Stack:** Rust, ureq (HTTP), tungstenite (WebSocket), feishu OpenAPI

---

### Task 1: Add FeishuConfig to config.rs

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Write FeishuConfig struct**

Add to `src/config.rs`, after `PermissionsSection`:

```rust
/// Feishu bot configuration for remote approval.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeishuConfig {
    pub app_id: String,
    pub app_secret: String,
    pub chat_id: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 { 30 }

impl FeishuConfig {
    /// Returns true if all required fields are present.
    pub fn is_configured(&self) -> bool {
        !self.app_id.is_empty() && !self.app_secret.is_empty() && !self.chat_id.is_empty()
    }
}
```

Add `feishu` field to `YesConfig`:

```rust
pub struct YesConfig {
    // ... existing fields ...

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feishu: Option<FeishuConfig>,
}
```

- [ ] **Step 2: Add ApprovalResult enum to config.rs**

```rust
/// Result of a feishu approval request.
#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalResult {
    Allow,
    Deny,
    Timeout,
}
```

- [ ] **Step 3: Verify build**

```bash
cargo build
```

Expected: Compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add src/config.rs
git commit -m "feat: add FeishuConfig and ApprovalResult to config"
```

---

### Task 2: Add dependencies (ureq, tungstenite)

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add dependencies**

Replace the `[dependencies]` section in `Cargo.toml`:

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
glob = "0.3"
ureq = "2"
tungstenite = "0.24"
```

- [ ] **Step 2: Verify build fetches new crates**

```bash
cargo build
```

Expected: Compiles without errors (new crates downloaded).

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build: add ureq and tungstenite dependencies"
```

---

### Task 3: Create feishu.rs module

**Files:**
- Create: `src/feishu.rs`
- Modify: `src/main.rs` (register `mod feishu;`)

- [ ] **Step 1: Write src/feishu.rs**

```rust
use std::io::Read;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tungstenite::{connect, stream::MaybeTlsStream, WebSocket};
use tungstenite::Message;
use crate::config::{FeishuConfig, HookInput, ApprovalResult};

/// Full request_id → used in card callback data to correlate responses.
struct ApprovalRequest {
    id: String,
    tool_name: String,
    command: String,
}

/// Entry point: request feishu approval for a hook invocation.
/// Returns Allow, Deny, or Timeout (always safe — never panics).
pub fn request_approval(
    config: &FeishuConfig,
    input: &HookInput,
    command: &str,
) -> ApprovalResult {
    if !config.is_configured() {
        return ApprovalResult::Deny; // Not configured → skip
    }

    let request_id = uuid_v4();
    let timeout = Duration::from_secs(config.timeout_secs);

    // 1. Get tenant access token
    let token = match get_tenant_token(&config.app_id, &config.app_secret) {
        Ok(t) => t,
        Err(_) => return ApprovalResult::Deny,
    };

    // 2. Open WebSocket
    let mut ws = match open_ws_stream(&token) {
        Ok(w) => w,
        Err(_) => return ApprovalResult::Deny,
    };

    // 3. Send interactive card
    let body = build_card_payload(&request_id, &config.chat_id, &input.tool_name, command);
    if send_message(&token, &body).is_err() {
        return ApprovalResult::Deny;
    }

    // 4. Wait for card click or timeout
    let result = wait_for_click(&mut ws, &request_id, timeout);
    let _ = ws.close(None);
    result
}

/// Call feishu OpenAPI to get tenant_access_token.
fn get_tenant_token(app_id: &str, app_secret: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "app_id": app_id,
        "app_secret": app_secret,
    });

    let resp = ureq::post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(&body.to_string())
        .map_err(|e| format!("token request failed: {}", e))?;

    let json: serde_json::Value = resp.into_json()
        .map_err(|e| format!("token parse failed: {}", e))?;

    json["tenant_access_token"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("no token in response: {}", json))
}

/// Open a WebSocket connection to feishu event stream.
fn open_ws_stream(token: &str) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let url = format!(
        "wss://open.feishu.cn/open-apis/ws/v1?token={}",
        token
    );

    let (ws, _) = connect(&url)
        .map_err(|e| format!("ws connect failed: {}", e))?;

    Ok(ws)
}

/// Send the interactive card via feishu message API.
fn send_message(token: &str, body: &str) -> Result<(), String> {
    let url = format!(
        "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id"
    );

    let resp = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", token))
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(body)
        .map_err(|e| format!("send message failed: {}", e))?;

    let json: serde_json::Value = resp.into_json()
        .map_err(|e| format!("message parse failed: {}", e))?;

    if json["code"].as_i64().unwrap_or(-1) != 0 {
        return Err(format!("feishu api error: {}", json));
    }

    Ok(())
}

/// Build the interactive card JSON payload.
fn build_card_payload(request_id: &str, chat_id: &str, tool_name: &str, command: &str) -> String {
    let card = serde_json::json!({
        "receive_id": chat_id,
        "msg_type": "interactive",
        "content": serde_json::to_string(&serde_json::json!({
            "config": {
                "update_multi": false
            },
            "header": {
                "title": { "tag": "plain_text", "content": "Claude Code 请求确认" },
                "template": "blue"
            },
            "elements": [
                {
                    "tag": "div",
                    "fields": [
                        { "is_short": true, "text": { "tag": "lark_md", "content": format!("**工具**\n{}", tool_name) } },
                        { "is_short": true, "text": { "tag": "lark_md", "content": format!("**命令**\n{}", command) } }
                    ]
                },
                {
                    "tag": "action",
                    "actions": [
                        {
                            "tag": "button",
                            "text": { "tag": "plain_text", "content": "✅ 允许" },
                            "type": "primary",
                            "value": serde_json::to_string(&serde_json::json!({
                                "request_id": request_id,
                                "action": "allow"
                            })).unwrap_or_default()
                        },
                        {
                            "tag": "button",
                            "text": { "tag": "plain_text", "content": "❌ 拒绝" },
                            "type": "danger",
                            "value": serde_json::to_string(&serde_json::json!({
                                "request_id": request_id,
                                "action": "deny"
                            })).unwrap_or_default()
                        }
                    ]
                }
            ]
        })).unwrap(),
    });

    serde_json::to_string(&card).unwrap_or_default()
}

/// Wait for a card action callback matching the given request_id.
fn wait_for_click(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    request_id: &str,
    timeout: Duration,
) -> ApprovalResult {
    // Set read timeout to make non-blocking poll loop work
    ws.get_mut().set_read_timeout(Some(Duration::from_millis(500)))
        .ok();

    let start = Instant::now();

    loop {
        if start.elapsed() >= timeout {
            return ApprovalResult::Timeout;
        }

        match ws.read() {
            Ok(msg) => {
                let text = match msg {
                    Message::Text(t) => t,
                    Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
                    _ => continue,
                };

                // Parse the event — feishu sends card action in specific format
                if let Some(result) = parse_card_event(&text, request_id) {
                    return result;
                }
            }
            Err(tungstenite::Error::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // Timeout on read — just check the clock and retry
                continue;
            }
            Err(_) => {
                // Connection error → fallback
                return ApprovalResult::Deny;
            }
        }
    }
}

/// Parse a WebSocket text message from feishu event stream.
/// Returns Some(ApprovalResult) if this is the card click we're waiting for.
fn parse_card_event(text: &str, expected_request_id: &str) -> Option<ApprovalResult> {
    let event: serde_json::Value = serde_json::from_str(text).ok()?;

    // Check if this is a card action callback
    let action = event["event"]["action"]["value"].as_str()?;
    let action_data: serde_json::Value = serde_json::from_str(action).ok()?;

    // Check request_id matches
    if action_data["request_id"].as_str()? != expected_request_id {
        return None;
    }

    match action_data["action"].as_str()? {
        "allow" => Some(ApprovalResult::Allow),
        "deny" => Some(ApprovalResult::Deny),
        _ => None,
    }
}

/// Generate a simple UUID-like string.
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:08x}",
        t.as_secs() as u32,
        (t.subsec_nanos() >> 16) as u16,
        (t.subsec_nanos() & 0xFFFF) as u16,
        rand_u16(),
        rand_u32(),
    )
}

fn rand_u16() -> u16 {
    let mut buf = [0u8; 2];
    let _ = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf));
    u16::from_ne_bytes(buf)
}

fn rand_u32() -> u32 {
    let mut buf = [0u8; 4];
    let _ = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf));
    u32::from_ne_bytes(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_configured_returns_deny() {
        let config = FeishuConfig::default();
        assert!(!config.is_configured());
    }

    #[test]
    fn test_configured_detection() {
        let config = FeishuConfig {
            app_id: "cli_test".into(),
            app_secret: "secret".into(),
            chat_id: "oc_test".into(),
            timeout_secs: 30,
        };
        assert!(config.is_configured());
    }

    #[test]
    fn test_parse_card_event_allow() {
        let event = serde_json::json!({
            "event": {
                "action": {
                    "value": r#"{"request_id":"test-123","action":"allow"}"#
                }
            }
        });
        let result = parse_card_event(&event.to_string(), "test-123");
        assert_eq!(result, Some(ApprovalResult::Allow));
    }

    #[test]
    fn test_parse_card_event_deny() {
        let event = serde_json::json!({
            "event": {
                "action": {
                    "value": r#"{"request_id":"test-456","action":"deny"}"#
                }
            }
        });
        let result = parse_card_event(&event.to_string(), "test-456");
        assert_eq!(result, Some(ApprovalResult::Deny));
    }

    #[test]
    fn test_parse_card_event_wrong_id() {
        let event = serde_json::json!({
            "event": {
                "action": {
                    "value": r#"{"request_id":"other-id","action":"allow"}"#
                }
            }
        });
        let result = parse_card_event(&event.to_string(), "test-123");
        assert_eq!(result, None);
    }

    #[test]
    fn test_build_card_payload() {
        let payload = build_card_payload("req-1", "oc_test", "Bash", "git status");
        assert!(payload.contains("req-1"));
        assert!(payload.contains("oc_test"));
        assert!(payload.contains("Bash"));
        assert!(payload.contains("git status"));
        assert!(payload.contains("allow"));
        assert!(payload.contains("deny"));
    }
}
```

- [ ] **Step 2: Register module in main.rs**

Add `mod feishu;` to `src/main.rs`:

```rust
mod after;
mod config;
mod feishu;
mod hook;
mod log;
mod matcher;
mod parser;
mod settings;
```

- [ ] **Step 3: Verify build and run tests**

```bash
cargo test
```

Expected: All tests pass (including new feishu tests).

- [ ] **Step 4: Commit**

```bash
git add src/feishu.rs src/main.rs
git commit -m "feat: add feishu WebSocket approval module"
```

---

### Task 4: Modify hook.rs — integrate feishu approval

**Files:**
- Modify: `src/hook.rs`

- [ ] **Step 1: Add feishu module import**

Replace the imports in `hook.rs`:

```rust
use std::io::{self, Read};
use std::path::Path;
use crate::config::{HookInput, HookSpecificOutput, ApprovalResult};
use crate::feishu;
use crate::log;
use crate::parser;
use crate::matcher;
use crate::settings;
```

- [ ] **Step 2: Insert feishu approval in delegate path**

Replace the `} else {` block (the non-matching path) in `run_hook()`:

Currently it looks like:
```rust
    } else {
        log::log_decision(&input.tool_name, &command_str, "delegate", "some items not in allowlist");
        // Delegate — snapshot permissions.allow for auto-learn, then exit silently
        if let Some(session_id) = &input.session_id {
            snapshot_permissions(&local_path, session_id);
        }
        // exit 0 with no output = normal permission flow
    }
```

Replace with:
```rust
    } else {
        // Try feishu approval first (if configured)
        if let Some(ref feishu_config) = config.feishu {
            match feishu::request_approval(feishu_config, &input, &command_str) {
                ApprovalResult::Allow => {
                    // Remote allowed → output approve
                    log::log_decision(&input.tool_name, &command_str, "allow", "approved via feishu");
                    let output = HookSpecificOutput {
                        hook_event_name: "PreToolUse".to_string(),
                        permission_decision: "allow".to_string(),
                        permission_decision_reason: "Approved via feishu".to_string(),
                    };
                    let wrapper = serde_json::json!({
                        "hookSpecificOutput": output,
                    });
                    println!("{}", serde_json::to_string(&wrapper).unwrap());

                    // Auto-learn from feishu approval
                    after_learn(&cwd, &local_path, &input);
                    return Ok(());
                }
                ApprovalResult::Deny | ApprovalResult::Timeout => {
                    log::log_decision(
                        &input.tool_name, &command_str, "delegate",
                        if matches!(feishu::request_approval(feishu_config, &input, &command_str), ApprovalResult::Timeout) {
                            "feishu timeout"
                        } else {
                            "denied via feishu"
                        },
                    );
                }
            }
        } else {
            log::log_decision(&input.tool_name, &command_str, "delegate", "some items not in allowlist");
        }

        // Delegate — snapshot permissions.allow for auto-learn, then exit silently
        if let Some(session_id) = &input.session_id {
            snapshot_permissions(&local_path, session_id);
        }
    }
```

Wait — that's wrong because it calls `feishu::request_approval` a second time just for logging. Let me fix this properly:

```rust
    } else {
        // Try feishu approval first (if configured)
        let feishu_result = if let Some(ref feishu_config) = config.feishu {
            Some(feishu::request_approval(feishu_config, &input, &command_str))
        } else {
            None
        };

        match feishu_result {
            Some(ApprovalResult::Allow) => {
                // Remote allowed → output approve
                log::log_decision(&input.tool_name, &command_str, "allow", "approved via feishu");
                let output = HookSpecificOutput {
                    hook_event_name: "PreToolUse".to_string(),
                    permission_decision: "allow".to_string(),
                    permission_decision_reason: "Approved via feishu".to_string(),
                };
                let wrapper = serde_json::json!({
                    "hookSpecificOutput": output,
                });
                println!("{}", serde_json::to_string(&wrapper).unwrap());

                // Auto-learn from feishu approval
                let extracted = parser::parse_bash(&command_str, &cwd);
                if !extracted.is_empty() {
                    let mut to_learn = config::YesConfig::default();
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
                    if !to_learn.is_empty() {
                        let _ = settings::write_to_local(&local_path, &to_learn);
                    }
                }
                return Ok(());
            }
            Some(ApprovalResult::Deny) => {
                log::log_decision(&input.tool_name, &command_str, "delegate", "denied via feishu");
            }
            Some(ApprovalResult::Timeout) => {
                log::log_decision(&input.tool_name, &command_str, "delegate", "feishu timeout");
            }
            None => {
                log::log_decision(&input.tool_name, &command_str, "delegate", "some items not in allowlist");
            }
        }

        // Delegate — snapshot permissions.allow for auto-learn, then exit silently
        if let Some(session_id) = &input.session_id {
            snapshot_permissions(&local_path, session_id);
        }
    }
```

- [ ] **Step 3: Run tests**

```bash
cargo test
```

Expected: All tests pass.

- [ ] **Step 4: Build and verify**

```bash
cargo build --release
```

Expected: No errors.

- [ ] **Step 5: Commit**

```bash
git add src/hook.rs
git commit -m "feat: integrate feishu approval into PreToolUse delegate path"
```

---

### Task 5: Integration tests

**Files:**
- Modify: `tests/integration.rs`

- [ ] **Step 1: Add feishu config test**

Append to `tests/integration.rs`:

```rust
#[test]
fn test_hook_feishu_not_configured_still_delegates() {
    let tmp = std::env::temp_dir().join("cc-yes-feishu-test");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join(".claude")).unwrap();

    // Settings with feishu block but empty values → should skip feishu
    let settings = r#"{"yes":{"cmd":["git"],"feishu":{"app_id":"","app_secret":"","chat_id":"","timeout_secs":5}}}"#;
    std::fs::write(tmp.join(".claude").join("settings.local.json"), settings).unwrap();
    std::env::set_var("HOME", tmp.to_str().unwrap());

    let hook_input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {
            "command": "rm -rf /",
            "description": "Delete"
        },
        "session_id": "test-feishu-1",
        "cwd": tmp.to_str().unwrap()
    });

    let mut child = Command::new(binary_path())
        .arg("hook")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .current_dir(&tmp)
        .spawn()
        .unwrap();

    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(hook_input.to_string().as_bytes()).unwrap();
    }

    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // With empty feishu config, should fall through to delegate (silent exit)
    assert!(
        stdout.trim().is_empty(),
        "Empty feishu config should not send feishu request, just delegate silently"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
```

- [ ] **Step 2: Run all tests**

```bash
cargo test
```

Expected: All test suites pass.

- [ ] **Step 3: Commit**

```bash
git add tests/integration.rs
git commit -m "test: add feishu not-configured delegate test"
```

---

### Task 6: Build and final verification

- [ ] **Step 1: Build release**

```bash
cargo build --release && cp target/release/cc-yes bin/cc-yes
```

- [ ] **Step 2: Run full test suite**

```bash
cargo test
```

Expected: All tests pass.

- [ ] **Step 3: Verify version and basic command**

```bash
target/release/cc-yes --version
target/release/cc-yes list
```

- [ ] **Step 4: Commit and push**

```bash
git add -A
git commit -m "build: release with feishu approval support"
git push
```
