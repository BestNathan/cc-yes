//! Feishu WebSocket client — mirrors Go SDK `handleMessage` logic exactly.
//!
//! Proto2 wire format. Each event data frame must be acknowledged with a response frame.
//! Card actions arrive as `type=event` with `event_type=card.action.trigger`.

use std::collections::HashMap;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::Message;

// ── Proto2 codec (mirrors pbbp2.proto + Go SDK Marshal/Unmarshal) ──

fn encode_varint(buf: &mut Vec<u8>, mut v: u64) {
    loop { if v < 0x80 { buf.push(v as u8); break; } buf.push((v as u8 & 0x7f) | 0x80); v >>= 7; }
}
fn pb_tag_len(buf: &mut Vec<u8>, f: u64) { encode_varint(buf, (f << 3) | 2); }

/// Proto2: ALL fields are written (required fields always serialized).
fn build_ping(svc: i32, seq: u64) -> Vec<u8> {
    let mut b = Vec::new();
    // SeqID(1), LogID(2), Service(3), Method(4) — all required
    encode_varint(&mut b, 0x08); encode_varint(&mut b, seq);           // field 1, varint
    encode_varint(&mut b, 0x10); encode_varint(&mut b, 0);             // field 2, varint
    encode_varint(&mut b, 0x18); encode_varint(&mut b, svc as u64);    // field 3, varint
    encode_varint(&mut b, 0x20); encode_varint(&mut b, 0);             // field 4, varint (0=control)
    // Headers(5): sub-message { type: "ping" }
    let mut h = Vec::new();
    pb_tag_len(&mut h, 1); encode_varint(&mut h, 4); h.extend_from_slice(b"type");  // key="type"
    pb_tag_len(&mut h, 2); encode_varint(&mut h, 4); h.extend_from_slice(b"ping");  // value="ping"
    pb_tag_len(&mut b, 5); encode_varint(&mut b, h.len() as u64); b.extend_from_slice(&h);
    b
}

/// Build a response frame (for acknowledging event messages).
fn build_response(svc: i32, seq: u64, log_id: u64, msg_type: &str, msg_id: &str, code: i32) -> Vec<u8> {
    let payload = serde_json::json!({"code": code});
    let payload_str = serde_json::to_string(&payload).unwrap();
    let payload_bytes = payload_str.as_bytes();

    let mut b = Vec::new();
    encode_varint(&mut b, 0x08); encode_varint(&mut b, seq);             // SeqID
    encode_varint(&mut b, 0x10); encode_varint(&mut b, log_id);          // LogID (from incoming frame)
    encode_varint(&mut b, 0x18); encode_varint(&mut b, svc as u64);     // Service
    encode_varint(&mut b, 0x20); encode_varint(&mut b, 1);              // Method(1=data)

    // Headers: each is a sub-message {key, value}, repeated
    let mut h = Vec::new();
    // Header 1: key="type", value=<msg_type>
    pb_tag_len(&mut h, 1); encode_varint(&mut h, 4); h.extend_from_slice(b"type");
    pb_tag_len(&mut h, 2); encode_varint(&mut h, msg_type.len() as u64); h.extend_from_slice(msg_type.as_bytes());
    // Header 2: key="message_id", value=<msg_id>
    pb_tag_len(&mut h, 1); encode_varint(&mut h, 10); h.extend_from_slice(b"message_id");
    pb_tag_len(&mut h, 2); encode_varint(&mut h, msg_id.len() as u64); h.extend_from_slice(msg_id.as_bytes());
    pb_tag_len(&mut b, 5); encode_varint(&mut b, h.len() as u64); b.extend_from_slice(&h);

    // Payload(8)
    pb_tag_len(&mut b, 8); encode_varint(&mut b, payload_bytes.len() as u64); b.extend_from_slice(payload_bytes);
    b
}

// ── Frame ──

