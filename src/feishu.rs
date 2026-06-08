//! Feishu interactive approval — async implementation using WsClient + HandlerRegistry.
//!
//! Flow: get token -> send card -> start WS with EventHandler -> wait for approval.
//!
//! The outer `request_approval` is SYNC (preserving backward compatibility with hook.rs).
//! It creates a tokio runtime internally and calls the async inner function via block_on.

use std::sync::Arc;
use std::time::Duration;
use serde_json::Value;
use crate::config::{FeishuConfig, HookInput, ApprovalResult};
use crate::ws::{WsClient, WsConfig, HandlerRegistry, EventHandler};

/// Sync entry point — internal tokio runtime bridges to async implementation.
pub fn request_approval(config: &FeishuConfig, input: &HookInput, command: &str) -> ApprovalResult {
    if !config.is_configured() {
        return ApprovalResult::Deny;
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(request_approval_async(config, input, command))
}

/// Inner async implementation using the new WsClient + EventHandler + tokio::select! race.
async fn request_approval_async(
    config: &FeishuConfig,
    input: &HookInput,
    command: &str,
) -> ApprovalResult {
    let timeout = Duration::from_secs(config.timeout_secs);
    let request_id = format!(
        "ccyes-{}",
        std::time::UNIX_EPOCH.elapsed().unwrap().as_secs()
    );

    // 1. Get token
    let token = match get_token(&config.app_id, &config.app_secret).await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("feishu get_token failed: {}", e);
            return ApprovalResult::Deny;
        }
    };

    // 2. Send interactive card
    let body = build_card(&request_id, &config.chat_id, &input.tool_name, command);
    if let Err(e) = send_msg(&token, &body).await {
        tracing::error!("feishu send_msg failed: {}", e);
        return ApprovalResult::Deny;
    }

    // 3. Set up HandlerRegistry with approval listener
    let rid = request_id.clone();
    let (result_tx, mut result_rx) = tokio::sync::mpsc::channel(1);

    let registry = Arc::new(HandlerRegistry::new(64));
    registry
        .register(EventHandler::new(move |event: Value| {
            let action = parse_card_action(&event);
            if let Some(action) = action {
                if action.request_id == rid {
                    let result = match action.action.as_str() {
                        "allow" => "allow",
                        _ => "deny",
                    };
                    let _ = result_tx.try_send(result.to_string());
                }
            }
            None
        }))
        .await;

    // 4. Spawn WS client in background
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

    // 5. Race: approval result vs timeout
    let outcome = tokio::select! {
        result = result_rx.recv() => {
            match result.as_deref() {
                Some("allow") => ApprovalResult::Allow,
                _ => ApprovalResult::Deny,
            }
        }
        _ = tokio::time::sleep(timeout) => {
            ApprovalResult::Timeout
        }
    };

    ws_handle.abort();
    outcome
}

// ── Card action parsing ──

/// Parsed card.action.trigger event.
struct CardAction {
    action: String,
    request_id: String,
}

/// Parse card.action.trigger from an already-parsed JSON Value (as received
/// from EventHandler callback). `event.event.action.value` is double-JSON-encoded:
///   raw: `"{\\"action\\":\\"allow\\",\\"request_id\\":\\"...\\"}"`
///   First parse -> JSON string `{"action":"allow","request_id":"..."}`
///   Second parse -> JSON object {action: "allow", request_id: "..."}
fn parse_card_action(event: &Value) -> Option<CardAction> {
    if event["header"]["event_type"].as_str()? != "card.action.trigger" {
        return None;
    }
    let value_str = event["event"]["action"]["value"].as_str()?;
    // value_str is a JSON-encoded string — parse as String first
    let inner_json: String = serde_json::from_str(value_str).ok()?;
    // inner_json is the actual action object
    let action: Value = serde_json::from_str(&inner_json).ok()?;
    Some(CardAction {
        action: action["action"].as_str()?.to_string(),
        request_id: action["request_id"].as_str()?.to_string(),
    })
}

// ── HTTP helpers (reqwest) ──

/// Get tenant access token from Feishu Open API.
async fn get_token(app_id: &str, app_secret: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
        .header("Content-Type", "application/json; charset=utf-8")
        .json(&serde_json::json!({"app_id": app_id, "app_secret": app_secret}))
        .send()
        .await
        .map_err(|e| format!("token request failed: {}", e))?;
    let j: Value = resp.json().await.map_err(|e| format!("token json: {}", e))?;
    j["tenant_access_token"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "no tenant_access_token in response".to_string())
}

/// Send an interactive card message to a Feishu chat.
async fn send_msg(token: &str, body: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id")
        .header("Authorization", &format!("Bearer {}", token))
        .header("Content-Type", "application/json; charset=utf-8")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| format!("send request failed: {}", e))?;
    let j: Value = resp.json().await.map_err(|e| format!("send json: {}", e))?;
    if j["code"].as_i64().unwrap_or(-1) != 0 {
        return Err(format!("api error: {}", j));
    }
    Ok(())
}

// ── Card builder (unchanged) ──

/// Build the interactive card message body for approval.
fn build_card(rid: &str, chat_id: &str, tool: &str, cmd: &str) -> String {
    let card = serde_json::json!({
        "config": {"update_multi": false},
        "header": {"title": {"tag": "plain_text", "content": "Claude Code 请求确认"}, "template": "blue"},
        "elements": [
            {"tag": "div", "fields": [
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**工具**\n{}", tool)}},
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**命令**\n{}", cmd)}}
            ]},
            {"tag": "action", "actions": [
                {"tag": "button", "text": {"tag": "plain_text", "content": "✅ 允许"}, "type": "primary",
                 "value": serde_json::to_string(&serde_json::json!({"request_id":rid,"action":"allow"})).unwrap()},
                {"tag": "button", "text": {"tag": "plain_text", "content": "❌ 拒绝"}, "type": "danger",
                 "value": serde_json::to_string(&serde_json::json!({"request_id":rid,"action":"deny"})).unwrap()}
            ]}
        ]
    });
    serde_json::to_string(&serde_json::json!({
        "receive_id": chat_id, "msg_type": "interactive",
        "content": serde_json::to_string(&card).unwrap()
    }))
    .unwrap()
}
