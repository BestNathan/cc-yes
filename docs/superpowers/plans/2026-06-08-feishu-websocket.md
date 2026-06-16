# Feishu WebSocket Transport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor the monolithic synchronous `ws_client.rs` into a layered async Feishu WebSocket transport with clean protocol/business separation and dynamic handler registration.

**Architecture:** Protocol layer (`ws/proto/`) handles protobuf frame encoding via prost, WebSocket lifecycle, heartbeat, reassembly, and reconnect via tokio-tungstenite. Business layer (`ws/business/`) provides a `MessageHandler` trait + `HandlerRegistry` that spawns one tokio task per handler, connected via mpsc channels. Response flows back through oneshot channels embedded in `IncomingMessage`.

**Tech Stack:** tokio, tokio-tungstenite, prost (no .proto file), reqwest, serde_json, async-trait

---

### Task 1: Add async dependencies to Cargo.toml

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add new dependencies to Cargo.toml**

Replace the existing `[dependencies]` section with the following additions (keep all existing deps, add the new ones):

```toml
[dependencies]
# --- existing ---
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
glob = "0.3"
ureq = { version = "2", features = ["json"] }
base64 = "0.22.1"
tungstenite = { version = "0.24", features = ["native-tls"] }

# --- new for async WebSocket ---
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = { version = "0.24", features = ["native-tls"] }
prost = "0.13"
prost-derive = "0.13"
reqwest = { version = "0.12", features = ["json"] }
async-trait = "0.1"
futures-util = "0.3"
tracing = "0.1"
tracing-subscriber = "0.3"
```

- [ ] **Step 2: Run cargo check to verify dependency resolution**

