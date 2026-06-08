/// Feishu WebSocket card interaction test using typed card event parsing.
/// Run: cargo run --example card-test
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use cc_yes::ws::{CardEvent, EventHandler, HandlerRegistry, WsClient, WsConfig};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let app_id = "cli_aaafed6c0b789be2";
    let app_secret = "8zs3WVavTDWfM1scnvb5VgiGMYNChv4I";
    let chat_id = "oc_56e159a98849d5491f939086e2b08899";

    let request_id = format!("cardtest-{}", std::time::UNIX_EPOCH.elapsed().unwrap().as_secs());
    let found = Arc::new(AtomicBool::new(false));

    // ── Set up registry ──
    let registry = Arc::new(HandlerRegistry::new(64));

    // Register event handler with typed card event parsing
    let found_clone = found.clone();
    let expected_id = request_id.clone();
    registry
        .register(EventHandler::new(move |event| {
            // Try to parse as CardEvent
            if let Ok(card) = serde_json::from_value::<CardEvent>(event.clone()) {
                if card.is_card_action() {
                    let tag = card.event.action.tag.as_deref().unwrap_or("?");
                    let name = card.event.action.name.as_deref().unwrap_or("?");
                    let operator = &card.event.operator.open_id;

                    // Decode the double-JSON-encoded action value
                    if let Some(action) = card.action_value::<serde_json::Value>() {
                        let action_name = action["action"].as_str().unwrap_or("?");
                        let rid = action["request_id"].as_str().unwrap_or("?");
                        println!(
                            "[CARD] tag={tag} name={name} action={action_name} request_id={rid} operator={operator}"
                        );

                        if rid == expected_id {
                            println!(">>> MATCH! <<<");
                            found_clone.store(true, Ordering::SeqCst);
                        }
                    }
                } else {
                    println!(
                        "[EVENT] type={} id={}",
                        card.header.event_type,
                        card.header.event_id.as_deref().unwrap_or("?")
                    );
                }
            }
            None
        }))
        .await;

    // ── Start WsClient in background ──
    let config = WsConfig {
        app_id: app_id.to_string(),
        app_secret: app_secret.to_string(),
        domain: "https://open.feishu.cn".into(),
        registry: Arc::clone(&registry),
    };
    let client = WsClient::new(config);
    let _ws_handle = tokio::spawn(async move {
        match client.start().await {
            Ok(()) => eprintln!("[WS] exited cleanly"),
            Err(e) => eprintln!("[WS] error: {}", e),
        }
    });

    // Give WS time to connect
    println!("Connecting...");
    tokio::time::sleep(Duration::from_secs(2)).await;

    // ── Get token & send card ──
    let http = reqwest::Client::new();
    let resp = http
        .post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
        .json(&serde_json::json!({"app_id": app_id, "app_secret": app_secret}))
        .send().await.unwrap();
    let token = resp.json::<serde_json::Value>().await.unwrap()
        ["tenant_access_token"].as_str().unwrap().to_string();
    println!("[OK] Token obtained");

    let card = serde_json::json!({
        "config": {"update_multi": false},
        "header": {"title": {"tag": "plain_text", "content": "WebSocket 卡片测试"}, "template": "blue"},
        "elements": [
            {"tag": "div", "fields": [
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**Request ID**\n{}", request_id)}}
            ]},
            {"tag": "action", "actions": [
                {"tag": "button", "text": {"tag": "plain_text", "content": "✅ 允许"}, "type": "primary",
                 "value": serde_json::to_string(&serde_json::json!({"request_id":&request_id,"action":"allow"})).unwrap()},
                {"tag": "button", "text": {"tag": "plain_text", "content": "❌ 拒绝"}, "type": "danger",
                 "value": serde_json::to_string(&serde_json::json!({"request_id":&request_id,"action":"deny"})).unwrap()}
            ]}
        ]
    });
    let body = serde_json::json!({
        "receive_id": chat_id,
        "msg_type": "interactive",
        "content": serde_json::to_string(&card).unwrap()
    });

    let r: serde_json::Value = http
        .post("https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id")
        .header("Authorization", &format!("Bearer {}", token))
        .json(&body)
        .send().await.unwrap().json().await.unwrap();

    if r["code"] != 0 {
        eprintln!("[FAIL] Send card failed: {:?}", r);
        return;
    }
    println!("[SEND] Card sent! request_id={}", request_id);
    println!(">>> Click a button in Feishu (60s timeout) <<<");

    // ── Wait for card click or timeout ──
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while tokio::time::Instant::now() < deadline {
        if found.load(Ordering::SeqCst) {
            println!("\n✅ Card action received! Test PASSED.");
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    eprintln!("\n⏰ Timeout — no card action received within 60s.");
    std::process::exit(1);
}
