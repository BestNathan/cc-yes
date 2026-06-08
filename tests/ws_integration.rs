#[cfg(test)]
mod ws_tests {
    use std::sync::{Arc, Mutex};
    use tokio::sync::oneshot;
    use cc_yes::ws::{
        HandlerRegistry, EventHandler, IncomingMessage, MessageType,
    };

    #[tokio::test]
    async fn registry_dispatch_event() {
        let registry = HandlerRegistry::new(8);
        let (done_tx, done_rx) = oneshot::channel();
        let done_tx = Arc::new(Mutex::new(Some(done_tx)));

        let dt = done_tx.clone();
        registry.register(EventHandler::new(move |_event| {
            if let Some(tx) = dt.lock().unwrap().take() {
                let _ = tx.send(true);
            }
            Some(b"{\"code\":200}".to_vec())
        })).await;

        let (tx, _rx) = oneshot::channel();
        let msg = IncomingMessage::new(
            b"{\"test\":true}".to_vec(),
            vec![],
            tx,
        );

        registry.dispatch(MessageType::Event, msg).await.unwrap();
        assert!(done_rx.await.is_ok());
    }
}