Run: `cargo check`
Expected: Dependencies resolve. May have unused import warnings (fine for now).

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: add async WebSocket dependencies (tokio, prost, reqwest, async-trait)"
```

---

### Task 2: Create ws/proto/frame.rs — Frame struct with prost

**Files:**
- Create: `src/ws/proto/mod.rs`
- Create: `src/ws/proto/frame.rs`
- Create: `src/ws/mod.rs`

- [ ] **Step 1: Create mod.rs files**

```rust
// src/ws/mod.rs
pub mod proto;
pub mod business;
```

```rust
// src/ws/proto/mod.rs
pub mod frame;
pub mod codec;
pub mod headers;
pub mod client;
pub mod bootstrap;
pub mod heartbeat;
pub mod reassembly;
pub mod reconnect;
pub mod error;
```

- [ ] **Step 2: Write frame.rs with prost-derived Frame and Header structs**

```rust
// src/ws/proto/frame.rs
use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct Header {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct Frame {
    #[prost(uint64, tag = "1")]
    pub seq_id: u64,
    #[prost(uint64, tag = "2")]
    pub log_id: u64,
    #[prost(int32, tag = "3")]
    pub service: i32,
    #[prost(int32, tag = "4")]
    pub method: i32,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<Header>,
    #[prost(string, optional, tag = "6")]
    pub payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    pub payload_type: Option<String>,
    #[prost(bytes, optional, tag = "8")]
    pub payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    pub log_id_new: Option<String>,
}

impl Frame {
    /// Get a header value by key, returning empty string if not found.
    pub fn header(&self, key: &str) -> &str {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
            .unwrap_or("")
    }

    /// Get a header value as i32, defaulting to 0.
    pub fn header_int(&self, key: &str) -> i32 {
        self.header(key).parse().unwrap_or(0)
    }

    /// Message type from "type" header: "event", "card", "ping", "pong"
    pub fn msg_type(&self) -> &str {
        self.header("type")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_encode_decode_roundtrip() {
        let frame = Frame {
            seq_id: 42,
            log_id: 100,
            service: 33554678,
            method: 1,
            headers: vec![
                Header { key: "type".into(), value: "event".into() },
                Header { key: "message_id".into(), value: "msg-1".into() },
            ],
            payload_encoding: Some("json".into()),
            payload_type: None,
            payload: Some(b"{\"test\":true}".to_vec()),
            log_id_new: None,
        };

        let mut buf = Vec::new();
        frame.encode(&mut buf).unwrap();
        let decoded = Frame::decode(buf.as_slice()).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn control_frame_method_zero() {
        let frame = Frame {
            seq_id: 1,
            log_id: 0,
            service: 123,
            method: 0,
            headers: vec![Header { key: "type".into(), value: "ping".into() }],
            payload_encoding: None,
            payload_type: None,
            payload: None,
            log_id_new: None,
        };
        assert_eq!(frame.method, 0);
        assert_eq!(frame.msg_type(), "ping");
    }

    #[test]
    fn data_frame_method_one() {
        let frame = Frame {
            seq_id: 1,
            log_id: 0,
            service: 123,
            method: 1,
            headers: vec![
                Header { key: "type".into(), value: "event".into() },
                Header { key: "sum".into(), value: "1".into() },
                Header { key: "seq".into(), value: "0".into() },
            ],
            payload_encoding: Some("json".into()),
            payload_type: None,
            payload: Some(b"{}".to_vec()),
            log_id_new: None,
        };
        assert_eq!(frame.method, 1);
        assert_eq!(frame.header_int("sum"), 1);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test ws::proto::frame`
Expected: 3 tests pass (roundtrip, control, data)

- [ ] **Step 4: Add ws module to lib.rs and main.rs**

In `src/main.rs`, add `mod ws;` before the `use` statements (alongside existing `mod` declarations).

- [ ] **Step 5: Commit**

```bash
git add src/ws/ src/main.rs
git commit -m "feat(ws): add Frame struct with prost encoding"
```

---

### Task 3: Create ws/proto/headers.rs — Header constants and accessors

**Files:**
- Create: `src/ws/proto/headers.rs`

- [ ] **Step 1: Write headers.rs**

```rust
// src/ws/proto/headers.rs
use super::frame::Header;

// Header key constants
pub const HEADER_TIMESTAMP: &str = "timestamp";
pub const HEADER_TYPE: &str = "type";
pub const HEADER_MESSAGE_ID: &str = "message_id";
pub const HEADER_SUM: &str = "sum";
pub const HEADER_SEQ: &str = "seq";
pub const HEADER_TRACE_ID: &str = "trace_id";
pub const HEADER_INSTANCE_ID: &str = "instance_id";
pub const HEADER_BIZ_RT: &str = "biz_rt";
pub const HEADER_HANDSHAKE_STATUS: &str = "Handshake-Status";
pub const HEADER_HANDSHAKE_MSG: &str = "Handshake-Msg";
pub const HEADER_HANDSHAKE_AUTH_ERR_CODE: &str = "Handshake-Autherrcode";

// Message types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageType {
    Event,
    Card,
}

impl MessageType {
    pub fn from_header(value: &str) -> Option<Self> {
        match value {
            "event" => Some(Self::Event),
            "card" => Some(Self::Card),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Event => "event",
            Self::Card => "card",
        }
    }
}

// Frame types
pub const FRAME_TYPE_CONTROL: i32 = 0;
pub const FRAME_TYPE_DATA: i32 = 1;

// Error codes
pub const ERR_OK: i32 = 0;
pub const ERR_SYSTEM_BUSY: i32 = 1;
pub const ERR_FORBIDDEN: i32 = 403;
pub const ERR_AUTH_FAILED: i32 = 514;
pub const ERR_EXCEED_CONN_LIMIT: i32 = 1000040350;
pub const ERR_INTERNAL: i32 = 1000040343;

/// Wrapper around Vec<Header> for convenient access.
pub struct Headers<'a>(pub &'a [Header]);

impl<'a> Headers<'a> {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find(|h| h.key == key).map(|h| h.value.as_str())
    }

    pub fn get_int(&self, key: &str) -> i32 {
        self.get(key).and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    pub fn msg_type(&self) -> Option<MessageType> {
        self.get(HEADER_TYPE).and_then(MessageType::from_header)
    }
}

/// Builder for constructing header lists.
pub struct HeaderBuilder {
    headers: Vec<Header>,
}

impl HeaderBuilder {
    pub fn new() -> Self {
        Self { headers: Vec::new() }
    }

    pub fn add(mut self, key: &str, value: &str) -> Self {
        self.headers.push(Header { key: key.into(), value: value.into() });
        self
    }

    pub fn msg_type(self, mt: MessageType) -> Self {
        self.add(HEADER_TYPE, mt.as_str())
    }

    pub fn build(self) -> Vec<Header> {
        self.headers
    }
}
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check`
Expected: Compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/ws/proto/headers.rs
git commit -m "feat(ws): add header constants, MessageType enum, Headers wrapper"
```

---

### Task 4: Create ws/proto/codec.rs — encode/decode helpers

**Files:**
- Create: `src/ws/proto/codec.rs`

- [ ] **Step 1: Write codec.rs**

```rust
// src/ws/proto/codec.rs
use prost::Message;
use super::frame::Frame;

/// Decode protobuf bytes into a Frame.
pub fn decode_frame(data: &[u8]) -> Result<Frame, prost::DecodeError> {
    Frame::decode(data)
}

/// Encode a Frame into protobuf bytes.
pub fn encode_frame(frame: &Frame) -> Vec<u8> {
    let mut buf = Vec::with_capacity(frame.encoded_len());
    frame.encode(&mut buf).expect("Frame encode should not fail");
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::frame::{Frame, Header};

    #[test]
    fn decode_encode_roundtrip() {
        let original = Frame {
            seq_id: 1,
            log_id: 99,
            service: 5,
            method: 1,
            headers: vec![Header { key: "type".into(), value: "event".into() }],
            payload_encoding: Some("json".into()),
            payload_type: None,
            payload: Some(b"{\"hello\":\"world\"}".to_vec()),
            log_id_new: None,
        };
        let encoded = encode_frame(&original);
        let decoded = decode_frame(&encoded).unwrap();
        assert_eq!(original, decoded);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test ws::proto::codec`
Expected: 1 test passes.

- [ ] **Step 3: Commit**

```bash
git add src/ws/proto/codec.rs
git commit -m "feat(ws): add frame encode/decode helpers"
```

---

### Task 5: Create ws/proto/error.rs — error types

**Files:**
- Create: `src/ws/proto/error.rs`

- [ ] **Step 1: Write error.rs**

```rust
// src/ws/proto/error.rs
use std::fmt;

#[derive(Debug)]
pub enum WsError {
    /// HTTP bootstrap failed (network, auth, etc.)
    Bootstrap(String),
    /// WebSocket handshake rejected with status
    Handshake { status: u16, msg: String },
    /// WebSocket read/write I/O error (triggers reconnect for server errors)
    Io(std::io::Error),
    /// Protobuf frame decode failed (skip frame)
    Decode(prost::DecodeError),
    /// Message reassembly timed out (skip frame)
    ReassemblyTimeout,
    /// No handler registered for this message type (degraded, return 200)
    NoHandler(String),
    /// Handler task channel closed — task panicked or exited
    ChannelClosed,
}

impl fmt::Display for WsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bootstrap(msg) => write!(f, "bootstrap error: {}", msg),
            Self::Handshake { status, msg } => write!(f, "handshake error: {} {}", status, msg),
            Self::Io(e) => write!(f, "I/O error: {}", e),
            Self::Decode(e) => write!(f, "decode error: {}", e),
            Self::ReassemblyTimeout => write!(f, "reassembly timeout"),
            Self::NoHandler(t) => write!(f, "no handler for type: {}", t),
            Self::ChannelClosed => write!(f, "handler channel closed"),
        }
    }
}

impl std::error::Error for WsError {}

impl From<std::io::Error> for WsError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<prost::DecodeError> for WsError {
    fn from(e: prost::DecodeError) -> Self {
        Self::Decode(e)
    }
}

/// Severity determines whether to trigger reconnect or skip.
#[derive(Debug, PartialEq)]
pub enum Severity {
    /// Trigger reconnect
    Fatal,
    /// Skip current frame, continue
    Skip,
    /// Return default response, continue
    Degraded,
}

impl WsError {
    pub fn severity(&self) -> Severity {
        match self {
            Self::Io(_) | Self::ChannelClosed => Severity::Fatal,
            Self::Decode(_) | Self::ReassemblyTimeout => Severity::Skip,
            Self::Bootstrap(_) | Self::Handshake { .. } | Self::NoHandler(_) => Severity::Degraded,
        }
    }
}

/// Client-side error — stops reconnection.
pub struct ClientError {
    pub code: i32,
    pub message: String,
}

impl ClientError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "client error [{}]: {}", self.code, self.message)
    }
}

impl fmt::Debug for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for ClientError {}
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check`
Expected: Compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/ws/proto/error.rs
git commit -m "feat(ws): add error types with severity classification"
```

---

### Task 6: Create ws/business/types.rs — business layer types

**Files:**
- Create: `src/ws/business/mod.rs`
- Create: `src/ws/business/types.rs`

- [ ] **Step 1: Write business mod.rs and types.rs**

```rust
// src/ws/business/mod.rs
pub mod types;
pub mod handler;
pub mod registry;
pub mod handlers;
```

```rust
// src/ws/business/types.rs
use tokio::sync::oneshot;
use crate::ws::proto::headers::{Headers, MessageType};

/// Message sent from protocol layer to a handler task.
pub struct IncomingMessage {
    /// JSON payload bytes
    pub payload: Vec<u8>,
    /// Frame headers (timestamp, message_id, trace_id, etc.)
    pub headers: Vec<super::super::proto::frame::Header>,
    /// One-shot channel for the handler to send back response JSON
    pub response_tx: oneshot::Sender<Vec<u8>>,
}

impl IncomingMessage {
    pub fn new(
        payload: Vec<u8>,
        headers: Vec<super::super::proto::frame::Header>,
        response_tx: oneshot::Sender<Vec<u8>>,
    ) -> Self {
        Self { payload, headers, response_tx }
    }

    /// Get a header value by key.
    pub fn header(&self, key: &str) -> &str {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
            .unwrap_or("")
    }
}
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check`
Expected: Compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/ws/business/
git commit -m "feat(ws): add business layer types"
```

---

### Task 7: Create ws/business/handler.rs — MessageHandler trait

**Files:**
- Create: `src/ws/business/handler.rs`

- [ ] **Step 1: Write handler.rs**

```rust
// src/ws/business/handler.rs
use async_trait::async_trait;
use crate::ws::proto::headers::MessageType;
use super::types::IncomingMessage;

/// Trait for business logic handlers. One handler per MessageType.
/// When registered, a tokio task is spawned that loops receiving
/// IncomingMessage from an mpsc channel and calling handle().
#[async_trait]
pub trait MessageHandler: Send + Sync + 'static {
    /// Which message type this handler processes.
    fn message_type(&self) -> MessageType;

    /// Process an incoming message. The handler should send its
    /// response JSON bytes through `msg.response_tx`.
    async fn handle(&self, msg: IncomingMessage);
}
```

- [ ] **Step 2: Verify MessageType import compiles**

Run: `cargo check`
Expected: Compiles cleanly (depends on types.rs from Task 6).

- [ ] **Step 3: Commit**

```bash
git add src/ws/business/handler.rs
git commit -m "feat(ws): add MessageHandler trait"
```

---

### Task 8: Create ws/business/registry.rs — HandlerRegistry

**Files:**
- Create: `src/ws/business/registry.rs`

- [ ] **Step 1: Write registry.rs**

```rust
// src/ws/business/registry.rs
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use crate::ws::proto::headers::MessageType;
use super::handler::MessageHandler;
use super::types::IncomingMessage;

/// Registry that maps MessageType → one mpsc sender + one handler.
/// Handlers are spawned as tokio tasks on registration.
pub struct HandlerRegistry {
    event_tx: Mutex<Option<mpsc::Sender<IncomingMessage>>>,
    card_tx: Mutex<Option<mpsc::Sender<IncomingMessage>>>,
    event_handler: Mutex<Option<Arc<dyn MessageHandler>>>,
    card_handler: Mutex<Option<Arc<dyn MessageHandler>>>,
    buffer_size: usize,
}

impl HandlerRegistry {
    pub fn new(buffer_size: usize) -> Self {
        Self {
            event_tx: Mutex::new(None),
            card_tx: Mutex::new(None),
            event_handler: Mutex::new(None),
            card_handler: Mutex::new(None),
            buffer_size,
        }
    }

    /// Register a handler for its declared MessageType.
    /// If a handler is already registered for that type, the old one
    /// is replaced (old channel dropped → old task exits).
    pub async fn register<H: MessageHandler>(&self, handler: H) {
        let msg_type = handler.message_type();
        let handler: Arc<dyn MessageHandler> = Arc::new(handler);
        let (tx, mut rx) = mpsc::channel(self.buffer_size);
        let h_clone = Arc::clone(&handler);

        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                h_clone.handle(msg).await;
            }
        });

        let (tx_slot, handler_slot) = self.slots_for(msg_type);
        let mut tx_guard = tx_slot.lock().await;
        let mut handler_guard = handler_slot.lock().await;
        *tx_guard = Some(tx);
        *handler_guard = Some(handler);
    }

    /// Dispatch a message to the handler for the given MessageType.
    /// Returns an error if no handler is registered for this type.
    pub async fn dispatch(&self, msg_type: MessageType, msg: IncomingMessage) -> Result<(), super::types::IncomingMessage> {
        let (tx_slot, _) = self.slots_for(msg_type);
        let tx_guard = tx_slot.lock().await;
        match tx_guard.as_ref() {
            Some(tx) => {
                tx.send(msg).await.map_err(|e| e.0)
            }
            None => Err(msg),
        }
    }

    /// Rebuild channels after reconnect. Drops old channels and
    /// re-spawns handler tasks from stored handlers.
    pub async fn rebuild_channels(&self) {
        // Drop old channels
        for (tx_slot, _) in self.all_slots() {
            let mut guard = tx_slot.lock().await;
            *guard = None;
        }
        // Re-create from stored handlers
        let event_h = self.event_handler.lock().await;
        let card_h = self.card_handler.lock().await;

        if let Some(h) = event_h.as_ref() {
            let (tx, mut rx) = mpsc::channel(self.buffer_size);
            let h = Arc::clone(h);
            tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    h.handle(msg).await;
                }
            });
            *self.event_tx.lock().await = Some(tx);
        }
        if let Some(h) = card_h.as_ref() {
            let (tx, mut rx) = mpsc::channel(self.buffer_size);
            let h = Arc::clone(h);
            tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    h.handle(msg).await;
                }
            });
            *self.card_tx.lock().await = Some(tx);
        }
    }

    fn slots_for(&self, msg_type: MessageType)
        -> (&Mutex<Option<mpsc::Sender<IncomingMessage>>>, &Mutex<Option<Arc<dyn MessageHandler>>>)
    {
        match msg_type {
            MessageType::Event => (&self.event_tx, &self.event_handler),
            MessageType::Card => (&self.card_tx, &self.card_handler),
        }
    }

    fn all_slots(&self) -> Vec<(&Mutex<Option<mpsc::Sender<IncomingMessage>>>, &Mutex<Option<Arc<dyn MessageHandler>>>)> {
        vec![
            (&self.event_tx, &self.event_handler),
            (&self.card_tx, &self.card_handler),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use tokio::sync::oneshot;

    struct TestHandler {
        msg_type: MessageType,
    }

    #[async_trait]
    impl MessageHandler for TestHandler {
        fn message_type(&self) -> MessageType { self.msg_type }
        async fn handle(&self, msg: IncomingMessage) {
            let _ = msg.response_tx.send(b"{\"code\":0}".to_vec());
        }
    }

    #[tokio::test]
    async fn register_and_dispatch() {
        let registry = HandlerRegistry::new(8);
        registry.register(TestHandler { msg_type: MessageType::Event }).await;

        let (tx, rx) = oneshot::channel();
        let msg = IncomingMessage::new(b"{}".to_vec(), vec![], tx);
        registry.dispatch(MessageType::Event, msg).await.unwrap();

        let resp = rx.await.unwrap();
        assert_eq!(resp, b"{\"code\":0}");
    }

    #[tokio::test]
    async fn no_handler_returns_error() {
        let registry = HandlerRegistry::new(8);
        let (tx, _rx) = oneshot::channel();
        let msg = IncomingMessage::new(b"{}".to_vec(), vec![], tx);
        let result = registry.dispatch(MessageType::Event, msg).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn reregister_replaces_old() {
        let registry = HandlerRegistry::new(8);
        registry.register(TestHandler { msg_type: MessageType::Event }).await;
        // Re-register should drop old channel and spawn new task
        registry.register(TestHandler { msg_type: MessageType::Event }).await;

        let (tx, rx) = oneshot::channel();
        let msg = IncomingMessage::new(b"{}".to_vec(), vec![], tx);
        registry.dispatch(MessageType::Event, msg).await.unwrap();
        assert!(rx.await.is_ok()); // new handler responds
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test ws::business::registry`
Expected: 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/ws/business/registry.rs
git commit -m "feat(ws): add HandlerRegistry with registration and dispatch"
```

---

### Task 9: Create ws/business/handlers/ — built-in EventHandler and CardActionHandler

**Files:**
- Create: `src/ws/business/handlers/mod.rs`
- Create: `src/ws/business/handlers/event.rs`
- Create: `src/ws/business/handlers/card.rs`

- [ ] **Step 1: Write handlers/mod.rs**

```rust
// src/ws/business/handlers/mod.rs
pub mod event;
pub mod card;
```

- [ ] **Step 2: Write handlers/event.rs**

```rust
// src/ws/business/handlers/event.rs
use async_trait::async_trait;
use serde_json::Value;
use crate::ws::proto::headers::MessageType;
use crate::ws::business::handler::MessageHandler;
use crate::ws::business::types::IncomingMessage;

type EventCallback = Box<dyn Fn(Value) -> Option<Vec<u8>> + Send + Sync + 'static>;

/// Built-in handler for MessageType::Event.
/// Parses the JSON payload and passes it to a user-provided callback.
pub struct EventHandler {
    callback: EventCallback,
}

impl EventHandler {
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(Value) -> Option<Vec<u8>> + Send + Sync + 'static,
    {
        Self { callback: Box::new(callback) }
    }
}

#[async_trait]
impl MessageHandler for EventHandler {
    fn message_type(&self) -> MessageType {
        MessageType::Event
    }

    async fn handle(&self, msg: IncomingMessage) {
        let event: Value = match serde_json::from_slice(&msg.payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("failed to parse event JSON: {}", e);
                let _ = msg.response_tx.send(b"{\"code\":500}".to_vec());
                return;
            }
        };

        let response = (self.callback)(event).unwrap_or_else(|| b"{\"code\":0}".to_vec());
        let _ = msg.response_tx.send(response);
    }
}
```

- [ ] **Step 3: Write handlers/card.rs**

```rust
// src/ws/business/handlers/card.rs

use async_trait::async_trait;
use serde::Serialize;
use crate::ws::proto::headers::MessageType;
use crate::ws::business::handler::MessageHandler;
use crate::ws::business::types::IncomingMessage;

#[derive(Debug, Serialize)]
pub struct Toast {
    #[serde(rename = "type")]
    pub toast_type: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct CardResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toast: Option<Toast>,
    // Future: card update support
}

impl CardResponse {
    pub fn toast(content: impl Into<String>) -> Self {
        Self {
            toast: Some(Toast {
                toast_type: "success".into(),
                content: content.into(),
            }),
        }
    }

    pub fn empty() -> Self {
        Self { toast: None }
    }
}

type CardCallback = Box<dyn Fn(serde_json::Value) -> CardResponse + Send + Sync + 'static>;

/// Built-in handler for MessageType::Card.
/// Parses the JSON payload and passes it to a user-provided callback.
pub struct CardActionHandler {
    callback: CardCallback,
}

impl CardActionHandler {
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(serde_json::Value) -> CardResponse + Send + Sync + 'static,
    {
        Self { callback: Box::new(callback) }
    }
}

#[async_trait]
impl MessageHandler for CardActionHandler {
    fn message_type(&self) -> MessageType {
        MessageType::Card
    }

    async fn handle(&self, msg: IncomingMessage) {
        let card_event: serde_json::Value = match serde_json::from_slice(&msg.payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("failed to parse card JSON: {}", e);
                let _ = msg.response_tx.send(b"{\"code\":500}".to_vec());
                return;
            }
        };

        let response = (self.callback)(card_event);
        let response_json = serde_json::to_vec(&response).unwrap_or_else(|_| b"{\"code\":0}".to_vec());
        let _ = msg.response_tx.send(response_json);
    }
}
```

- [ ] **Step 4: Run cargo check**

Run: `cargo check`
Expected: Compiles cleanly.

- [ ] **Step 5: Commit**

```bash
git add src/ws/business/handlers/
git commit -m "feat(ws): add built-in EventHandler and CardActionHandler"
```

---

### Task 10: Create ws/proto/bootstrap.rs — HTTP bootstrap

**Files:**
- Create: `src/ws/proto/bootstrap.rs`

- [ ] **Step 1: Write bootstrap.rs**

```rust
// src/ws/proto/bootstrap.rs
use serde::Deserialize;
use super::error::{ClientError, WsError};

const GEN_ENDPOINT_URI: &str = "/callback/ws/endpoint";

#[derive(Debug, Deserialize)]
pub struct EndpointResp {
    pub code: i32,
    pub msg: Option<String>,
    pub data: Option<Endpoint>,
}

#[derive(Debug, Deserialize)]
pub struct Endpoint {
    #[serde(rename = "URL")]
    pub url: String,
    #[serde(rename = "ClientConfig")]
    pub client_config: Option<ClientConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientConfig {
    #[serde(rename = "ReconnectCount")]
    pub reconnect_count: i32,
    #[serde(rename = "ReconnectInterval")]
    pub reconnect_interval: i32,
    #[serde(rename = "ReconnectNonce")]
    pub reconnect_nonce: i32,
    #[serde(rename = "PingInterval")]
    pub ping_interval: i32,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            reconnect_count: -1,
            reconnect_interval: 120,
            reconnect_nonce: 30,
            ping_interval: 120,
        }
    }
}

