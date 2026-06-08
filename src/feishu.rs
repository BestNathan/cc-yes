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
    let (result_tx, mut result_rx) = tokio::sync::mpsc::channel(1);

    let token_for_handler = token.clone();
    let registry = Arc::new(HandlerRegistry::new(64));
    registry
        .register(EventHandler::new(move |event: Event| {
            if let Ok(card) = serde_json::from_value::<CardActionBody>(event.event) {
                if let Some(av) = card.action.parse_value::<ActionValue>() {
                    if av.request_id == rid {
                        let action = av.action.clone();
                        let _ = result_tx.try_send(action.clone());
                        let action_for_update = action.clone();

                        // Update card asynchronously
                        let token = token_for_handler.clone();
                        let msg_id = card.context.as_ref()
                            .and_then(|c| c.open_message_id.clone());
                        tokio::spawn(async move {
                            if let Some(mid) = msg_id {
                                let _ = update_card(&token, &mid, &action_for_update).await;
                            }
                        });

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

    // 3. Send interactive card
    let body = build_card(&request_id, &config.chat_id, &input.tool_name, command);
    if send_msg(&token, &body).await.is_err() {
        ws_handle.abort();
        return ApprovalResult::Deny;
    }

    // 4. Race: approval result vs timeout
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

// ── Card update ──

/// Update the interactive card to show the final state and remove buttons.
/// Uses PATCH /open-apis/im/v1/messages/:message_id
async fn update_card(token: &str, message_id: &str, action: &str) -> Result<(), String> {
    let (status_text, template) = match action {
        "allow" => ("✅ 已允许", "green"),
        _ => ("❌ 已拒绝", "red"),
    };

    let card = serde_json::json!({
        "config": {"update_multi": false},
        "header": {
            "title": {"tag": "plain_text", "content": "Claude Code 请求确认"},
            "template": template
        },
        "elements": [
            {"tag": "div", "text": {"tag": "lark_md", "content": status_text}}
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

async fn send_msg(token: &str, body: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id")
        .header("Authorization", &format!("Bearer {}", token))
        .header("Content-Type", "application/json; charset=utf-8")
        .body(body.to_string())
        .send().await.map_err(|e| format!("send: {}", e))?;
    let j: serde_json::Value = resp.json().await.map_err(|e| format!("json: {}", e))?;
    if j["code"].as_i64().unwrap_or(-1) != 0 { return Err(format!("api: {}", j)); }
    Ok(())
}

// ── Card builder ──

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
    })).unwrap()
}