#[derive(Debug, Default, Clone)]
pub struct Frame {
    pub seq_id: u64, pub log_id: u64, pub service: i32, pub method: i32,
    pub headers: HashMap<String, String>, pub payload: Vec<u8>,
}
impl Frame {
    pub fn h(&self, k: &str) -> &str { self.headers.get(k).map(|s| s.as_str()).unwrap_or("") }
    pub fn msg_type(&self) -> &str { self.h("type") }
    pub fn sum(&self) -> i32 { self.h("sum").parse().unwrap_or(1) }
    pub fn seq(&self) -> i32 { self.h("seq").parse().unwrap_or(0) }
    pub fn msg_id(&self) -> &str { self.h("message_id") }
}

// ── Protobuf varint decoder ──

fn read_v(d: &[u8]) -> Option<(u64, usize)> {
    let mut v: u64 = 0; let mut s = 0;
    for (i, &b) in d.iter().enumerate() { v |= ((b & 0x7f) as u64) << s; if b < 0x80 { return Some((v,i+1)); } s+=7; if s>=64{return None;} } None
}

pub fn decode_frame(data: &[u8]) -> Option<Frame> {
    let mut f = Frame::default(); let mut p = 0;
    while p < data.len() {
        let (tag,n) = read_v(&data[p..])?; p+=n;
        let field=tag>>3; let wire=tag&0x7;
        match (field,wire){
            (1,0)=>{let(v,n)=read_v(&data[p..])?;f.seq_id=v;p+=n;}
            (2,0)=>{let(v,n)=read_v(&data[p..])?;f.log_id=v;p+=n;}
            (3,0)=>{let(v,n)=read_v(&data[p..])?;f.service=v as i32;p+=n;}
            (4,0)=>{let(v,n)=read_v(&data[p..])?;f.method=v as i32;p+=n;}
            (5,2)=>{let(hl,n)=read_v(&data[p..])?;p+=n;let end=p+hl as usize;
                let mut ip=p;p=end;
                while ip<end{
                    // Read key (field 1)
                    let(t1,n1)=read_v(&data[ip..])?;ip+=n1;
                    let(l1,n1b)=read_v(&data[ip..])?;ip+=n1b;
                    let k=String::from_utf8_lossy(&data[ip..ip+l1 as usize]).to_string();ip+=l1 as usize;
                    // Read value (field 2)
                    let(t2,n2)=read_v(&data[ip..])?;ip+=n2;
                    let(l2,n2b)=read_v(&data[ip..])?;ip+=n2b;
                    let v=String::from_utf8_lossy(&data[ip..ip+l2 as usize]).to_string();ip+=l2 as usize;
                    f.headers.insert(k,v);
                }}
            (8,2)=>{let(pl,n)=read_v(&data[p..])?;p+=n;f.payload=data[p..p+pl as usize].to_vec();p+=pl as usize;}
            _=>{if wire==0{let(_,n)=read_v(&data[p..])?;p+=n;}else if wire==2{let(l,n)=read_v(&data[p..])?;p+=n+l as usize;}}
        }
    } Some(f)
}

// ── Card action ──

#[derive(Debug, Clone)]
pub struct CardAction {
    pub action: String,
    pub request_id: String,
}

/// Parse card.action.trigger event payload.
/// `event.action.value` is double-JSON-encoded:
///   raw: `"{\\"action\\":\\"allow\\",\\"request_id\\":\\"...\\"}"`
///   First parse → JSON string `{"action":"allow","request_id":"..."}`
///   Second parse → JSON object {action: "allow", request_id: "..."}
pub fn parse_card_action(payload: &[u8]) -> Option<CardAction> {
    let ev: serde_json::Value = serde_json::from_slice(payload).ok()?;
    if ev["header"]["event_type"].as_str()? != "card.action.trigger" { return None; }
    let value_str = ev["event"]["action"]["value"].as_str()?;
    // value_str is a JSON-encoded string — parse as String first
    let inner_json: String = serde_json::from_str(value_str).ok()?;
    // inner_json is the actual action object
    let action: serde_json::Value = serde_json::from_str(&inner_json).ok()?;
    Some(CardAction {
        action: action["action"].as_str()?.to_string(),
        request_id: action["request_id"].as_str()?.to_string(),
    })
}