pub struct BootstrapResult {
    pub ws_url: String,
    pub service_id: i32,
    pub config: ClientConfig,
}

/// POST /callback/ws/endpoint to get WebSocket URL and config.
pub async fn bootstrap(
    domain: &str,
    app_id: &str,
    app_secret: &str,
) -> Result<BootstrapResult, WsError> {
    let url = format!("{}{}", domain.trim_end_matches('/'), GEN_ENDPOINT_URI);

    let body = serde_json::json!({
        "AppID": app_id,
        "AppSecret": app_secret,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| WsError::Bootstrap(format!("HTTP request failed: {}", e)))?;

    let status = resp.status();
    if !status.is_success() {
        let msg = resp.text().await.unwrap_or_default();
        return Err(WsError::Bootstrap(format!("HTTP {}: {}", status.as_u16(), msg)));
    }

    let endpoint_resp: EndpointResp = resp
        .json()
        .await
        .map_err(|e| WsError::Bootstrap(format!("JSON parse: {}", e)))?;

    match endpoint_resp.code {
        super::headers::ERR_OK => {}
        super::headers::ERR_SYSTEM_BUSY | super::headers::ERR_INTERNAL => {
            return Err(WsError::Bootstrap(format!(
                "server error {}: {}",
                endpoint_resp.code,
                endpoint_resp.msg.unwrap_or_default()
            )));
        }
        other => {
            return Err(WsError::Bootstrap(format!(
                "client error {}: {}",
                other,
                endpoint_resp.msg.unwrap_or_default()
            )));
        }
    }

    let endpoint = endpoint_resp
        .data
        .ok_or_else(|| WsError::Bootstrap("no endpoint data".into()))?;

    if endpoint.url.is_empty() {
        return Err(WsError::Bootstrap("empty URL".into()));
    }

    let service_id = endpoint
        .url
        .split("service_id=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let config = endpoint.client_config.unwrap_or_default();

    Ok(BootstrapResult {
        ws_url: endpoint.url,
        service_id,
        config,
    })
}
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check`
Expected: Compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/ws/proto/bootstrap.rs
git commit -m "feat(ws): add HTTP bootstrap for WebSocket endpoint discovery"
```

---

### Task 11: Create ws/proto/heartbeat.rs — ping/pong loop

**Files:**
- Create: `src/ws/proto/heartbeat.rs`

- [ ] **Step 1: Write heartbeat.rs**

```rust
// src/ws/proto/heartbeat.rs
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, watch};
use tokio::time;
use tokio_tungstenite::tungstenite::Message;
use futures_util::SinkExt;
use crate::ws::proto::frame::{Frame, Header};
use crate::ws::proto::codec::encode_frame;
use crate::ws::proto::headers;

/// Build a ping control frame.
pub fn build_ping_frame(service_id: i32, seq_id: u64) -> Frame {
    Frame {
        seq_id,
        log_id: 0,
        service: service_id,
        method: headers::FRAME_TYPE_CONTROL,
        headers: vec![Header {
            key: headers::HEADER_TYPE.into(),
            value: "ping".into(),
        }],
        payload_encoding: None,
        payload_type: None,
        payload: None,
        log_id_new: None,
    }
}

/// Spawn a heartbeat task that sends ping frames at the configured interval.
/// Returns a watch sender for dynamically updating the ping interval.
pub fn start_heartbeat(
    service_id: i32,
    write_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    initial_interval: Duration,
) -> watch::Sender<Duration> {
    let (interval_tx, mut interval_rx) = watch::channel(initial_interval);
    let mut seq_id: u64 = 1;

    tokio::spawn(async move {
        loop {
            let interval = *interval_rx.borrow();
            time::sleep(interval).await;

            let frame = build_ping_frame(service_id, seq_id);
            seq_id += 1;
            let data = encode_frame(&frame);

            if write_tx.send(data).await.is_err() {
                // Write channel closed — connection is dead
                break;
            }
        }
    });

    interval_tx
}

/// Update ping interval from a pong frame's ClientConfig payload.
pub fn update_from_pong(payload: &[u8], interval_tx: &watch::Sender<Duration>) {
    if payload.is_empty() {
        return;
    }
    if let Ok(conf) = serde_json::from_slice::<serde_json::Value>(payload) {
        if let Some(pi) = conf["PingInterval"].as_i64() {
            let new_interval = Duration::from_secs(pi as u64);
            let _ = interval_tx.send(new_interval);
        }
    }
}
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check`
Expected: Compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/ws/proto/heartbeat.rs
git commit -m "feat(ws): add heartbeat ping/pong loop with dynamic interval"
```

---

### Task 12: Create ws/proto/reassembly.rs — multipart message reassembly

**Files:**
- Create: `src/ws/proto/reassembly.rs`

- [ ] **Step 1: Write reassembly.rs**

```rust
// src/ws/proto/reassembly.rs
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Pending reassembly state for a single message_id.
struct Pending {
    sum: i32,
    fragments: Vec<Option<Vec<u8>>>,
    created: Instant,
}

/// Cache for reassembling multipart messages.
pub struct ReassemblyCache {
    pending: HashMap<String, Pending>,
    ttl: Duration,
}

impl ReassemblyCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            pending: HashMap::new(),
            ttl,
        }
    }

    /// Add a fragment. Returns Some(complete_payload) when all fragments
    /// are received, or None to wait for more fragments.
    pub fn add_fragment(
        &mut self,
        message_id: &str,
        sum: i32,
        seq: i32,
        payload: Vec<u8>,
    ) -> Option<Vec<u8>> {
        let entry = self.pending.entry(message_id.to_string()).or_insert_with(|| {
            Pending {
                sum,
                fragments: vec![None; sum as usize],
                created: Instant::now(),
            }
        });

        if seq >= 0 && (seq as usize) < entry.fragments.len() {
            entry.fragments[seq as usize] = Some(payload);
        }

        // Check if all fragments received
        if entry.fragments.iter().all(|f| f.is_some()) {
            let combined: Vec<u8> = entry
                .fragments
                .iter()
                .filter_map(|f| f.as_ref())
                .flat_map(|v| v.iter().copied())
                .collect();
            self.pending.remove(message_id);
            return Some(combined);
        }

        None
    }

    /// Remove expired entries. Call periodically.
    pub fn cleanup(&mut self) {
        let now = Instant::now();
        self.pending.retain(|_, v| now.duration_since(v.created) < self.ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_packet_no_reassembly() {
        let mut cache = ReassemblyCache::new(Duration::from_secs(5));
        let result = cache.add_fragment("msg-1", 1, 0, b"hello".to_vec());
        assert_eq!(result, Some(b"hello".to_vec()));
    }

    #[test]
    fn multipart_in_order() {
        let mut cache = ReassemblyCache::new(Duration::from_secs(5));
        assert!(cache.add_fragment("msg-2", 3, 0, b"hel".to_vec()).is_none());
        assert!(cache.add_fragment("msg-2", 3, 1, b"lo ".to_vec()).is_none());
        let result = cache.add_fragment("msg-2", 3, 2, b"world".to_vec());
        assert_eq!(result, Some(b"hello world".to_vec()));
    }

    #[test]
    fn multipart_out_of_order() {
        let mut cache = ReassemblyCache::new(Duration::from_secs(5));
        assert!(cache.add_fragment("msg-3", 3, 2, b"world".to_vec()).is_none());
        assert!(cache.add_fragment("msg-3", 3, 0, b"hel".to_vec()).is_none());
        let result = cache.add_fragment("msg-3", 3, 1, b"lo ".to_vec());
        assert_eq!(result, Some(b"hello world".to_vec()));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test ws::proto::reassembly`
Expected: 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/ws/proto/reassembly.rs
git commit -m "feat(ws): add multipart message reassembly"
```

---

### Task 13: Create ws/proto/reconnect.rs — reconnect logic

**Files:**
- Create: `src/ws/proto/reconnect.rs`

- [ ] **Step 1: Write reconnect.rs**

```rust
// src/ws/proto/reconnect.rs
use std::time::Duration;
use tokio::time;
use rand::Rng;

/// Reconnect scheduler with jitter and backoff.
pub struct ReconnectScheduler {
    /// Total reconnect attempts allowed (-1 = unlimited)
    pub max_attempts: i32,
    /// Base interval between attempts (seconds)
    pub interval: Duration,
    /// Initial jitter range (seconds)
    pub nonce: i32,
}

impl ReconnectScheduler {
    pub fn new(max_attempts: i32, interval_secs: i32, nonce_secs: i32) -> Self {
        Self {
            max_attempts,
            interval: Duration::from_secs(interval_secs as u64),
            nonce: nonce_secs,
        }
    }

    /// Wait for the next reconnect attempt. Returns None if max attempts
    /// reached. Applies initial jitter on first call.
    pub async fn wait(&self, attempt: i32) -> Option<()> {
        if self.max_attempts >= 0 && attempt >= self.max_attempts {
            return None;
        }

        if attempt == 0 && self.nonce > 0 {
            let jitter_ms = rand::thread_rng().gen_range(0..self.nonce * 1000);
            time::sleep(Duration::from_millis(jitter_ms as u64)).await;
        } else {
            time::sleep(self.interval).await;
        }

        Some(())
    }
}
```

- [ ] **Step 2: Add rand dependency to Cargo.toml**

In `[dependencies]`, add:
```toml
rand = "0.8"
```

- [ ] **Step 3: Run cargo check**

Run: `cargo check`
Expected: Compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add src/ws/proto/reconnect.rs Cargo.toml Cargo.lock
git commit -m "feat(ws): add reconnect scheduler with jitter"
```

---

### Task 14: Create ws/proto/client.rs — WsClient orchestrator

**Files:**
- Create: `src/ws/proto/client.rs`

- [ ] **Step 1: Write client.rs — the main orchestrator**

```rust
// src/ws/proto/client.rs
use std::sync::Arc;
use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::connect_async;
use crate::ws::proto::bootstrap::{bootstrap, BootstrapResult};
use crate::ws::proto::codec::{decode_frame, encode_frame};
use crate::ws::proto::frame::Frame;
use crate::ws::proto::headers::{self, MessageType};
use crate::ws::proto::heartbeat::{start_heartbeat, update_from_pong};
use crate::ws::proto::reassembly::ReassemblyCache;
use crate::ws::proto::reconnect::ReconnectScheduler;
use crate::ws::proto::error::{WsError, Severity};
use crate::ws::business::registry::HandlerRegistry;
use crate::ws::business::types::IncomingMessage;

/// Configuration for the WebSocket client.
pub struct WsConfig {
    pub app_id: String,
    pub app_secret: String,
    pub domain: String,
    pub registry: Arc<HandlerRegistry>,
}

/// The WebSocket client orchestrator.
pub struct WsClient {
    config: WsConfig,
}

impl WsClient {
    pub fn new(config: WsConfig) -> Self {
        Self { config }
    }

    /// Start the WebSocket client. Blocks until fatal error (auth failure,
    /// connection limit) or the context is cancelled.
    pub async fn start(&self) -> Result<(), WsError> {
        // Bootstrap to get connection URL
        let bootstrap_result = bootstrap(
            &self.config.domain,
            &self.config.app_id,
            &self.config.app_secret,
        ).await?;

        self.run_connection_loop(bootstrap_result).await
    }

    async fn run_connection_loop(&self, bootstrap_result: BootstrapResult) -> Result<(), WsError> {
        let scheduler = ReconnectScheduler::new(
            bootstrap_result.config.reconnect_count,
            bootstrap_result.config.reconnect_interval,
            bootstrap_result.config.reconnect_nonce,
        );

        let mut attempt = 0;
        loop {
            match self.connect_and_run(&bootstrap_result).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    tracing::error!("connection error: {}", e);
                    if e.severity() == Severity::Fatal {
                        // Check if ClientError-like — auth failure, forbidden
                        if matches!(e, WsError::Bootstrap(_)) {
                            return Err(e);
                        }
                    }
                }
            }

            // Wait before retry
            match scheduler.wait(attempt).await {
                Some(()) => attempt += 1,
                None => return Err(WsError::Bootstrap("max reconnect attempts reached".into())),
            }

            // Refresh endpoint on reconnect
            tracing::info!("reconnecting (attempt {})", attempt + 1);
            self.config.registry.rebuild_channels().await;
        }
    }

    async fn connect_and_run(&self, bootstrap: &BootstrapResult) -> Result<(), WsError> {
        let (ws_stream, _) = connect_async(&bootstrap.ws_url)
            .await
            .map_err(|e| WsError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        tracing::info!("WebSocket connected");

        let (mut ws_write, mut ws_read) = ws_stream.split();

        // Channel for heartbeat task to send ping frames to the write loop
        let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(32);

        // Start heartbeat
        let ping_interval = Duration::from_secs(bootstrap.config.ping_interval as u64);
        let interval_tx = start_heartbeat(bootstrap.service_id, write_tx.clone(), ping_interval);

        // Spawn write task: multiplexes heartbeat pings and response frames
        let write_task = tokio::spawn(async move {
            while let Some(data) = write_rx.recv().await {
                if let Err(e) = ws_write.send(WsMessage::Binary(data)).await {
                    tracing::warn!("write error: {}", e);
                    break;
                }
            }
        });

        // Read loop
        let mut reassembly = ReassemblyCache::new(Duration::from_secs(5));
        loop {
            let msg = match ws_read.next().await {
                Some(Ok(WsMessage::Binary(data))) => data,
                Some(Ok(WsMessage::Ping(d))) => {
                    // Respond to WebSocket ping automatically
                    let _ = write_tx.send(d).await;
                    continue;
                }
                Some(Ok(WsMessage::Close(_))) => {
                    return Err(WsError::Io(std::io::Error::new(
                        std::io::ErrorKind::ConnectionAborted,
                        "server closed",
                    )));
                }
                Some(Err(e)) => {
                    return Err(WsError::Io(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        e,
                    )));
                }
                _ => continue,
            };

            let frame = match decode_frame(&msg) {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!("decode error: {}", e);
                    continue;
                }
            };

            match frame.method {
                0 => {
                    // Control frame — handle pong
                    if frame.msg_type() == "pong" {
                        if let Some(ref payload) = frame.payload {
                            update_from_pong(payload, &interval_tx);
                        }
                    }
                }
                1 => {
                    // Data frame
                    let msg_type = match MessageType::from_header(frame.msg_type()) {
                        Some(mt) => mt,
                        None => {
                            tracing::warn!("unknown message type: {}", frame.msg_type());
                            continue;
                        }
                    };

                    // Reassembly if multipart
                    let payload = {
                        let sum = frame.header_int(headers::HEADER_SUM);
                        let seq = frame.header_int(headers::HEADER_SEQ);
                        let msg_id = frame.header(headers::HEADER_MESSAGE_ID);

                        if sum > 1 {
                            match reassembly.add_fragment(msg_id, sum, seq, frame.payload.clone().unwrap_or_default()) {
                                Some(combined) => combined,
                                None => continue, // waiting for more fragments
                            }
                        } else {
                            frame.payload.clone().unwrap_or_default()
                        }
                    };

                    // Build IncomingMessage with oneshot for response
                    let (tx_response, rx_response) = oneshot::channel();
                    let incoming = IncomingMessage::new(
                        payload,
                        frame.headers.clone(),
                        tx_response,
                    );

                    match self.config.registry.dispatch(msg_type, incoming).await {
                        Ok(()) => {}
                        Err(_) => {
                            tracing::warn!("no handler for {:?}, returning 200", msg_type);
                        }
                    }

                    // Wait for response (timeout at 30s)
                    let response_data = tokio::time::timeout(
                        Duration::from_secs(30),
                        rx_response,
                    )
                    .await
                    .unwrap_or_else(|_| Ok(b"{\"code\":200}".to_vec()))
                    .unwrap_or_else(|_| b"{\"code\":200}".to_vec());

                    // Build and send response frame
                    let resp_frame = build_response_frame(&frame, &response_data);
                    let _ = write_tx.send(encode_frame(&resp_frame)).await;
                }
                _ => {}
            }

            // Periodic reassembly cleanup
            reassembly.cleanup();
        }
    }
}

