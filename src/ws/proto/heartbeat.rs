use std::time::Duration;
use tokio::sync::watch;
use tokio::time;
use crate::ws::proto::frame::{Frame, Header};
use crate::ws::proto::codec::encode_frame;
use crate::ws::proto::headers;

/// Build a ping control frame.
pub fn build_ping_frame(service_id: i32, seq_id: u64) -> Frame {
    Frame {
        seq_id,
        log_id: 0,
        service: service_id,
        method: headers::FRAME_TYPE_CONTROL,
        headers: vec![Header {
            key: headers::HEADER_TYPE.into(),
            value: "ping".into(),
        }],
        payload_encoding: None,
        payload_type: None,
        payload: None,
        log_id_new: None,
    }
}

/// Spawn a heartbeat task that sends ping frames at the configured interval.
/// Returns a watch sender for dynamically updating the ping interval.
pub fn start_heartbeat(
    service_id: i32,
    write_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    initial_interval: Duration,
) -> watch::Sender<Duration> {
    let (interval_tx, mut interval_rx) = watch::channel(initial_interval);
    let mut seq_id: u64 = 1;

    tokio::spawn(async move {
        loop {
            let interval = *interval_rx.borrow();
            time::sleep(interval).await;

            let frame = build_ping_frame(service_id, seq_id);
            seq_id += 1;
            let data = encode_frame(&frame);

            if write_tx.send(data).await.is_err() {
                // Write channel closed — connection is dead
                break;
            }
        }
    });

    interval_tx
}

/// Update ping interval from a pong frame's ClientConfig payload.
pub fn update_from_pong(payload: &[u8], interval_tx: &watch::Sender<Duration>) {
    if payload.is_empty() {
        return;
    }
    if let Ok(conf) = serde_json::from_slice::<serde_json::Value>(payload) {
        if let Some(pi) = conf["PingInterval"].as_i64() {
            let new_interval = Duration::from_secs(pi as u64);
            let _ = interval_tx.send(new_interval);
        }
    }
}
