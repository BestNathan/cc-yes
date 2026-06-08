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