/// Build a response Frame echoing the original frame's fields but with new payload.
fn build_response_frame(original: &Frame, response_data: &[u8]) -> Frame {
    let mut headers = original.headers.clone();
    headers.push(super::frame::Header {
        key: headers::HEADER_BIZ_RT.into(),
        value: "0".into(),
    });

    Frame {
        seq_id: original.seq_id,
        log_id: original.log_id,
        service: original.service,
        method: 1,
        headers,
        payload_encoding: Some("json".into()),
        payload_type: None,
        payload: Some(response_data.to_vec()),
        log_id_new: None,
    }
}
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check`
Expected: Compiles cleanly. May have unused variable warnings (write_task, etc. — fine).

- [ ] **Step 3: Commit**

```bash
git add src/ws/proto/client.rs
git commit -m "feat(ws): add WsClient orchestrator with read/write loop and reconnect"
```

---

### Task 15: Update ws/mod.rs with public exports

**Files:**
- Modify: `src/ws/mod.rs`

- [ ] **Step 1: Update ws/mod.rs with re-exports**

Replace the current content:

```rust
// src/ws/mod.rs
pub mod proto;
pub mod business;

pub use proto::client::{WsClient, WsConfig};
pub use proto::error::WsError;
pub use proto::headers::MessageType;
pub use business::registry::HandlerRegistry;
pub use business::handler::MessageHandler;
pub use business::types::IncomingMessage;
pub use business::handlers::event::EventHandler;
pub use business::handlers::card::{CardActionHandler, CardResponse, Toast};
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check`
Expected: Compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/ws/mod.rs
git commit -m "chore(ws): add public re-exports"
```

