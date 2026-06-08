//! Generic Feishu v2 event types.
//!
//! All events share the same envelope (`Event`) with a typed header.
//! The body varies by `header.event_type` — use accessors like
//! `Event::card_action()` to decode specific event variants.

use serde::Deserialize;

/// Generic v2 event envelope.  All Feishu WebSocket events parse into this.
///
/// ```ignore
/// let ev: Event = serde_json::from_value(raw)?;
/// match ev.header.event_type.as_str() {
///     "card.action.trigger" => {
///         if let Some(card) = ev.card_action() { ... }
///     }
///     "im.message.receive_v1" => { ... }
///     _ => {}
/// }
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub schema: String,
    pub header: EventHeader,
    /// Raw event body — use typed accessors to decode.
    pub event: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EventHeader {
    pub app_id: Option<String>,
    pub event_type: String,
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(default)]
    pub tenant_key: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    /// Unix timestamp in microseconds (string).
    #[serde(default)]
    pub create_time: Option<String>,
}

impl Event {
    /// Returns true if the event type matches the given string.
    pub fn is_type(&self, t: &str) -> bool {
        self.header.event_type == t
    }

    /// Try to decode the body as a `card.action.trigger` event.
    pub fn card_action(&self) -> Option<CardActionBody> {
        if !self.is_type("card.action.trigger") {
            return None;
        }
        serde_json::from_value(self.event.clone()).ok()
    }
}

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

/// Card action details.  Fields are populated depending on the component tag.
///
/// | Tag | Fields used |
/// |-----|-------------|
/// | `button` | `value` |
/// | `select_static` | `option`, `value` |
/// | `date_picker` | `value` (ISO date string) |
/// | `picker_time` | `value` (ISO time string) |
/// | `picker_datetime` | `value` (ISO datetime string) |
/// | `overflow` | `option`, `value` |
/// | `select_person` | `value`, `options` |
/// | form components | `form_value`, `name` |
/// | input components | `input_value`, `name` |
#[derive(Debug, Clone, Deserialize)]
pub struct CardAction {
    pub tag: Option<String>,
    /// Usually a double-JSON-encoded string.  Use `CardAction::parse_value::<T>()`.
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
    /// Feishu encodes action values as a JSON string whose content is another
    /// JSON value: `"{\"key\":\"value\"}"`.  This method handles both layers.
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

/// Convenience struct for common button/overflow action values.
/// Use `CardAction::parse_value::<ActionValue>()` or `Event::card_action()?.action.parse_value::<ActionValue>()`.
#[derive(Debug, Clone, Deserialize)]
pub struct ActionValue {
    pub action: String,
    pub request_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_card_event() -> serde_json::Value {
        serde_json::json!({
            "schema": "2.0",
            "header": {
                "app_id": "cli_test",
                "event_type": "card.action.trigger",
                "event_id": "evt-1",
                "tenant_key": "tk",
                "token": "t1"
            },
            "event": {
                "operator": { "open_id": "ou_1" },
                "token": "c-1",
                "action": {
                    "tag": "button",
                    "value": "\"{\\\"action\\\":\\\"allow\\\",\\\"request_id\\\":\\\"r1\\\"}\""
                },
                "host": "im_message",
                "context": { "open_chat_id": "oc_1" }
            }
        })
    }

    #[test]
    fn parse_event_and_card_action() {
        let ev: Event = serde_json::from_value(sample_card_event()).unwrap();
        assert!(ev.is_type("card.action.trigger"));

        let card = ev.card_action().unwrap();
        assert_eq!(card.operator.open_id, "ou_1");
        assert_eq!(card.action.tag.as_deref(), Some("button"));
        assert_eq!(card.context.unwrap().open_chat_id.unwrap(), "oc_1");

        let av = card.action.parse_value::<ActionValue>().unwrap();
        assert_eq!(av.action, "allow");
    }

    #[test]
    fn non_card_event_returns_none() {
        let non_card = serde_json::json!({
            "schema": "2.0",
            "header": { "event_type": "im.message.receive_v1" },
            "event": { "message_id": "m1" }
        });
        let ev: Event = serde_json::from_value(non_card).unwrap();
        assert!(!ev.is_type("card.action.trigger"));
        assert!(ev.card_action().is_none());
    }
}
