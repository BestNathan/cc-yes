//! Generic Feishu v2 event envelope — thin wrapper.
//!
//! `Event` only provides the shared header and raw body.  Business modules
//! decode the body themselves via `serde_json::from_value(event.event)`.

use serde::Deserialize;

/// Generic v2 event envelope.  All Feishu WebSocket events parse into this.
///
/// ```ignore
/// let ev: Event = serde_json::from_value(raw)?;
/// match ev.header.event_type.as_str() {
///     "card.action.trigger" => {
///         let card: CardActionBody = serde_json::from_value(ev.event)?;
///         // ...
///     }
///     "im.message.receive_v1" => { ... }
///     _ => {}
/// }
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub schema: String,
    pub header: EventHeader,
    /// Raw event body — decode into specific types per `header.event_type`.
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
    #[serde(default)]
    pub create_time: Option<String>,
}

impl Event {
    /// Returns true if the event type matches the given string.
    pub fn is_type(&self, t: &str) -> bool {
        self.header.event_type == t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_event_envelope() {
        let raw = serde_json::json!({
            "schema": "2.0",
            "header": {
                "app_id": "cli_test",
                "event_type": "card.action.trigger",
                "event_id": "evt-1"
            },
            "event": { "operator": { "open_id": "ou_1" } }
        });
        let ev: Event = serde_json::from_value(raw).unwrap();
        assert!(ev.is_type("card.action.trigger"));
        assert_eq!(ev.header.app_id.as_deref(), Some("cli_test"));
        assert_ne!(ev.event, serde_json::Value::Null);
    }

    #[test]
    fn non_card_event() {
        let raw = serde_json::json!({
            "schema": "2.0",
            "header": { "event_type": "im.message.receive_v1" },
            "event": { "message_id": "m1" }
        });
        let ev: Event = serde_json::from_value(raw).unwrap();
        assert!(!ev.is_type("card.action.trigger"));
    }
}