---

### Task 16: Refactor feishu.rs to use new async WsClient

**Files:**
- Modify: `src/feishu.rs`

- [ ] **Step 1: Refactor feishu.rs to use new async API**

Replace `src/feishu.rs` with the async version:

```rust
//! Feishu interactive approval — uses async WebSocket client.
//! Flow: connect WS → send card → listen for card.action.trigger → return result.

use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use crate::config::{FeishuConfig, ApprovalResult};
use crate::ws::{WsClient, WsConfig, HandlerRegistry, EventHandler, CardResponse, IncomingMessage};

pub async fn request_approval(
    config: &FeishuConfig,
    tool_name: &str,
    command: &str,
) -> ApprovalResult {
    if !config.is_configured() {
        return ApprovalResult::Deny;
    }

    let timeout = Duration::from_secs(config.timeout_secs);
    let request_id = format!(
        "ccyes-{}",
        std::time::UNIX_EPOCH.elapsed().unwrap().as_secs()
    );

    // 1. Get token for sending card
    let token = match get_token(&config.app_id, &config.app_secret).await {
        Ok(t) => t,
        Err(_) => return ApprovalResult::Deny,
    };

    // 2. Send card first (so it's ready when WS connects)
    let body = build_card(&request_id, &config.chat_id, tool_name, command);
    if send_msg(&token, &body).await.is_err() {
        return ApprovalResult::Deny;
    }

    // 3. Set up handler registry with approval listener
    let rid = request_id.clone();
    let (result_tx, mut result_rx) = tokio::sync::mpsc::channel(1);

    let registry = Arc::new(HandlerRegistry::new(64));
    registry.register(EventHandler::new(move |event| {
        let action = parse_card_action(&event);
        if let Some(action) = action {
            if action.request_id == rid {
                let result = match action.action.as_str() {
                    "allow" => "allow",
                    _ => "deny",
                };
                let _ = result_tx.try_send(result.to_string());
            }
        }
        None
    })).await;

    // 4. Spawn WS client in background (start() blocks on receive loop)
    let ws_client = WsClient::new(WsConfig {
        app_id: config.app_id.clone(),
        app_secret: config.app_secret.clone(),
        domain: "https://open.feishu.cn".into(),
        registry,
    });

    let ws_handle = tokio::spawn(async move {
        if let Err(e) = ws_client.start().await {
            tracing::error!("ws client error: {}", e);
        }
    });

    // 5. Race: approval result vs timeout
    let outcome = tokio::select! {
        result = result_rx.recv() => {
            match result.as_deref() {
                Some("allow") => ApprovalResult::Allow,
                _ => ApprovalResult::Deny,
            }
        }
        _ = time::sleep(timeout) => {
            ApprovalResult::Timeout
        }
    };

    ws_handle.abort();
    outcome
}

// ── Card action parsing ──

struct CardAction {
    action: String,
    request_id: String,
}

fn parse_card_action(event: &serde_json::Value) -> Option<CardAction> {
    if event["header"]["event_type"].as_str()? != "card.action.trigger" {
        return None;
    }
    let value_str = event["event"]["action"]["value"].as_str()?;
    let inner_json: String = serde_json::from_str(value_str).ok()?;
    let action: serde_json::Value = serde_json::from_str(&inner_json).ok()?;
    Some(CardAction {
        action: action["action"].as_str()?.to_string(),
        request_id: action["request_id"].as_str()?.to_string(),
    })
}

// ── API helpers ──

async fn get_token(app_id: &str, app_secret: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
        .header("Content-Type", "application/json; charset=utf-8")
        .json(&serde_json::json!({"app_id": app_id, "app_secret": app_secret}))
        .send()
        .await
        .map_err(|e| format!("token: {}", e))?;
    let j: serde_json::Value = resp.json().await.map_err(|e| format!("json: {}", e))?;
    j["tenant_access_token"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or("no token".to_string())
}

async fn send_msg(token: &str, body: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id")
        .header("Authorization", &format!("Bearer {}", token))
        .header("Content-Type", "application/json; charset=utf-8")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| format!("send: {}", e))?;
    let j: serde_json::Value = resp.json().await.map_err(|e| format!("json: {}", e))?;
    if j["code"].as_i64().unwrap_or(-1) != 0 {
        return Err(format!("api: {}", j));
    }
    Ok(())
}

fn build_card(rid: &str, chat_id: &str, tool: &str, cmd: &str) -> String {
    let card = serde_json::json!({
        "config": {"update_multi": false},
        "header": {
            "title": {"tag": "plain_text", "content": "Claude Code 请求确认"},
            "template": "blue"
        },
        "elements": [
            {"tag": "div", "fields": [
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**工具**\n{}", tool)}},
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**命令**\n{}", cmd)}}
            ]},
            {"tag": "action", "actions": [
                {"tag": "button", "text": {"tag": "plain_text", "content": "✅ 允许"},
                 "type": "primary",
                 "value": serde_json::to_string(&serde_json::json!({"request_id":rid,"action":"allow"})).unwrap()},
                {"tag": "button", "text": {"tag": "plain_text", "content": "❌ 拒绝"},
                 "type": "danger",
                 "value": serde_json::to_string(&serde_json::json!({"request_id":rid,"action":"deny"})).unwrap()}
            ]}
        ]
    });
    serde_json::to_string(&serde_json::json!({
        "receive_id": chat_id,
        "msg_type": "interactive",
        "content": serde_json::to_string(&card).unwrap()
    }))
    .unwrap()
}
```

