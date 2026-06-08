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
