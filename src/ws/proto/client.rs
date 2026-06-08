use std::sync::Arc;
use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::connect_async;
use crate::ws::proto::bootstrap::{bootstrap, BootstrapResult};
use crate::ws::proto::codec::{decode_frame, encode_frame};
use crate::ws::proto::frame::Frame;
use crate::ws::proto::headers::{self, MessageType};
use crate::ws::proto::heartbeat::{start_heartbeat, update_from_pong};
use crate::ws::proto::reassembly::ReassemblyCache;
use crate::ws::proto::reconnect::ReconnectScheduler;
use crate::ws::proto::error::{WsError, Severity};
use crate::ws::business::registry::HandlerRegistry;
use crate::ws::business::types::IncomingMessage;

/// Configuration for the WebSocket client.
pub struct WsConfig {
    pub app_id: String,
    pub app_secret: String,
    pub domain: String,
    pub registry: Arc<HandlerRegistry>,
}

/// The WebSocket client orchestrator.
pub struct WsClient {
    config: WsConfig,
}

impl WsClient {
    pub fn new(config: WsConfig) -> Self {
        Self { config }
    }

    /// Start the WebSocket client. Blocks until fatal error (auth failure,
    /// connection limit) or the context is cancelled.
    pub async fn start(&self) -> Result<(), WsError> {
        // Bootstrap to get connection URL
        let bootstrap_result = bootstrap(
            &self.config.domain,
            &self.config.app_id,
            &self.config.app_secret,
        ).await?;

        self.run_connection_loop(bootstrap_result).await
    }

    async fn run_connection_loop(&self, bootstrap_result: BootstrapResult) -> Result<(), WsError> {
        let scheduler = ReconnectScheduler::new(
            bootstrap_result.config.reconnect_count,
            bootstrap_result.config.reconnect_interval,
            bootstrap_result.config.reconnect_nonce,
        );

        let mut attempt = 0;
        loop {
            match self.connect_and_run(&bootstrap_result).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    tracing::error!("connection error: {}", e);
                    if e.severity() == Severity::Fatal {
                        // Bootstrap errors (auth failure) are fatal — stop reconnecting
                        if matches!(e, WsError::Bootstrap(_)) {
                            return Err(e);
                        }
                    }
                }
            }

            // Wait before retry
            match scheduler.wait(attempt).await {
                Some(()) => attempt += 1,
                None => return Err(WsError::Bootstrap("max reconnect attempts reached".into())),
            }

            // Rebuild handler channels on reconnect
            tracing::info!("reconnecting (attempt {})", attempt + 1);
            self.config.registry.rebuild_channels().await;
        }
    }

    async fn connect_and_run(&self, bootstrap: &BootstrapResult) -> Result<(), WsError> {
        let (ws_stream, _) = connect_async(&bootstrap.ws_url)
            .await
            .map_err(|e| WsError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        tracing::info!("WebSocket connected");

        let (mut ws_write, mut ws_read) = ws_stream.split();

        // Channel for heartbeat task to send ping frames to the write loop
        let (write_tx, mut write_rx) = mpsc::channel::<Vec<u8>>(32);

        // Start heartbeat
        let ping_interval = Duration::from_secs(bootstrap.config.ping_interval as u64);
        let interval_tx = start_heartbeat(bootstrap.service_id, write_tx.clone(), ping_interval);

        // Spawn write task: multiplexes heartbeat pings and response frames
        tokio::spawn(async move {
            while let Some(data) = write_rx.recv().await {
                if let Err(e) = ws_write.send(WsMessage::Binary(data)).await {
                    tracing::warn!("write error: {}", e);
                    break;
                }
            }
        });

        // Read loop
        let mut reassembly = ReassemblyCache::new(Duration::from_secs(5));
        loop {
            let msg = match ws_read.next().await {
                Some(Ok(WsMessage::Binary(data))) => data,
                Some(Ok(WsMessage::Ping(d))) => {
                    // Respond to WebSocket ping with pong
                    let _ = write_tx.send(d).await;
                    continue;
                }
                Some(Ok(WsMessage::Close(_))) => {
                    return Err(WsError::Io(std::io::Error::new(
                        std::io::ErrorKind::ConnectionAborted,
                        "server closed",
                    )));
                }
                Some(Err(e)) => {
                    return Err(WsError::Io(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        e,
                    )));
                }
                _ => continue,
            };

            let frame = match decode_frame(&msg) {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!("decode error: {}", e);
                    continue;
                }
            };

            match frame.method {
                0 => {
                    // Control frame — handle pong
                    if frame.msg_type() == "pong" {
                        if let Some(ref payload) = frame.payload {
                            update_from_pong(payload, &interval_tx);
                        }
                    }
                }
                1 => {
                    // Data frame
                    let msg_type = match MessageType::from_header(frame.msg_type()) {
                        Some(mt) => mt,
                        None => {
                            tracing::warn!("unknown message type: {}", frame.msg_type());
                            continue;
                        }
                    };

                    // Reassembly if multipart
                    let payload = {
                        let sum = frame.header_int(headers::HEADER_SUM);
                        let seq = frame.header_int(headers::HEADER_SEQ);
                        let msg_id = frame.header(headers::HEADER_MESSAGE_ID);

                        if sum > 1 {
                            match reassembly.add_fragment(msg_id, sum, seq, frame.payload.clone().unwrap_or_default()) {
                                Some(combined) => combined,
                                None => continue, // waiting for more fragments
                            }
                        } else {
                            frame.payload.clone().unwrap_or_default()
                        }
                    };

                    // Build IncomingMessage with oneshot for response
                    let (tx_response, rx_response) = oneshot::channel();
                    let incoming = IncomingMessage::new(
                        payload,
                        frame.headers.clone(),
                        tx_response,
                    );

                    match self.config.registry.dispatch(msg_type, incoming).await {
                        Ok(()) => {}
                        Err(_) => {
                            tracing::warn!("no handler for {:?}, returning 200", msg_type);
                        }
                    }

                    // Wait for response (timeout at 30s)
                    let response_data = tokio::time::timeout(
                        Duration::from_secs(30),
                        rx_response,
                    )
                    .await
                    .unwrap_or_else(|_| Ok(b"{\"code\":200}".to_vec()))
                    .unwrap_or_else(|_| b"{\"code\":200}".to_vec());

                    // Build and send response frame
                    let resp_frame = build_response_frame(&frame, &response_data);
                    let _ = write_tx.send(encode_frame(&resp_frame)).await;
                }
                _ => {}
            }

            // Periodic reassembly cleanup
            reassembly.cleanup();
        }
    }
}

/// Build a response Frame echoing the original frame's fields but with new payload.
fn build_response_frame(original: &Frame, response_data: &[u8]) -> Frame {
    let mut headers = original.headers.clone();
    headers.push(crate::ws::proto::frame::Header {
        key: headers::HEADER_BIZ_RT.into(),
        value: "0".into(),
    });

    Frame {
        seq_id: original.seq_id,
        log_id: original.log_id,
        service: original.service,
        method: 1,
        headers,
        payload_encoding: Some("json".into()),
        payload_type: None,
        payload: Some(response_data.to_vec()),
        log_id_new: None,
    }
}