- [ ] **Step 2: Remove old ws_client module reference from lib.rs**

Edit `src/lib.rs`:
```rust
// Remove: pub mod ws_client;
// (The ws module is now in main.rs)
```

- [ ] **Step 3: Update main.rs to include ws module**

In `src/main.rs`, add:
```rust
mod ws;
```

- [ ] **Step 4: Run cargo check**

Run: `cargo check`
Expected: Compiles cleanly. The `tungstenite` and `ureq` deps may show unused warnings — kept for now until old code paths are fully migrated.

- [ ] **Step 5: Commit**

```bash
git add src/feishu.rs src/lib.rs src/main.rs
git commit -m "refactor: migrate feishu.rs to async WsClient with HandlerRegistry"
```

---

### Task 17: Add 'daemon' subcommand to main.rs

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add Daemon variant to Commands enum**

Add to the `Commands` enum:
```rust
/// Start WebSocket daemon for long-running event/card handling
Daemon,
```

- [ ] **Step 2: Add Daemon match arm**

Add to the `match cli.command` block:
```rust
Commands::Daemon => {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| format!("tokio runtime: {}", e))?;
    rt.block_on(async {
        let registry = std::sync::Arc::new(
            crate::ws::HandlerRegistry::new(64)
        );
        // Register built-in handlers (users can extend later)
        registry.register(crate::ws::EventHandler::new(|event| {
            tracing::info!("event received: {:?}", event);
            None
        })).await;

        let config = crate::ws::WsConfig {
            app_id: std::env::var("FEISHU_APP_ID")
                .map_err(|_| "FEISHU_APP_ID not set".to_string())?,
            app_secret: std::env::var("FEISHU_APP_SECRET")
                .map_err(|_| "FEISHU_APP_SECRET not set".to_string())?,
            domain: "https://open.feishu.cn".into(),
            registry,
        };

        let client = crate::ws::WsClient::new(config);
        client.start().await.map_err(|e| format!("ws error: {}", e))
    })?;
}
```

