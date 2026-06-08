use tokio::sync::oneshot;

/// Message sent from protocol layer to a handler task.
pub struct IncomingMessage {
    /// JSON payload bytes
    pub payload: Vec<u8>,
    /// Frame headers (timestamp, message_id, trace_id, etc.)
    pub headers: Vec<crate::ws::proto::frame::Header>,
    /// One-shot channel for the handler to send back response JSON
    pub response_tx: oneshot::Sender<Vec<u8>>,
}

impl IncomingMessage {
    pub fn new(
        payload: Vec<u8>,
        headers: Vec<crate::ws::proto::frame::Header>,
        response_tx: oneshot::Sender<Vec<u8>>,
    ) -> Self {
        Self { payload, headers, response_tx }
    }

    /// Get a header value by key.
    pub fn header(&self, key: &str) -> &str {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
            .unwrap_or("")
    }
}
