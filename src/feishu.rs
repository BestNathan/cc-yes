//! Feishu interactive approval — async implementation using WsClient + EventHandler.
//!
//! Flow: get token -> start WS -> send card -> wait for approval.

use std::sync::Arc;
use std::time::Duration;
use crate::config::{FeishuConfig, HookInput, ApprovalResult};
use crate::ws::{ActionValue, CardActionBody, Event, EventHandler, HandlerRegistry, WsClient, WsConfig};

/// Sync entry point — internal tokio runtime bridges to async implementation.
pub fn request_approval(config: &FeishuConfig, input: &HookInput, command: &str) -> ApprovalResult {
    if !config.is_configured() {
        return ApprovalResult::Deny;
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(request_approval_async(config, input, command))
}

async fn request_approval_async(
    config: &FeishuConfig,
    input: &HookInput,
    command: &str,
) -> ApprovalResult {
    let timeout = Duration::from_secs(config.timeout_secs);
    let request_id = format!("ccyes-{}", std::time::UNIX_EPOCH.elapsed().unwrap().as_secs());

    // 1. Get token (shared for card send + card update)
    let token = match get_token(&config.app_id, &config.app_secret).await {
        Ok(t) => t,
        Err(_) => return ApprovalResult::Deny,
    };

    // 2. Set up handler + start WS (so we're listening before the card arrives)
    let rid = request_id.clone();
    // Channel carries (action, open_message_id) so we can update the card later
    let (result_tx, mut result_rx) = tokio::sync::mpsc::channel::<(String, Option<String>)>(1);

    let registry = Arc::new(HandlerRegistry::new(64));
    registry
        .register(EventHandler::new(move |event: Event| {
            if let Ok(card) = serde_json::from_value::<CardActionBody>(event.event) {
                if let Some(av) = card.action.parse_value::<ActionValue>() {
                    if av.request_id == rid {
                        let action = av.action.clone();
                        let msg_id = card.context.as_ref()
                            .and_then(|c| c.open_message_id.clone());
                        let _ = result_tx.try_send((action.clone(), msg_id));

                        // Return toast in WS response
                        let toast = match action.as_str() {
                            "allow" => r#"{"toast":{"type":"success","content":"已允许"}}"#,
                            _ => r#"{"toast":{"type":"info","content":"已拒绝"}}"#,
                        };
                        return Some(toast.as_bytes().to_vec());
                    }
                }
            }
            None
        }))
        .await;

    let ws_client = WsClient::new(WsConfig {
        app_id: config.app_id.clone(),
        app_secret: config.app_secret.clone(),
        domain: "https://open.feishu.cn".into(),
        registry,
    });

    let ws_handle = tokio::spawn(async move {
        if let Err(e) = ws_client.start().await {
            tracing::error!("feishu ws client error: {}", e);
        }
    });

    // Give WS a moment to connect before sending the card
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 3. Detect project info
    let cwd = input.cwd.as_deref().unwrap_or("");
    let project = std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let branch = std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(cwd)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let card_info = CardInfo {
        request_id: request_id.clone(),
        project,
        branch,
        tool: input.tool_name.clone(),
        cmd: command.to_string(),
        session_id: input.session_id.clone(),
    };
    let body = build_card(&card_info, &config.chat_id);
    let sent_msg_id = match send_msg(&token, &body).await {
        Ok(mid) => mid,
        Err(_) => {
            ws_handle.abort();
            return ApprovalResult::Deny;
        }
    };

    // 4. Race: approval result vs timeout
    let (outcome, action_msg_id) = tokio::select! {
        result = result_rx.recv() => {
            match result {
                Some((action, msg_id)) => {
                    let outcome = match action.as_str() {
                        "allow" => ApprovalResult::Allow,
                        _ => ApprovalResult::Deny,
                    };
                    (outcome, msg_id)
                }
                None => (ApprovalResult::Deny, None),
            }
        }
        _ = tokio::time::sleep(timeout) => {
            (ApprovalResult::Timeout, None)
        }
    };

    ws_handle.abort();

    // 5. Update the card (runtime still alive)
    let update_action = match &outcome {
        ApprovalResult::Allow => Some("allow"),
        ApprovalResult::Deny => Some("deny"),
        ApprovalResult::Timeout => Some("timeout"),
    };
    if let Some(action) = update_action {
        // Use action_msg_id if available, otherwise fall back to sent_msg_id
        let mid = action_msg_id.as_deref().unwrap_or(&sent_msg_id);
        let _ = update_card(&token, mid, action, &card_info).await;
    }

    outcome
}

// ── HTTP helpers ──

async fn get_token(app_id: &str, app_secret: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
        .header("Content-Type", "application/json; charset=utf-8")
        .json(&serde_json::json!({"app_id": app_id, "app_secret": app_secret}))
        .send().await.map_err(|e| format!("token: {}", e))?;
    let j: serde_json::Value = resp.json().await.map_err(|e| format!("json: {}", e))?;
    j["tenant_access_token"].as_str().map(|s| s.to_string()).ok_or("no token".to_string())
}

async fn send_msg(token: &str, body: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id")
        .header("Authorization", &format!("Bearer {}", token))
        .header("Content-Type", "application/json; charset=utf-8")
        .body(body.to_string())
        .send().await.map_err(|e| format!("send: {}", e))?;
    let j: serde_json::Value = resp.json().await.map_err(|e| format!("json: {}", e))?;
    if j["code"].as_i64().unwrap_or(-1) != 0 { return Err(format!("api: {}", j)); }
    j["data"]["message_id"].as_str().map(|s| s.to_string()).ok_or("no message_id".to_string())
}

// ── Card info ──

struct CardInfo {
    request_id: String,
    project: String,
    branch: String,
    tool: String,
    cmd: String,
    session_id: Option<String>,
}

// ── Card builder ──

fn build_card(info: &CardInfo, chat_id: &str) -> String {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let sid = info.session_id.as_deref().unwrap_or("-");
    let branch_display = if info.branch.is_empty() { "-".to_string() } else { info.branch.clone() };
    let title = if info.branch.is_empty() {
        info.project.clone()
    } else {
        format!("{} ({})", info.project, info.branch)
    };

    let card = serde_json::json!({
        "config": {"update_multi": true},
        "header": {
            "title": {"tag": "plain_text", "content": title},
            "template": "blue"
        },
        "elements": [
            {"tag": "hr"},
            {"tag": "div", "fields": [
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**工具**\n{}", info.tool)}},
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**命令**\n{}", info.cmd)}}
            ]},
            {"tag": "div", "fields": [
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**Session**\n{}", sid)}},
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**分支**\n{}", branch_display)}}
            ]},
            {"tag": "hr"},
            {"tag": "note", "elements": [
                {"tag": "plain_text", "content": format!("🕐 {}  ·  request_id: {}", now, info.request_id)}
            ]},
            {"tag": "action", "actions": [
                {"tag": "button", "text": {"tag": "plain_text", "content": "✅ 允许"}, "type": "primary",
                 "value": serde_json::to_string(&serde_json::json!({"request_id":&info.request_id,"action":"allow"})).unwrap()},
                {"tag": "button", "text": {"tag": "plain_text", "content": "❌ 拒绝"}, "type": "danger",
                 "value": serde_json::to_string(&serde_json::json!({"request_id":&info.request_id,"action":"deny"})).unwrap()}
            ]}
        ]
    });
    serde_json::to_string(&serde_json::json!({
        "receive_id": chat_id, "msg_type": "interactive",
        "content": serde_json::to_string(&card).unwrap()
    })).unwrap()
}

// ── Card update ──

async fn update_card(token: &str, message_id: &str, action: &str, info: &CardInfo) -> Result<(), String> {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let sid = info.session_id.as_deref().unwrap_or("-");
    let branch_display = if info.branch.is_empty() { "-".to_string() } else { info.branch.clone() };
    let title = if info.branch.is_empty() {
        info.project.clone()
    } else {
        format!("{} ({})", info.project, info.branch)
    };

    let (status_text, template) = match action {
        "allow" => ("✅ 已允许", "green"),
        "deny" => ("❌ 已拒绝", "red"),
        _ => ("⏰ 已超时", "grey"),
    };

    // Keep original info, remove buttons, show result
    let card = serde_json::json!({
        "config": {"update_multi": true},
        "header": {
            "title": {"tag": "plain_text", "content": format!("{} — {}", title, status_text)},
            "template": template
        },
        "elements": [
            {"tag": "hr"},
            {"tag": "div", "fields": [
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**工具**\n{}", info.tool)}},
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**命令**\n{}", info.cmd)}}
            ]},
            {"tag": "div", "fields": [
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**Session**\n{}", sid)}},
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**分支**\n{}", branch_display)}}
            ]},
            {"tag": "hr"},
            {"tag": "div", "text": {"tag": "lark_md", "content": format!("{}  ·  🕐 {}", status_text, now)}},
            {"tag": "note", "elements": [
                {"tag": "plain_text", "content": format!("request_id: {}", info.request_id)}
            ]}
        ]
    });

    let client = reqwest::Client::new();
    let resp = client
        .patch(&format!(
            "https://open.feishu.cn/open-apis/im/v1/messages/{}",
            message_id
        ))
        .header("Authorization", &format!("Bearer {}", token))
        .header("Content-Type", "application/json; charset=utf-8")
        .json(&serde_json::json!({"content": serde_json::to_string(&card).unwrap()}))
        .send().await.map_err(|e| format!("update card: {}", e))?;

    let j: serde_json::Value = resp.json().await.map_err(|e| format!("json: {}", e))?;
    if j["code"].as_i64().unwrap_or(-1) != 0 {
        return Err(format!("update card api: {}", j));
    }
    Ok(())
}
