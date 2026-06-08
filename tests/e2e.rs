/// E2E: feishu WebSocket interactive approval
/// Run: cargo test --test e2e -- --ignored --nocapture
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use cc_yes::ws::{
    Event, ActionValue, CardActionHandler, CardResponse, EventHandler, HandlerRegistry, WsClient, WsConfig,
};

#[tokio::test]
#[ignore]
async fn test_feishu_interactive() {
    let sp = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".claude")
        .join("settings.local.json");
    let sj: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&sp).unwrap()).unwrap();
    let fs = &sj["yes"]["feishu"];
    let app_id = fs["app_id"].as_str().unwrap().to_string();
    let app_secret = fs["app_secret"].as_str().unwrap().to_string();
    let chat_id = fs["chat_id"].as_str().unwrap().to_string();

    let request_id = format!(
        "e2e-{}",
        std::time::UNIX_EPOCH.elapsed().unwrap().as_secs()
    );
    let found = Arc::new(AtomicBool::new(false));

    let registry = Arc::new(HandlerRegistry::new(64));

    // Register a minimal event handler (just logs)
    registry
        .register(EventHandler::new(|event| {
            tracing::info!("event received: {:?}", event);
            None
        }))
        .await;

    // Register card handler with typed Event parsing
    let found_clone = found.clone();
    let expected_id = request_id.clone();
    registry
        .register(CardActionHandler::new(move |event: Event| {
            if let Some(card) = event.card_action() {
                if let Some(av) = card.action.parse_value::<ActionValue>() {
                    if av.request_id == expected_id {
                        found_clone.store(true, Ordering::SeqCst);
                        println!("  >>> MATCH! action={} request_id={} <<<", av.action, expected_id);
                    }
                }
            }
            CardResponse::empty()
        }))
        .await;

    // Start WsClient in background
    let config = WsConfig {
        app_id: app_id.clone(),
        app_secret: app_secret.clone(),
        domain: "https://open.feishu.cn".into(),
        registry: Arc::clone(&registry),
    };
    let client = WsClient::new(config);
    let _ws_handle = tokio::spawn(async move {
        match client.start().await {
            Ok(()) => tracing::info!("WsClient exited cleanly"),
            Err(e) => tracing::error!("WsClient error: {}", e),
        }
    });

    // Small delay to let the WsClient bootstrap and connect
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Get token via HTTP
    let http = reqwest::Client::new();
    let resp = http
        .post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
        .json(&serde_json::json!({"app_id": app_id, "app_secret": app_secret}))
        .send()
        .await
        .unwrap();
    let token = resp.json::<serde_json::Value>().await.unwrap()
        ["tenant_access_token"]
        .as_str()
        .unwrap()
        .to_string();

    // Build and send card message
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
    let body = serde_json::json!({
        "receive_id": chat_id,
        "msg_type": "interactive",
        "content": serde_json::to_string(&card).unwrap()
    });

    let r: serde_json::Value = http
        .post("https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id")
        .header("Authorization", &format!("Bearer {}", token))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(r["code"], 0);
    println!("Card sent! request_id={}", request_id);
    println!(">>> 请点击飞书按钮 (60s) <<<");

    // Wait for card action (60s timeout)
    tokio::time::sleep(Duration::from_secs(60)).await;

    assert!(
        found.load(Ordering::SeqCst),
        "No card action received for {}",
        request_id
    );
}
