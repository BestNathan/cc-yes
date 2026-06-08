//! Card action types, response types, and handler.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::ws::proto::headers::MessageType;
use crate::ws::business::handler::MessageHandler;
use crate::ws::business::types::IncomingMessage;

// ── Card action body (deserialization) ──

/// Body for `card.action.trigger` events.
#[derive(Debug, Clone, Deserialize)]
pub struct CardActionBody {
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

/// Card action details.  Fields populated per component tag.
///
/// | Tag | Fields |
/// |-----|--------|
/// | `button` | `value` |
/// | `select_static` | `option`, `value` |
/// | `date_picker` | `value` (ISO date) |
/// | `picker_time` / `picker_datetime` | `value` |
/// | `overflow` | `option`, `value` |
/// | `select_person` | `value`, `options` |
/// | form | `form_value`, `name` |
/// | input | `input_value`, `name` |
#[derive(Debug, Clone, Deserialize)]
pub struct CardAction {
    pub tag: Option<String>,
    /// Double-JSON-encoded string.  Use `CardAction::parse_value::<T>()`.
    pub value: Option<String>,
    pub option: Option<String>,
    pub name: Option<String>,
    pub timezone: Option<String>,
    #[serde(default)]
    pub form_value: Option<serde_json::Map<String, serde_json::Value>>,
    pub input_value: Option<String>,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    pub checked: Option<bool>,
}

impl CardAction {
    /// Decode the double-JSON-encoded `value` field.
    ///
    /// Feishu encodes: `"{\"key\":\"value\"}"` — a JSON string whose content
    /// is another JSON value.  This handles both layers.
    pub fn parse_value<T: for<'de> Deserialize<'de>>(&self) -> Option<T> {
        let raw = self.value.as_ref()?;
        let inner: String = serde_json::from_str(raw).ok()?;
        serde_json::from_str(&inner).ok()
    }
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

/// Convenience type for common button/overflow action values.
/// Use `CardAction::parse_value::<ActionValue>()`.
#[derive(Debug, Clone, Deserialize)]
pub struct ActionValue {
    pub action: String,
    pub request_id: String,
}

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

// ── Handler ──

type CardCallback = Box<dyn Fn(CardActionBody) -> CardResponse + Send + Sync + 'static>;

/// Built-in handler for `MessageType::Card`.
///
/// Note: `card.action.trigger` events arrive with frame header `type=event`
/// (not `type=card`).  For WebSocket, use `EventHandler` and decode
/// `CardActionBody` from `event.event` yourself.  This handler exists
/// for the HTTP callback path.
pub struct CardActionHandler {
    callback: CardCallback,
}

impl CardActionHandler {
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(CardActionBody) -> CardResponse + Send + Sync + 'static,
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
        let body: CardActionBody = match serde_json::from_slice(&msg.payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("failed to parse card body: {}", e);
                let _ = msg.response_tx.send(b"{\"code\":500}".to_vec());
                return;
            }
        };

        let response = (self.callback)(body);
        let response_json =
            serde_json::to_vec(&response).unwrap_or_else(|_| b"{\"code\":0}".to_vec());
        let _ = msg.response_tx.send(response_json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_card_action_value() {
        let action = CardAction {
            tag: Some("button".into()),
            value: Some(r#""{\"action\":\"allow\",\"request_id\":\"r1\"}""#.into()),
            option: None,
            name: None,
            timezone: None,
            form_value: None,
            input_value: None,
            options: None,
            checked: None,
        };

        let av = action.parse_value::<ActionValue>().unwrap();
        assert_eq!(av.action, "allow");
        assert_eq!(av.request_id, "r1");
    }
}
