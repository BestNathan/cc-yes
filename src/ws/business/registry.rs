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
