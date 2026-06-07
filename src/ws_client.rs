//! Feishu WebSocket client — proto2 binary frames with fragment reassembly.
//! Card actions arrive as type=event with event_type=card.action.trigger.

use std::collections::HashMap;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::Message;

// ── Proto2 codec ──

fn e(buf: &mut Vec<u8>, mut v: u64) {
    loop { if v < 0x80 { buf.push(v as u8); break; } buf.push((v as u8 & 0x7f) | 0x80); v >>= 7; }
}
fn pb_v(buf: &mut Vec<u8>, f: u64, v: u64) { e(buf, (f << 3) | 0); e(buf, v); }
fn pb_l(buf: &mut Vec<u8>, f: u64, d: &[u8]) {
    if d.is_empty() { return; } e(buf, (f << 3) | 2); e(buf, d.len() as u64); buf.extend_from_slice(d);
}
fn build_ping(svc: u64, seq: u64) -> Vec<u8> {
    let mut b = Vec::new(); pb_v(&mut b, 1, seq); pb_v(&mut b, 2, 0); pb_v(&mut b, 3, svc); pb_v(&mut b, 4, 0);
    let mut h = Vec::new(); pb_l(&mut h, 1, b"type"); pb_l(&mut h, 2, b"ping"); pb_l(&mut b, 5, &h); b
}
fn build_pong(svc: u64, seq: u64) -> Vec<u8> {
    let mut b = Vec::new(); pb_v(&mut b, 1, seq); pb_v(&mut b, 2, 0); pb_v(&mut b, 3, svc); pb_v(&mut b, 4, 0);
    let mut h = Vec::new(); pb_l(&mut h, 1, b"type"); pb_l(&mut h, 2, b"pong"); pb_l(&mut b, 5, &h); b
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

fn read_v(d: &[u8]) -> Option<(u64, usize)> {
    let mut v: u64 = 0; let mut s = 0;
    for (i, &b) in d.iter().enumerate() { v |= ((b & 0x7f) as u64) << s; if b < 0x80 { return Some((v,i+1)); } s+=7; if s>=64{return None;} } None
}

pub fn decode_frame(data: &[u8]) -> Option<Frame> {
    let mut f = Frame::default(); let mut p = 0;
    while p < data.len() {
        let (tag,n) = read_v(&data[p..])?; p+=n; let field=tag>>3; let wire=tag&0x7;
        match (field,wire){
            (1,0)=>{let(v,n)=read_v(&data[p..])?;f.seq_id=v;p+=n;}
            (2,0)=>{let(v,n)=read_v(&data[p..])?;f.log_id=v;p+=n;}
            (3,0)=>{let(v,n)=read_v(&data[p..])?;f.service=v as i32;p+=n;}
            (4,0)=>{let(v,n)=read_v(&data[p..])?;f.method=v as i32;p+=n;}
            (5,2)=>{let(hl,n)=read_v(&data[p..])?;p+=n;let end=p+hl as usize;
                let mut ip=p;p=end;let(mut k,mut v)=(String::new(),String::new());
                while ip<end{let(t,n)=read_v(&data[ip..])?;ip+=n;let(l,n)=read_v(&data[ip..])?;ip+=n;
                    let s=String::from_utf8_lossy(&data[ip..ip+l as usize]).to_string();ip+=l as usize;
                    match t>>3{1=>k=s,2=>v=s,_=>{}}}f.headers.insert(k,v);}
            (8,2)=>{let(pl,n)=read_v(&data[p..])?;p+=n;f.payload=data[p..p+pl as usize].to_vec();p+=pl as usize;}
            _=>{if wire==0{let(_,n)=read_v(&data[p..])?;p+=n;}else if wire==2{let(l,n)=read_v(&data[p..])?;p+=n+l as usize;}}
        }
    } Some(f)
}

// ── Card action ──

/// Parsed card action from a card.action.trigger event.
#[derive(Debug, Clone)]
pub struct CardAction {
    pub action: String,      // "allow" or "deny"
    pub request_id: String,
}

/// Extract card action from a frame payload.
/// `event.action.value` is a JSON string that represents a JSON object.
/// serde parses it directly: `json.loads(value_str)` → {action, request_id}.
pub fn parse_card_action(payload: &[u8]) -> Option<CardAction> {
    let ev: serde_json::Value = serde_json::from_slice(payload).ok()?;
    if ev["header"]["event_type"].as_str()? != "card.action.trigger" { return None; }
    let value_str = ev["event"]["action"]["value"].as_str()?;
    // value_str is a JSON-encoded object like {"action":"allow","request_id":"ws-..."}
    let action: serde_json::Value = serde_json::from_str(value_str).ok()?;
    Some(CardAction {
        action: action["action"].as_str()?.to_string(),
        request_id: action["request_id"].as_str()?.to_string(),
    })
}

// ── WsClient ──

pub struct WsClient {
    ws: tungstenite::WebSocket<MaybeTlsStream<TcpStream>>,
    service_id: i32,
    seq_id: u64,
    last_ping: Instant,
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
        Ok(Self { ws, service_id, seq_id: 1, last_ping: Instant::now(), fragments: HashMap::new() })
    }

    fn send_bin(&mut self, d: Vec<u8>) -> Result<(), String> {
        self.ws.write(Message::Binary(d)).map_err(|e| format!("write: {}", e))
    }

    fn send_ping(&mut self) -> Result<(), String> {
        let d = build_ping(self.service_id as u64, self.seq_id); self.seq_id += 1; self.last_ping = Instant::now(); self.send_bin(d)
    }

    fn send_pong(&mut self) -> Result<(), String> {
        let d = build_pong(self.service_id as u64, self.seq_id); self.seq_id += 1; self.send_bin(d)
    }

    /// Read next frame. Returns None on timeout. Handles WS ping/pong + fragment assembly.
    pub fn read_message(&mut self) -> Result<Option<Frame>, String> {
        loop {
            let raw = match self.ws.read() {
                Ok(Message::Binary(d)) => d,
                Ok(Message::Ping(d)) => { self.ws.write(Message::Pong(d)).ok(); continue; }
                Ok(Message::Close(_)) => return Err("closed".to_string()),
                Err(tungstenite::Error::Io(ref e))
                    if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {
                    if self.last_ping.elapsed() > Duration::from_secs(30) { self.send_ping()?; }
                    return Ok(None);
                }
                Err(e) => return Err(format!("read: {}", e)),
                _ => continue,
            };

            let frame = decode_frame(&raw).ok_or("decode failed")?;

            match frame.method {
                0 => {
                    // Control: handle ping/pong
                    if frame.msg_type() == "ping" { self.send_pong()?; }
                    if frame.msg_type() == "pong" { return Ok(Some(frame)); }
                }
                1 => {
                    // Data: handle fragmentation
                    let sum = frame.sum();
                    if sum > 1 {
                        let mid = frame.msg_id().to_string();
                        let entry = self.fragments.entry(mid.clone())
                            .or_insert_with(|| (sum, vec![Vec::new(); sum as usize]));
                        entry.1[frame.seq() as usize] = frame.payload.clone();
                        if entry.1.iter().all(|p| !p.is_empty()) {
                            let combined: Vec<u8> = entry.1.iter().flat_map(|p| p.iter().cloned()).collect();
                            self.fragments.remove(&mid);
                            let mut complete = frame.clone();
                            complete.payload = combined;
                            return Ok(Some(complete));
                        }
                        continue;
                    }
                    return Ok(Some(frame));
                }
                _ => continue,
            }
        }
    }

    /// Listen until deadline, calling on_frame for each complete data frame.
    pub fn listen(&mut self, deadline: Instant, mut on_frame: impl FnMut(Frame)) -> Result<(), String> {
        while Instant::now() < deadline {
            match self.read_message()? {
                Some(f) if f.method == 1 => on_frame(f),
                Some(_) => {} // control frames handled in read_message
                None => continue,
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
    fn test_ping_roundtrip() {
        let d = build_ping(42, 1);
        let f = decode_frame(&d).unwrap();
        assert_eq!(f.method, 0);
        assert_eq!(f.msg_type(), "ping");
    }

    #[test]
    fn test_parse_card_action_real() {
        // Build the exact JSON structure using serde_json (correct escaping)
        let inner = serde_json::json!({"action": "allow", "request_id": "ws-123"});
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {"event_type": "card.action.trigger"},
            "event": {"action": {"value": serde_json::to_string(&inner).unwrap(), "tag": "button"}}
        });
        let payload = serde_json::to_string(&event).unwrap();
        let a = parse_card_action(payload.as_bytes()).unwrap();
        assert_eq!(a.action, "allow");
        assert_eq!(a.request_id, "ws-123");
    }

    #[test]
    fn test_parse_card_deny_real() {
        let inner = serde_json::json!({"action": "deny", "request_id": "xyz"});
        let event = serde_json::json!({
            "schema": "2.0",
            "header": {"event_type": "card.action.trigger"},
            "event": {"action": {"value": serde_json::to_string(&inner).unwrap(), "tag": "button"}}
        });
        let payload = serde_json::to_string(&event).unwrap();
        let a = parse_card_action(payload.as_bytes()).unwrap();
        assert_eq!(a.action, "deny");
        assert_eq!(a.request_id, "xyz");
    }
}
