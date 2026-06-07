/// E2E: feishu WebSocket interactive approval
/// Run: cargo test --test e2e -- --ignored --nocapture
use std::path::PathBuf;
use std::time::{Duration, Instant};
use cc_yes::ws_client;

#[test]
#[ignore]
fn test_feishu_interactive() {
    let sp = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".claude").join("settings.local.json");
    let sj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&sp).unwrap()).unwrap();
    let fs = &sj["yes"]["feishu"];
    let app_id = fs["app_id"].as_str().unwrap(); let app_secret = fs["app_secret"].as_str().unwrap();
    let chat_id = fs["chat_id"].as_str().unwrap();

    // Token
    let resp = ureq::post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(&serde_json::json!({"app_id":app_id,"app_secret":app_secret}).to_string()).unwrap();
    let token = resp.into_json::<serde_json::Value>().unwrap()["tenant_access_token"].as_str().unwrap().to_string();

    // WS URL
    let resp = ureq::post("https://open.feishu.cn/callback/ws/endpoint")
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(&serde_json::json!({"AppID":app_id,"AppSecret":app_secret}).to_string()).unwrap();
    let ws_url = resp.into_json::<serde_json::Value>().unwrap()["data"]["URL"].as_str().unwrap().to_string();

    // Connect
    println!("Connecting to: {}...", &ws_url[..ws_url.len().min(80)]);
    let mut client = match ws_client::WsClient::connect(&ws_url) {
        Ok(c) => { println!("Handshake OK!"); c }
        Err(e) => { panic!("Connect failed: {}", e); }
    };

    // Send card
    let request_id = format!("e2e-{}", std::time::UNIX_EPOCH.elapsed().unwrap().as_secs());
    let card = serde_json::json!({
        "config": {"update_multi": false},
        "header": {"title": {"tag": "plain_text", "content": "E2E 测试"}, "template": "blue"},
        "elements": [
            {"tag": "div", "fields": [
                {"is_short": true, "text": {"tag": "lark_md", "content": "**工具**\nBash"}},
                {"is_short": true, "text": {"tag": "lark_md", "content": "**命令**\ntest"}}
            ]},
            {"tag": "action", "actions": [
                {"tag": "button", "text": {"tag": "plain_text", "content": "✅ 允许"}, "type": "primary",
                 "value": serde_json::to_string(&serde_json::json!({"request_id":&request_id,"action":"allow"})).unwrap()},
                {"tag": "button", "text": {"tag": "plain_text", "content": "❌ 拒绝"}, "type": "danger",
                 "value": serde_json::to_string(&serde_json::json!({"request_id":&request_id,"action":"deny"})).unwrap()}
            ]}
        ]
    });
    let body = serde_json::json!({"receive_id": chat_id, "msg_type": "interactive", "content": serde_json::to_string(&card).unwrap()});
    let r: serde_json::Value = ureq::post("https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id")
        .set("Authorization", &format!("Bearer {}", token))
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(&body.to_string()).unwrap().into_json().unwrap();
    assert_eq!(r["code"], 0);
    println!("Card sent! request_id={}", request_id);
    println!(">>> 请点击飞书按钮 (60s) <<<");

    // Listen
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut found = false;
    let mut msg_count = 0;
    client.listen(deadline, |frame| {
        msg_count += 1;
        let mt = frame.msg_type();
        let payload_str = String::from_utf8_lossy(&frame.payload);
        println!("\n[{}] type={} payload={}", msg_count, mt, &payload_str[..payload_str.len().min(500)]);

        if mt == "card" || mt == "event" {
            if let Ok(ev) = serde_json::from_slice::<serde_json::Value>(&frame.payload) {
                // Check for card action
                if let Some(action_val) = ev["action"]["value"].as_str() {
                    if let Ok(action) = serde_json::from_str::<serde_json::Value>(action_val) {
                        let rid = action["request_id"].as_str().unwrap_or("");
                        let act = action["action"].as_str().unwrap_or("");
                        println!("  >>> CARD ACTION: request_id={} action={} <<<", rid, act);
                        if rid == request_id {
                            println!("  >>> MATCH! {} <<<", act);
                            found = true;
                        }
                    }
                }
                // Also check event type
                let et = ev["header"]["event_type"].as_str().unwrap_or("");
                if et.contains("card") {
                    println!("  >>> CARD EVENT: {} <<<", et);
                }
            }
        }
    }).ok();
    let _ = client.close();
    println!("\nTotal: {} msgs, found={}", msg_count, found);
    assert!(found, "No card action received for {}", request_id);
}
