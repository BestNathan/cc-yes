/// Standalone feishu WS test using WsClient (sends response frames per protocol).
/// Run: cargo run --example feishu-ws
use std::time::{Duration, Instant};
use cc_yes::ws_client;

fn main() {
    let app_id = "cli_aaafed6c0b789be2";
    let app_secret = "8zs3WVavTDWfM1scnvb5VgiGMYNChv4I";
    let chat_id = "oc_56e159a98849d5491f939086e2b08899";

    // Token
    let resp = ureq::post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(&serde_json::json!({"app_id":app_id,"app_secret":app_secret}).to_string()).unwrap();
    let token = resp.into_json::<serde_json::Value>().unwrap()["tenant_access_token"].as_str().unwrap().to_string();
    println!("[OK] Token");

    // WS URL
    let resp = ureq::post("https://open.feishu.cn/callback/ws/endpoint")
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(&serde_json::json!({"AppID":app_id,"AppSecret":app_secret}).to_string()).unwrap();
    let ws_url = resp.into_json::<serde_json::Value>().unwrap()["data"]["URL"].as_str().unwrap().to_string();
    println!("[OK] WS URL");

    // Connect
    let mut ws = ws_client::WsClient::connect(&ws_url).unwrap();
    println!("[OK] Connected");

    // Send card
    let rid = format!("ws-{}", std::time::UNIX_EPOCH.elapsed().unwrap().as_secs());
    let card = serde_json::json!({
        "config": {"update_multi": false},
        "header": {"title": {"tag": "plain_text", "content": "交互审批测试"}, "template": "blue"},
        "elements": [
            {"tag": "div", "fields": [
                {"is_short": true, "text": {"tag": "lark_md", "content": "**工具**\nBash"}},
                {"is_short": true, "text": {"tag": "lark_md", "content": "**命令**\ntest"}}
            ]},
            {"tag": "action", "actions": [
                {"tag": "button", "text": {"tag": "plain_text", "content": "✅ 允许"}, "type": "primary",
                 "value": serde_json::to_string(&serde_json::json!({"request_id":&rid,"action":"allow"})).unwrap()},
                {"tag": "button", "text": {"tag": "plain_text", "content": "❌ 拒绝"}, "type": "danger",
                 "value": serde_json::to_string(&serde_json::json!({"request_id":&rid,"action":"deny"})).unwrap()}
            ]}
        ]
    });
    let body = serde_json::json!({"receive_id":chat_id,"msg_type":"interactive","content":serde_json::to_string(&card).unwrap()});
    let r: serde_json::Value = ureq::post("https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id")
        .set("Authorization", &format!("Bearer {}", token))
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(&body.to_string()).unwrap().into_json().unwrap();
    println!("[SEND] Card: code={} request_id={}", r["code"], rid);

    // Listen — uses WsClient which sends response frames automatically
    println!("=== Listening (Ctrl+C to stop, 120s max) ===");
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut msg_count = 0;
    ws.listen(deadline, |frame| {
        msg_count += 1;
        let payload_str = String::from_utf8_lossy(&frame.payload);
        println!("\n[DATA #{}] type={} msg_id={} sum={}",
            msg_count, frame.msg_type(), frame.msg_id(), frame.sum());
        println!("{}", payload_str);

        if let Some(action) = ws_client::parse_card_action(&frame.payload) {
            println!(">>> CARD CLICK: action={} request_id={} <<<", action.action, action.request_id);
            if action.request_id == rid {
                println!(">>> MATCH! <<<");
            }
        }
    }).ok();
    let _ = ws.close();
    println!("\nDone. {} messages", msg_count);
}