// ── WsClient: mirrors Go SDK Client ──

pub struct WsClient {
    ws: tungstenite::WebSocket<MaybeTlsStream<TcpStream>>,
    service_id: i32,
    seq_id: u64,
    last_ping: Instant,
    ping_interval: Duration,
    fragments: HashMap<String, (i32, Vec<Vec<u8>>)>,
}

impl WsClient {
    pub fn connect(ws_url: &str) -> Result<Self, String> {
        let service_id = ws_url.split("service_id=").nth(1)
            .and_then(|s| s.split('&').next()).and_then(|s| s.parse().ok()).unwrap_or(0);
        let (mut ws, _) = tungstenite::connect(ws_url).map_err(|e| format!("connect: {}", e))?;
        match ws.get_mut() {
            MaybeTlsStream::Plain(s) => { s.set_read_timeout(Some(Duration::from_secs(1))).ok(); }
            MaybeTlsStream::NativeTls(s) => { s.get_ref().set_read_timeout(Some(Duration::from_secs(1))).ok(); }
            _ => {}
        }
        Ok(Self {
            ws, service_id, seq_id: 1, last_ping: Instant::now(),
            ping_interval: Duration::from_secs(120), // default, updated by pong config
            fragments: HashMap::new(),
        })
    }

    fn write_bin(&mut self, d: Vec<u8>) -> Result<(), String> {
        self.ws.write(Message::Binary(d)).map_err(|e| format!("write: {}", e))
    }

    fn send_ping(&mut self) -> Result<(), String> {
        let d = build_ping(self.service_id, self.seq_id); self.seq_id += 1;
        self.last_ping = Instant::now(); self.write_bin(d)
    }

