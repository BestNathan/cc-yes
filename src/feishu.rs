//! Feishu interactive approval via WebSocket.
//! Flow: connect WS → send card with buttons → listen for card.action.trigger → return result.

use std::time::{Duration, Instant};
use crate::config::{FeishuConfig, HookInput, ApprovalResult};
use crate::ws_client;

pub fn request_approval(config: &FeishuConfig, input: &HookInput, command: &str) -> ApprovalResult {
    if !config.is_configured() { return ApprovalResult::Deny; }

    let request_id = format!("ccyes-{}", std::time::UNIX_EPOCH.elapsed().unwrap().as_secs());
    let timeout = Duration::from_secs(config.timeout_secs);

    // 1. Get token
    let token = match get_token(&config.app_id, &config.app_secret) {
        Ok(t) => t, Err(_) => return card_fallback(&config.chat_id, &input.tool_name, command),
    };

    // 2. Get WS URL
    let ws_url = match get_ws_url(&config.app_id, &config.app_secret) {
        Ok(u) => u, Err(_) => return card_fallback(&config.chat_id, &input.tool_name, command),
    };

    // 3. Connect WS
    let mut ws = match ws_client::WsClient::connect(&ws_url) {
        Ok(w) => w, Err(_) => return card_fallback(&config.chat_id, &input.tool_name, command),
    };

    // 4. Send interactive card
    let body = build_card(&request_id, &config.chat_id, &input.tool_name, command);
    if send_msg(&token, &body).is_err() { let _ = ws.close(); return ApprovalResult::Deny; }

    // 5. Listen for card.action.trigger
    let deadline = Instant::now() + timeout;
    let mut approval: Option<ApprovalResult> = None;

    let listen_result = ws.listen(deadline, |frame| {
        if frame.msg_type() == "event" {
            if let Some(action) = ws_client::parse_card_action(&frame.payload) {
                if action.request_id == request_id {
                    approval = Some(match action.action.as_str() {
                        "allow" => ApprovalResult::Allow,
                        _ => ApprovalResult::Deny,
                    });
                }
            }
        }
    });

    let _ = ws.close();

    match approval {
        Some(r) => r,
        None => {
            if listen_result.is_err() { ApprovalResult::Deny }
            else { ApprovalResult::Timeout }
        }
    }
}

// For now, keep the simpler notification fallback approach working.
// The full interactive flow needs refactoring (closures can't return values).

fn card_fallback(_chat_id: &str, _tool_name: &str, _command: &str) -> ApprovalResult {
    ApprovalResult::Deny
}

// ── API helpers ──

fn get_token(app_id: &str, app_secret: &str) -> Result<String, String> {
    let resp = ureq::post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(&serde_json::json!({"app_id":app_id,"app_secret":app_secret}).to_string())
        .map_err(|e| format!("token: {}", e))?;
    let j: serde_json::Value = resp.into_json().map_err(|e| format!("json: {}", e))?;
    j["tenant_access_token"].as_str().map(|s| s.to_string()).ok_or("no token".to_string())
}

fn get_ws_url(app_id: &str, app_secret: &str) -> Result<String, String> {
    let resp = ureq::post("https://open.feishu.cn/callback/ws/endpoint")
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(&serde_json::json!({"AppID":app_id,"AppSecret":app_secret}).to_string())
        .map_err(|e| format!("ws url: {}", e))?;
    let j: serde_json::Value = resp.into_json().map_err(|e| format!("json: {}", e))?;
    j["data"]["URL"].as_str().map(|s| s.to_string()).ok_or("no ws url".to_string())
}

fn send_msg(token: &str, body: &str) -> Result<(), String> {
    let resp = ureq::post("https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id")
        .set("Authorization", &format!("Bearer {}", token))
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(body).map_err(|e| format!("send: {}", e))?;
    let j: serde_json::Value = resp.into_json().map_err(|e| format!("json: {}", e))?;
    if j["code"].as_i64().unwrap_or(-1) != 0 { return Err(format!("api: {}", j)); }
    Ok(())
}

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

fn build_notification(chat_id: &str, tool: &str, cmd: &str) -> String {
    let card = serde_json::json!({
        "config": {"update_multi": false},
        "header": {"title": {"tag": "plain_text", "content": "Claude Code 请求确认"}, "template": "blue"},
        "elements": [
            {"tag": "div", "fields": [
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**工具**\n{}", tool)}},
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**命令**\n{}", cmd)}}
            ]},
            {"tag": "note", "elements": [{"tag": "plain_text", "content": "请在终端确认此操作"}]}
        ]
    });
    serde_json::to_string(&serde_json::json!({
        "receive_id": chat_id, "msg_type": "interactive",
        "content": serde_json::to_string(&card).unwrap()
    })).unwrap()
}
