# Feishu WebSocket Transport Design

**Date**: 2026-06-08
**Status**: Approved
**Reference**: [WEBSOCKET_PROTOCOL.md](https://github.com/larksuite/oapi-sdk-go/blob/main/docs/WEBSOCKET_PROTOCOL.md)

## Overview

Implement a Feishu (Lark) WebSocket transport layer for cc-yes, with clean separation between protocol layer (connection management, frame encoding, heartbeat, reassembly) and business layer (event/card message handling via dynamic handler registration).

## Architecture

### Layer Model

```
┌────────────────────────────────────────────────┐
│              WsClient (orchestrator)            │
│                                                 │
│  ┌─────────────────┐       mpsc       ┌───────┐│
│  │  Protocol Layer  │ ───────────────→ │Business││
│  │                  │ ←─────────────── │ Layer  ││
│  │  • bootstrap     │     oneshot      │        ││
│  │  • connect       │                  │ • Registry    ││
│  │  • frame codec   │                  │ • EventHandler││
│  │  • heartbeat     │                  │ • CardHandler ││
│  │  • reassembly    │                  │        ││
│  │  • reconnect     │                  │        ││
│  └─────────────────┘                  └───────┘│
└────────────────────────────────────────────────┘
```

- Protocol layer communicates with business layer exclusively through channels
- Each `MessageType` gets one `mpsc::Sender<IncomingMessage>`, one handler registered per type
- Handler registers → mpsc channel created → `tokio::spawn` runs handler loop on rx
- Response returned via `oneshot::Sender<Vec<u8>>` embedded in `IncomingMessage`

## Module Structure

```
src/
├── ws/
│   ├── mod.rs              # re-exports, WsClient
│   ├── proto/
│   │   ├── mod.rs
│   │   ├── frame.rs        # Frame + Header struct, prost::Message impl
│   │   ├── codec.rs        # encode/decode: Frame ↔ bytes
│   │   ├── headers.rs      # Header constants, Headers type
│   │   ├── client.rs       # WebSocket connection management
│   │   ├── bootstrap.rs    # HTTP POST /callback/ws/endpoint
│   │   ├── heartbeat.rs    # Ping/Pong loop
│   │   ├── reassembly.rs   # Multipart message reassembly
│   │   ├── reconnect.rs    # Reconnect with jitter
│   │   └── error.rs        # WsError enum
│   └── business/
│       ├── mod.rs
│       ├── registry.rs     # HandlerRegistry
│       ├── handler.rs      # MessageHandler trait
│       ├── types.rs        # IncomingMessage, MessageType
│       └── handlers/
│           ├── mod.rs
│           ├── event.rs    # EventHandler
│           └── card.rs     # CardActionHandler
```

## Protocol Layer

### Frame Encoding

Manual `Frame` struct with `prost::Message` derive (no `.proto` file, no protoc):

```rust
#[derive(Clone, prost::Message)]
pub struct Frame {
    #[prost(uint64, tag = "1")] pub seq_id: u64,
    #[prost(uint64, tag = "2")] pub log_id: u64,
    #[prost(int32,  tag = "3")] pub service: i32,
    #[prost(int32,  tag = "4")] pub method: i32,
    #[prost(message, repeated, tag = "5")] pub headers: Vec<Header>,
    #[prost(string, optional, tag = "6")] pub payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")] pub payload_type: Option<String>,
    #[prost(bytes,  optional, tag = "8")] pub payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")] pub log_id_new: Option<String>,
}
```

- `method = 0` → Control frame (Ping/Pong)
- `method = 1` → Data frame (Event/Card)
- `Header["type"]` → `"event"` / `"card"` / `"ping"` / `"pong"`

### Connection Lifecycle

```
bootstrap() → HTTP POST /callback/ws/endpoint → (wss_url, service_id, ClientConfig)
    ↓
connect() → tokio_tungstenite::connect_async(wss_url)
    ↓
spawn: heartbeat_loop()  +  receive_loop()
    ↓ disconnect/error
reconnect() → jitter → bootstrap() → connect()
```

### Receive Loop

```rust
loop {
    let msg = ws_stream.next().await?;        // BinaryMessage
    let frame = Frame::decode(&msg)?;          // protobuf decode

    match frame.method {
        0 => handle_pong(&frame),              // update ClientConfig if present
        1 => {
            let payload = reassembly(&frame)?; // multipart merge if needed
            let (tx, rx) = oneshot::channel();
            let msg = IncomingMessage { payload, headers, response_tx: tx };
            let msg_type = frame.header("type"); // "event" | "card"
            registry.dispatch(msg_type, msg).await?;
            let response_data = rx.await.unwrap_or_default();
            send_response(frame, response_data).await?;
        }
    }
}
```

### Reassembly

- Keyed by `message_id` from headers
- `HeaderSum > 1` triggers reassembly: buffer fragments by `seq` index
- TTL = 5 seconds per message_id
- On all fragments received: concatenate in `seq` order, return complete payload
- On timeout: drop buffer, log warning

### Error Classification

| Error | Severity | Action |
|-------|----------|--------|
| `DecodeError` | Skip | Log, skip frame |
| `Io` / WebSocket disconnect | Fatal | Trigger reconnect |
| `ReassemblyTimeout` | Skip | Log, drop buffer |
| `NoHandler` | Degraded | Return `{"code":200}`, no error |
| `ChannelClosed` | Fatal | Return `{"code":500}`, trigger reconnect |

### Dependencies (protocol layer)

- `tokio` + `tokio-tungstenite` — async WebSocket
- `prost` — protobuf frame encode/decode
- `reqwest` — HTTP bootstrap (async)
- `serde` + `serde_json` — JSON payload handling

## Business Layer

### MessageHandler Trait

```rust
#[async_trait]
pub trait MessageHandler: Send + 'static {
    fn message_type(&self) -> MessageType;
    async fn handle(&self, msg: IncomingMessage);
}
```

### HandlerRegistry

```rust
pub struct HandlerRegistry {
    event_tx: Mutex<Option<mpsc::Sender<IncomingMessage>>>,
    card_tx:  Mutex<Option<mpsc::Sender<IncomingMessage>>>,
    // Stored for channel rebuild on reconnect
    event_handler: Mutex<Option<Arc<dyn MessageHandler>>>,
    card_handler:  Mutex<Option<Arc<dyn MessageHandler>>>,
    buffer_size: usize,
}
```

**Registration**: `register(handler, buffer_size)` stores `Arc<dyn MessageHandler>`, creates an `mpsc::channel`, stores the `tx`, spawns a `tokio::task`.

**Reconnect**: `rebuild_channels()` drops old channels (tasks exit), re-creates mpsc channels from stored handler refs, re-spawns tasks.

**Re-registration**: Registering the same type again drops old `tx`, replaces stored handler, spawns new task.

### IncomingMessage

```rust
pub struct IncomingMessage {
    pub payload: Vec<u8>,                        // JSON bytes
    pub headers: Headers,                        // timestamp, message_id, trace_id, etc.
    pub response_tx: oneshot::Sender<Vec<u8>>,   // handler sends response JSON here
}
```

### Built-in Handlers

**EventHandler** — parses JSON payload, calls user-provided closure:

```rust
pub struct EventHandler {
    on_event: Box<dyn Fn(Value) -> Option<Vec<u8>> + Send + Sync + 'static>,
}
```

**CardActionHandler** — parses card action payload, calls user-provided closure:

```rust
pub struct CardActionHandler {
    on_action: Box<dyn Fn(CardActionPayload) -> CardResponse + Send + Sync + 'static>,
}
```

### Usage

```rust
let registry = HandlerRegistry::new();

registry.register(
    EventHandler::new(|event| {
        tracing::info!("event: {:?}", event);
        None
    }),
    64,
);

registry.register(
    CardActionHandler::new(|card| {
        CardResponse::toast("操作成功")
    }),
    64,
);

let client = WsClient::new(WsConfig {
    app_id: "...".into(),
    app_secret: "...".into(),
    registry,
});
client.start().await?;
```

## Data Flow (Complete)

```
WsClient::start()
  │
  ├─ ① bootstrap() → HTTP POST /callback/ws/endpoint
  │     Response: { URL: "wss://...", ClientConfig: {...} }
  │
  ├─ ② connect() → tokio_tungstenite::connect_async(URL)
  │     Success: 101 Switching Protocols
  │     Failure: parse Handshake-* error headers
  │
  ├─ ③ spawn heartbeat_loop
  │     Every PingInterval seconds:
  │       Frame { method=0, headers=[{type: "ping"}], service }
  │       → ws.send(binary)
  │
  ├─ ④ spawn receive_loop
  │     loop {
  │       ws.recv() → Binary → Frame::decode(msg)
  │       if method==0: handle_pong (update ClientConfig if payload present)
  │       if method==1:
  │         reassembly? → IncomingMessage + oneshot
  │         dispatch via registry → mpsc send
  │         handler task: handle(msg) → response_tx.send(data)
  │         collect response → build Frame(method=1, payload=Response JSON)
  │         ws.send(binary)
  │     }
  │
  └─ ⑤ on error/disconnect → reconnect_loop
        jitter(0..ReconnectNonce) → bootstrap → connect → backoff(ReconnectInterval)
```

## Reconnect Semantics

- **Triggers**: WebSocket read/write error, `ChannelClosed`
- **Non-triggers**: `DecodeError`, `NoHandler`, `ReassemblyTimeout`
- **State preserved**: `HandlerRegistry`, `app_id/app_secret`
- **State rebuilt**: WebSocket connection, mpsc channels, heartbeat timer
- **ClientError** (auth failure, forbidden, connection limit): stop reconnect, call `on_error` callback
- **ServerError** (system busy, internal error): continue reconnect with backoff

## Testing

### Unit Tests (no network)

| Test | Scope |
|------|-------|
| `frame_encode_decode_roundtrip` | Frame ↔ bytes via prost |
| `control_frame_method_zero` | Ping frame structure |
| `data_frame_method_one` | Event frame structure |
| `single_packet_no_reassembly` | sum=1 → direct pass-through |
| `multipart_in_order` | seq 0,1,2 in order |
| `multipart_out_of_order` | seq 2,0,1 reordered |
| `reassembly_timeout_drop` | 5s TTL → buffer cleared |
| `headers_get_string` / `headers_get_int` | Header accessors |
| `register_and_dispatch` | Handler receives message |
| `no_handler_returns_error` | Unregistered type → error |
| `reregister_replaces_old` | Old handler dropped, new one active |
| `handler_panic_channel_closed` | Panic → ChannelClosed |

### Integration Tests

| Test | Scope |
|------|-------|
| `full_pipeline_mock_server` | Bootstrap mock + WS echo server → connect → receive → respond |
| `reconnect_on_disconnect` | Kill mock server → verify reconnect |
| `heartbeat_ping_pong` | Verify ping interval, pong response |

## Host Process

cc-yes is currently a short-lived CLI hook (read stdin → write stdout → exit). WebSocket requires a long-running process. The WebSocket client will run as a separate binary or daemon subcommand (e.g., `cc-yes daemon` or `cc-yes serve`), not within the hook path. This binary reuses the shared `ws/` library crate.

## Reconnect & Channel Lifecycle

On reconnect, the old WebSocket connection and all mpsc channels are dropped (old handler tasks exit on channel close). The orchestrator calls `registry.rebuild_channels()` which re-creates mpsc channels and re-spawns handler tasks from the stored handler instances. The registry stores `Arc<dyn MessageHandler>` internally to support this.

## Non-Goals

- Card message sending via WebSocket (card updates use REST API, out of scope)
- HTTP callback mode (this implementation handles card actions via WebSocket)
- Multi-connection support (single WebSocket connection only)
- Encryption/decryption of HTTP callback bodies (not applicable to WS path)
