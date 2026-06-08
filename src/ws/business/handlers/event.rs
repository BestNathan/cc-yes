//! Event handler for WebSocket `type=event` messages.

use async_trait::async_trait;
use crate::ws::proto::headers::MessageType;
use crate::ws::business::handler::MessageHandler;
use crate::ws::business::event_types::Event;
use crate::ws::business::types::IncomingMessage;

type EventCallback = Box<dyn Fn(Event) -> Option<Vec<u8>> + Send + Sync + 'static>;

/// Built-in handler for `MessageType::Event`.
///
/// Parses the JSON payload into a typed [`Event`] and passes it to the
/// user-provided callback.  Use `Event::is_type()` / `Event::card_action()`
/// to inspect specific event variants.
pub struct EventHandler {
    callback: EventCallback,
}

impl EventHandler {
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(Event) -> Option<Vec<u8>> + Send + Sync + 'static,
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
        let event: Event = match serde_json::from_slice(&msg.payload) {
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
