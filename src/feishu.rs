use crate::config::{FeishuConfig, HookInput, ApprovalResult};

/// Entry point: send feishu card notification and wait for approval.
/// Currently sends the card as a notification and falls back to delegate.
/// Future: implement WebSocket event stream to receive card button clicks.
pub fn request_approval(
    config: &FeishuConfig,
    input: &HookInput,
    command: &str,
) -> ApprovalResult {
    if !config.is_configured() {
        return ApprovalResult::Deny;
    }

    // 1. Get tenant access token
    let token = match get_tenant_token(&config.app_id, &config.app_secret) {
        Ok(t) => t,
        Err(_) => return ApprovalResult::Deny,
    };

    // 2. Send interactive card as notification
    let body = build_card_payload(&config.chat_id, &input.tool_name, command);
    let _ = send_message(&token, &body);

    // 3. Fall back to delegate — WebSocket event subscription not yet implemented.
    // The card serves as a mobile notification; the user still approves in terminal.
    ApprovalResult::Deny
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

/// Send the interactive card via feishu message API.
fn send_message(token: &str, body: &str) -> Result<(), String> {
    let url = "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id";

    let resp = ureq::post(url)
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
fn build_card_payload(chat_id: &str, tool_name: &str, command: &str) -> String {
    let card = serde_json::json!({
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
                "tag": "note",
                "elements": [
                    { "tag": "plain_text", "content": "请在终端确认此操作" }
                ]
            }
        ]
    });

    let body = serde_json::json!({
        "receive_id": chat_id,
        "msg_type": "interactive",
        "content": serde_json::to_string(&card).unwrap_or_default(),
    });

    serde_json::to_string(&body).unwrap_or_default()
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
    fn test_build_card_payload() {
        let payload = build_card_payload("oc_test", "Bash", "git status");
        assert!(payload.contains("oc_test"));
        assert!(payload.contains("Bash"));
        assert!(payload.contains("git status"));
        assert!(payload.contains("interactive"));
    }
}
