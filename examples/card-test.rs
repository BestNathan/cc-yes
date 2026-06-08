/// Feishu WebSocket card interaction test using typed Event + CardActionBody.
/// Run: cargo run --example card-test
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use cc_yes::ws::{ActionValue, CardActionBody, Event, EventHandler, HandlerRegistry, WsClient, WsConfig};

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

    let found_clone = found.clone();
    let expected_id = request_id.clone();
    registry
        .register(EventHandler::new(move |event: Event| {
            // Decode card body from raw event
            if let Ok(card) = serde_json::from_value::<CardActionBody>(event.event) {
                let tag = card.action.tag.as_deref().unwrap_or("?");
                let operator = &card.operator.open_id;
                let host = &card.host;

                // Parse the double-JSON-encoded action value
                if let Some(av) = card.action.parse_value::<ActionValue>() {
                    println!(
                        "[CARD] tag={tag} action={action} request_id={rid} host={host} operator={operator}",
                        tag = tag,
                        action = av.action,
                        rid = av.request_id,
                        host = host,
                        operator = operator
                    );

                    if av.request_id == expected_id {
                        println!(">>> MATCH! <<<");
                        found_clone.store(true, Ordering::SeqCst);
                    }
                }
            } else {
                println!(
                    "[EVENT] type={} id={}",
                    event.header.event_type,
                    event.header.event_id.as_deref().unwrap_or("?")
                );
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
    println!("[OK] Token");

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
        "receive_id": chat_id, "msg_type": "interactive",
        "content": serde_json::to_string(&card).unwrap()
    });

    let r: serde_json::Value = http
        .post("https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id")
        .header("Authorization", &format!("Bearer {}", token)).json(&body)
        .send().await.unwrap().json().await.unwrap();

    if r["code"] != 0 {
        eprintln!("[FAIL] Send card failed: {:?}", r);
        return;
    }
    println!("[SEND] Card sent! request_id={}", request_id);
    println!(">>> Click a button in Feishu (60s timeout) <<<");

    // ── Wait for card click ──
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
