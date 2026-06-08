use std::time::Duration;
use tokio::time;
use rand::Rng;

/// Reconnect scheduler with jitter and backoff.
pub struct ReconnectScheduler {
    /// Total reconnect attempts allowed (-1 = unlimited)
    pub max_attempts: i32,
    /// Base interval between attempts (seconds)
    pub interval: Duration,
    /// Initial jitter range (seconds)
    pub nonce: i32,
}

impl ReconnectScheduler {
    pub fn new(max_attempts: i32, interval_secs: i32, nonce_secs: i32) -> Self {
        Self {
            max_attempts,
            interval: Duration::from_secs(interval_secs as u64),
            nonce: nonce_secs,
        }
    }

    /// Wait for the next reconnect attempt. Returns None if max attempts
    /// reached. Applies initial jitter on first call.
    pub async fn wait(&self, attempt: i32) -> Option<()> {
        if self.max_attempts >= 0 && attempt >= self.max_attempts {
            return None;
        }

        if attempt == 0 && self.nonce > 0 {
            let jitter_ms = rand::thread_rng().gen_range(0..self.nonce * 1000);
            time::sleep(Duration::from_millis(jitter_ms as u64)).await;
        } else {
            time::sleep(self.interval).await;
        }

        Some(())
    }
}
