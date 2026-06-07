/// Standalone test: connect feishu WS, send card, keepalive ping, print all events.
/// Run: cargo run --example feishu-ws
use std::io::Write;
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;

// ── Proto2 codec ──

fn encode_varint(buf: &mut Vec<u8>, mut v: u64) {
    loop { if v < 0x80 { buf.push(v as u8); break; } buf.push((v as u8 & 0x7f) | 0x80); v >>= 7; }
}
fn pb_varint(buf: &mut Vec<u8>, f: u64, v: u64) {
    encode_varint(buf, (f << 3) | 0); encode_varint(buf, v);
}
fn pb_len(buf: &mut Vec<u8>, f: u64, d: &[u8]) {
    if d.is_empty() { return; } encode_varint(buf, (f << 3) | 2); encode_varint(buf, d.len() as u64); buf.extend_from_slice(d);
}

fn build_ping(svc: u64, seq: u64) -> Vec<u8> {
    let mut b = Vec::new();
    pb_varint(&mut b, 1, seq); pb_varint(&mut b, 2, 0);
    pb_varint(&mut b, 3, svc); pb_varint(&mut b, 4, 0);
    let mut h = Vec::new(); pb_len(&mut h, 1, b"type"); pb_len(&mut h, 2, b"ping");
    pb_len(&mut b, 5, &h); b
}

fn read_varint(d: &[u8]) -> Option<(u64, usize)> {
    let mut v: u64 = 0; let mut s = 0;
    for (i, &b) in d.iter().enumerate() { v |= ((b & 0x7f) as u64) << s; if b < 0x80 { return Some((v, i+1)); } s += 7; if s >= 64 { return None; } } None
}

#[derive(Debug, Default)]
struct Frame { seq_id: u64, log_id: u64, method: i32, headers: Vec<(String,String)>, payload: Vec<u8> }
impl Frame {
    fn h(&self, key: &str) -> &str { self.headers.iter().find(|(k,_)| k==key).map(|(_,v)| v.as_str()).unwrap_or("") }
    fn msg_type(&self) -> &str { self.h("type") }
    fn sum(&self) -> i32 { self.h("sum").parse().unwrap_or(1) }
    fn seq(&self) -> i32 { self.h("seq").parse().unwrap_or(0) }
    fn msg_id(&self) -> &str { self.h("message_id") }
}

fn decode_frame(data: &[u8]) -> Option<Frame> {
    let mut f = Frame::default(); let mut pos = 0;
    while pos < data.len() {
        let (tag, n) = read_varint(&data[pos..])?; pos += n;
        let field = tag >> 3; let wire = tag & 0x7;
        match (field, wire) {
            (1,0)=>{let(v,n)=read_varint(&data[pos..])?;f.seq_id=v;pos+=n;}
            (2,0)=>{let(v,n)=read_varint(&data[pos..])?;f.log_id=v;pos+=n;}
            (3,0)=>{let(v,n)=read_varint(&data[pos..])?;pos+=n;}
            (4,0)=>{let(v,n)=read_varint(&data[pos..])?;f.method=v as i32;pos+=n;}
            (5,2)=>{let(hl,n)=read_varint(&data[pos..])?;pos+=n;let end=pos+hl as usize;
                let mut ip=pos;pos=end;let(mut k,mut v)=(String::new(),String::new());
                while ip<end{let(t,n)=read_varint(&data[ip..])?;ip+=n;let(l,n)=read_varint(&data[ip..])?;ip+=n;
                    let s=String::from_utf8_lossy(&data[ip..ip+l as usize]).to_string();ip+=l as usize;
                    match t>>3{1=>k=s,2=>v=s,_=>{}}}f.headers.push((k,v));}
            (8,2)=>{let(pl,n)=read_varint(&data[pos..])?;pos+=n;f.payload=data[pos..pos+pl as usize].to_vec();pos+=pl as usize;}
            _=>{if wire==0{let(_,n)=read_varint(&data[pos..])?;pos+=n;}else if wire==2{let(l,n)=read_varint(&data[pos..])?;pos+=n+l as usize;}}
        }
    } Some(f)
}