    /// Read and process one message. Returns Some(data_frame) on complete data frame, None on timeout.
    /// Control frames are handled internally. Response frames are sent for event messages.
    pub fn read_message(&mut self) -> Result<Option<Frame>, String> {
        loop {
            let raw = match self.ws.read() {
                Ok(Message::Binary(d)) => d,
                Ok(Message::Ping(d)) => { self.ws.write(Message::Pong(d)).ok(); continue; }
                Ok(Message::Close(_)) => return Err("closed".to_string()),
                Err(tungstenite::Error::Io(ref e))
                    if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    if self.last_ping.elapsed() > self.ping_interval { self.send_ping()?; }
                    return Ok(None);
                }
                Err(e) => return Err(format!("read: {}", e)),
                _ => continue,
            };

            let frame = decode_frame(&raw).ok_or("proto decode failed")?;

            match frame.method {
                0 => {
                    // ── Control frame (mirrors Go handleControlFrame) ──
                    let mt = frame.msg_type();
                    if mt == "pong" && !frame.payload.is_empty() {
                        // Parse ClientConfig from pong payload
                        if let Ok(conf) = serde_json::from_slice::<serde_json::Value>(&frame.payload) {
                            if let Some(pi) = conf["PingInterval"].as_i64() {
                                self.ping_interval = Duration::from_secs(pi as u64);
                            }
                        }
                    }
                    // Note: Go SDK does NOT handle incoming "ping" control frames
                }
                1 => {
                    // ── Data frame (mirrors Go handleDataFrame) ──
                    let sum = frame.sum();
                    if sum > 1 {
                        // Fragment reassembly (mirrors Go combine())
                        let mid = frame.msg_id().to_string();
                        let entry = self.fragments.entry(mid.clone())
                            .or_insert_with(|| (sum, vec![Vec::new(); sum as usize]));
                        entry.1[frame.seq() as usize] = frame.payload.clone();
                        if entry.1.iter().all(|p| !p.is_empty()) {
                            let combined: Vec<u8> = entry.1.iter().flat_map(|p| p.iter().cloned()).collect();
                            self.fragments.remove(&mid);
                            let mut complete = frame.clone();
                            complete.payload = combined;

                            // Send response for event messages
                            if complete.msg_type() == "event" {
                                let resp = build_response(self.service_id, self.seq_id, complete.log_id,
                                    "event", complete.msg_id(), 0);
                                self.seq_id += 1;
                                self.write_bin(resp)?;
                            }
                            return Ok(Some(complete));
                        }
                        continue;
                    }

                    // Single-part data frame
                    // Per Go SDK: send response for event; card returns without response
                    if frame.msg_type() == "event" {
                        let resp = build_response(self.service_id, self.seq_id, frame.log_id,
                            "event", frame.msg_id(), 0);
                        self.seq_id += 1;
                        self.write_bin(resp)?;
                    }
                    return Ok(Some(frame));
                }
                _ => {} // Unknown method → ignore
            }
        }
    }

    /// Listen until deadline, calling on_frame for each event data frame.
    pub fn listen(&mut self, deadline: Instant, mut on_event: impl FnMut(Frame)) -> Result<(), String> {
        while Instant::now() < deadline {
            match self.read_message()? {
                Some(f) if f.msg_type() == "event" => on_event(f),
                Some(_) => {} // card or unknown → skip per Go SDK
                None => continue, // timeout → keepalive handled in read_message
            }
        }
        Ok(())
    }

    pub fn close(mut self) -> Result<(), String> {
        self.ws.close(None).map_err(|e| format!("close: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ping_encode() {
        let d = build_ping(33554678, 1);
        let f = decode_frame(&d).unwrap();
        assert_eq!(f.method, 0);
        assert_eq!(f.msg_type(), "ping");
        assert_eq!(f.service, 33554678);
    }

    #[test]
    fn test_response_roundtrip() {
        let d = build_response(5, 2, 100, "event", "msg-1", 0);
        let f = decode_frame(&d).unwrap();
        assert_eq!(f.method, 1);
        assert_eq!(f.msg_type(), "event");
        let pl: serde_json::Value = serde_json::from_slice(&f.payload).unwrap();
        assert_eq!(pl["code"], 0);
    }

    #[test]
    fn test_parse_card_action() {
        // Real WS protocol: action.value is double-encoded JSON string.
        // serde parse of event → value = `"{\\"action\\":\\"allow\\",...}"` (starts with quote)
        // from_str::<String> → `{"action":"allow",...}` (inner JSON text)
        // from_str::<Value> → action object
        let action_obj = serde_json::json!({"action": "allow", "request_id": "ws-123"});
        let action_json = serde_json::to_string(&action_obj).unwrap(); // {"action":"allow","request_id":"ws-123"}
        // After outer serde parse, value should be a JSON-encoded string of action_json
        let value_after_serde = serde_json::to_string(&action_json).unwrap(); // "{\"action\":\"allow\",\"request_id\":\"ws-123\"}"

        // Build event: use Value::String for value, then serialize event → parse back → pass to parse_card_action
        let mut event_map = serde_json::Map::new();
        event_map.insert("schema".into(), serde_json::json!("2.0"));
        event_map.insert("header".into(), serde_json::json!({"event_type": "card.action.trigger"}));

        let mut action_map = serde_json::Map::new();
        // Store the already-JSON-encoded value string
        action_map.insert("value".into(), serde_json::Value::String(value_after_serde));
        action_map.insert("tag".into(), serde_json::json!("button"));
        event_map.insert("event".into(), serde_json::json!({"action": serde_json::Value::Object(action_map)}));

        let payload = serde_json::to_string(&event_map).unwrap();
        // When serde parses this back, value becomes: "{\"action\":\"allow\",\"request_id\":\"ws-123\"}"
        // which starts with " → from_str::<String> works
        let a = parse_card_action(payload.as_bytes()).unwrap();
        assert_eq!(a.action, "allow");
        assert_eq!(a.request_id, "ws-123");
    }
}
