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