- [ ] **Step 3: Run cargo check**

Run: `cargo check`
Expected: Compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: add 'daemon' subcommand for WebSocket event loop"
```

---

### Task 18: Integration test — full pipeline

**Files:**
- Create: `tests/ws_integration.rs`

- [ ] **Step 1: Write integration test**

```rust
// tests/ws_integration.rs

#[cfg(test)]
mod ws_tests {
    use std::sync::Arc;
    use tokio::sync::oneshot;
    use cc_yes::ws::{
        HandlerRegistry, EventHandler, IncomingMessage, MessageType,
    };

    #[tokio::test]
    async fn registry_dispatch_event() {
        let registry = HandlerRegistry::new(8);
        let (done_tx, done_rx) = oneshot::channel();

        registry.register(EventHandler::new(move |_event| {
            let _ = done_tx.send(true);
            Some(b"{\"code\":200}".to_vec())
        })).await;

        let (tx, _rx) = oneshot::channel();
        let msg = IncomingMessage::new(
            b"{\"test\":true}".to_vec(),
            vec![],
            tx,
        );

        registry.dispatch(MessageType::Event, msg).await.unwrap();
        assert!(done_rx.await.is_ok());
    }
}
```

- [ ] **Step 2: Run integration test**

Run: `cargo test --test ws_integration`
Expected: 1 test passes.

- [ ] **Step 3: Commit**

```bash
git add tests/ws_integration.rs
git commit -m "test: add integration test for registry dispatch pipeline"
```

---

### Task 19: Remove old ws_client.rs

**Files:**
- Delete: `src/ws_client.rs`

- [ ] **Step 1: Remove old ws_client module reference**

In `src/main.rs`, remove `mod ws_client;` (if still present — the `mod ws;` replaces it).

- [ ] **Step 2: Remove old lib.rs export**

Verify `src/lib.rs` no longer contains `pub mod ws_client;`.

- [ ] **Step 3: Delete the file**

Run: `rm src/ws_client.rs`

- [ ] **Step 4: Remove unused dependencies from Cargo.toml**

Remove `tungstenite` from `[dependencies]` (replaced by `tokio-tungstenite`):
```toml
# Remove: tungstenite = { version = "0.24", features = ["native-tls"] }
```

Also remove `ureq` if no longer used:
```toml
# Remove: ureq = { version = "2", features = ["json"] }
```

- [ ] **Step 5: Run cargo build and cargo test**

Run:
```bash
cargo build
```
Expected: Builds successfully with only new dependencies.

Run:
```bash
cargo test
```
Expected: All 19 existing tests + new WS tests pass.

- [ ] **Step 6: Commit**

```bash
git rm src/ws_client.rs
git add Cargo.toml Cargo.lock src/main.rs src/lib.rs
git commit -m "refactor: remove old ws_client.rs, replaced by ws/ module"
```
