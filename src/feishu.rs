use std::io::Read;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tungstenite::{connect, stream::MaybeTlsStream, WebSocket};
use tungstenite::Message;
use crate::config::{FeishuConfig, HookInput, ApprovalResult};

/// Entry point: request feishu approval for a hook invocation.
/// Returns Allow, Deny, or Timeout (always safe — never panics).
pub fn request_approval(
    config: &FeishuConfig,
    input: &HookInput,
    command: &str,
) -> ApprovalResult {
    if !config.is_configured() {
        return ApprovalResult::Deny;
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
        let _ = ws.close(None);
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

/// Set a read timeout on the underlying TcpStream of a MaybeTlsStream.
fn set_stream_read_timeout(stream: &mut MaybeTlsStream<TcpStream>, timeout: Duration) {
    match stream {
        MaybeTlsStream::Plain(tcp) => {
            tcp.set_read_timeout(Some(timeout)).ok();
        }
        MaybeTlsStream::NativeTls(tls) => {
            // native_tls::TlsStream::get_mut() returns &mut TcpStream
            tls.get_mut().set_read_timeout(Some(timeout)).ok();
        }
        _ => {}
    }
}

/// Wait for a card action callback matching the given request_id.
fn wait_for_click(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    request_id: &str,
    timeout: Duration,
) -> ApprovalResult {
    set_stream_read_timeout(ws.get_mut(), Duration::from_millis(500));

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

                if let Some(result) = parse_card_event(&text, request_id) {
                    return result;
                }
            }
            Err(tungstenite::Error::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(_) => {
                return ApprovalResult::Deny;
            }
        }
    }
}

/// Parse a WebSocket text message from feishu event stream.
/// Returns Some(ApprovalResult) if this is the card click we're waiting for.
fn parse_card_event(text: &str, expected_request_id: &str) -> Option<ApprovalResult> {
    let event: serde_json::Value = serde_json::from_str(text).ok()?;

    let action = event["event"]["action"]["value"].as_str()?;
    let action_data: serde_json::Value = serde_json::from_str(action).ok()?;

    if action_data["request_id"].as_str()? != expected_request_id {
        return None;
    }

    match action_data["action"].as_str()? {
        "allow" => Some(ApprovalResult::Allow),
        "deny" => Some(ApprovalResult::Deny),
        _ => None,
    }
}

/// Generate a simple unique ID.
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
