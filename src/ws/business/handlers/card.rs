//! Card action types and handler.
//!
//! Covers the full card.action.trigger event payload as defined in
//! [WEBSOCKET_PROTOCOL.md §4](https://github.com/larksuite/oapi-sdk-go/blob/main/docs/WEBSOCKET_PROTOCOL.md#4-card-%E4%B8%9A%E5%8A%A1%E5%B1%82%E5%8D%8F%E8%AE%AE-card-business-layer).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::ws::proto::headers::MessageType;
use crate::ws::business::handler::MessageHandler;
use crate::ws::business::types::IncomingMessage;

// ── Response types ──

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
}

impl CardResponse {
    pub fn toast(content: impl Into<String>) -> Self {
        Self {
            toast: Some(Toast { toast_type: "success".into(), content: content.into() }),
        }
    }

    pub fn empty() -> Self {
        Self { toast: None }
    }
}

// ── Card event types (deserialization) ──

/// Top-level card.action.trigger event (v2 schema).
#[derive(Debug, Clone, Deserialize)]
pub struct CardEvent {
    pub schema: String,
    pub header: CardEventHeader,
    pub event: CardEventBody,
}

impl CardEvent {
    /// Parse the double-JSON-encoded `action.value` field into a typed struct.
    ///
    /// Feishu encodes action values as: `"{\"key\":\"value\"}"` — a JSON string
    /// whose content is another JSON object.  This method handles both layers.
    pub fn action_value<T: for<'de> Deserialize<'de>>(&self) -> Option<T> {
        let raw = self.event.action.value.as_ref()?;
        let inner: String = serde_json::from_str(raw).ok()?;
        serde_json::from_str(&inner).ok()
    }

    /// Returns true if this is a card.action.trigger event.
    pub fn is_card_action(&self) -> bool {
        self.header.event_type == "card.action.trigger"
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CardEventHeader {
    pub app_id: Option<String>,
    pub event_type: String,
    pub event_id: Option<String>,
    pub tenant_key: Option<String>,
    pub token: Option<String>,
    /// Unix timestamp in microseconds (string).
    pub create_time: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CardEventBody {
    pub operator: Operator,
    pub token: String,
    pub action: CardAction,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub delivery_type: Option<String>,
    #[serde(default)]
    pub context: Option<CardContext>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Operator {
    pub open_id: String,
    #[serde(default)]
    pub union_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub tenant_key: Option<String>,
}

/// Card action details.  Fields are populated depending on the component tag.
#[derive(Debug, Clone, Deserialize)]
pub struct CardAction {
    /// Component tag: "button", "select_static", "date_picker", "overflow", etc.
    pub tag: Option<String>,
    /// Button / overflow / picker custom value (double-JSON-encoded string).
    /// Use `CardEvent::action_value::<T>()` to decode.
    pub value: Option<String>,
    /// Selected option key (select_static, picker_*).
    pub option: Option<String>,
    /// Component name attribute.
    pub name: Option<String>,
    /// User's timezone.
    pub timezone: Option<String>,
    /// Form values when the card contains a form (key → value map).
    #[serde(default)]
    pub form_value: Option<serde_json::Map<String, serde_json::Value>>,
    /// Text input value.
    pub input_value: Option<String>,
    /// Multi-select options.
    #[serde(default)]
    pub options: Option<Vec<String>>,
    /// Checkbox / toggle state.
    pub checked: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CardContext {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub preview_token: Option<String>,
    #[serde(default)]
    pub open_message_id: Option<String>,
    #[serde(default)]
    pub open_chat_id: Option<String>,
}

// ── Typed action value (convenience) ──

/// Convenience struct for common button/overflow values.
/// Use `CardEvent::action_value::<ActionValue>()` to decode.
#[derive(Debug, Clone, Deserialize)]
pub struct ActionValue {
    pub action: String,
    pub request_id: String,
}

// ── Handler ──

type CardCallback = Box<dyn Fn(CardEvent) -> CardResponse + Send + Sync + 'static>;

/// Built-in handler for `MessageType::Card`.
///
/// Note: `card.action.trigger` events arrive with frame header `type=event`
/// (not `type=card`).  Use `EventHandler` with `CardEvent` parsing for
/// WebSocket card callbacks.  This handler is for the HTTP callback path.
pub struct CardActionHandler {
    callback: CardCallback,
}

impl CardActionHandler {
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(CardEvent) -> CardResponse + Send + Sync + 'static,
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
        let card_event: CardEvent = match serde_json::from_slice(&msg.payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("failed to parse card event: {}", e);
                let _ = msg.response_tx.send(b"{\"code\":500}".to_vec());
                return;
            }
        };

        let response = (self.callback)(card_event);
        let response_json =
            serde_json::to_vec(&response).unwrap_or_else(|_| b"{\"code\":0}".to_vec());
        let _ = msg.response_tx.send(response_json);
    }
}