// ── Main ──

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
    let svc: u64 = ws_url.split("service_id=").nth(1).and_then(|s| s.split('&').next()).and_then(|s| s.parse().ok()).unwrap_or(0);
    println!("[OK] WS URL, service_id={}", svc);

    // Connect WS
    let (mut ws, _) = tungstenite::connect(&ws_url).unwrap();
    match ws.get_mut() {
        MaybeTlsStream::Plain(s) => { s.set_read_timeout(Some(Duration::from_secs(1))).ok(); }
        MaybeTlsStream::NativeTls(s) => { s.get_ref().set_read_timeout(Some(Duration::from_secs(1))).ok(); }
        _ => {}
    }
    println!("[OK] WS Connected");

    // Skip ping/pong — go straight to listening

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

    // Main loop: listen + keepalive ping every 30s
    let start = Instant::now();
    let mut last_ping = Instant::now();
    let mut seq: u64 = 2;
    let mut msg_count = 0;
    let mut fragments: std::collections::HashMap<String, (i32, Vec<Vec<u8>>)> = std::collections::HashMap::new();

    println!("=== Listening (Ctrl+C to stop) ===");
    loop {
        match ws.read() {
            Ok(tungstenite::Message::Binary(data)) => {
                if let Some(f) = decode_frame(&data) {
                    let mt = f.msg_type();
                    match f.method {
                        0 => {
                            // Control
                            println!("[CTRL] type={} seq={}", mt, f.seq_id);
                            if mt == "pong" {
                                last_ping = Instant::now();
                            }
                        }
                        1 => {
                            // Data
                            let sum = f.sum();
                            if sum > 1 {
                                // Fragment reassembly
                                let mid = f.msg_id().to_string();
                                let entry = fragments.entry(mid.clone()).or_insert_with(|| (sum, vec![Vec::new(); sum as usize]));
                                entry.1[f.seq() as usize] = f.payload.clone();
                                if entry.1.iter().all(|p| !p.is_empty()) {
                                    let combined: Vec<u8> = entry.1.iter().flat_map(|p| p.iter().cloned()).collect();
                                    let mid_clone = mid.clone();
                                    fragments.remove(&mid_clone);
                                    msg_count += 1;
                                    let payload_str = String::from_utf8_lossy(&combined);
                                    println!("\n[DATA #{msg_count}] type={} msg_id={} sum={} FRA", mt, mid, sum);
                                    println!("{}", payload_str);
                                    process_payload(&combined, &rid);
                                } else {
                                    println!("[FRAG] type={} msg_id={} seq={}/{}", mt, mid, f.seq(), sum);
                                }
                            } else {
                                msg_count += 1;
                                let payload_str = String::from_utf8_lossy(&f.payload);
                                println!("\n[DATA #{msg_count}] type={} msg_id={} sum=1", mt, f.msg_id());
                                println!("{}", payload_str);
                                process_payload(&f.payload, &rid);
                            }
                        }
                        _ => { println!("[????] method={}", f.method); }
                    }
                } else {
                    println!("[BIN] {} bytes (can't decode)", data.len());
                }
            }
            Ok(tungstenite::Message::Ping(d)) => { ws.write(tungstenite::Message::Pong(d)).ok(); println!("[WS-PING]"); }
            Ok(tungstenite::Message::Close(_)) => { println!("[CLOSE]"); break; }
            Err(tungstenite::Error::Io(ref e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                if last_ping.elapsed() > Duration::from_secs(30) {
                    let p = build_ping(svc, seq);
                    seq += 1;
                    ws.write(tungstenite::Message::Binary(p)).ok();
                    println!("[PING] keepalive seq={}", seq-1);
                    last_ping = Instant::now();
                }
            }
            Err(e) => { println!("[ERR] {:?}", e); break; }
            _ => {}
        }
        if start.elapsed() > Duration::from_secs(120) {
            println!("[DONE] 120s limit"); break;
        }
    }
}

fn process_payload(payload: &[u8], expected_rid: &str) {
    if let Ok(ev) = serde_json::from_slice::<serde_json::Value>(payload) {
        if let Some(av) = ev["action"]["value"].as_str() {
            if let Ok(ad) = serde_json::from_str::<serde_json::Value>(av) {
                let rid = ad["request_id"].as_str().unwrap_or("");
                let act = ad["action"].as_str().unwrap_or("");
                if rid == expected_rid { println!(">>> MATCH! action={} <<<", act); }
                else { println!("  [card action: {} {}]", rid, act); }
            }
        }
        let et = ev["header"]["event_type"].as_str().unwrap_or("");
        if et.contains("card") { println!("  >>> CARD EVENT: {} <<<", et); }
    }
}
