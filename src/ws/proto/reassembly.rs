use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Pending reassembly state for a single message_id.
struct Pending {
    sum: i32,
    fragments: Vec<Option<Vec<u8>>>,
    created: Instant,
}

/// Cache for reassembling multipart messages.
pub struct ReassemblyCache {
    pending: HashMap<String, Pending>,
    ttl: Duration,
}

impl ReassemblyCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            pending: HashMap::new(),
            ttl,
        }
    }

    /// Add a fragment. Returns Some(complete_payload) when all fragments
    /// are received, or None to wait for more fragments.
    pub fn add_fragment(
        &mut self,
        message_id: &str,
        sum: i32,
        seq: i32,
        payload: Vec<u8>,
    ) -> Option<Vec<u8>> {
        let entry = self.pending.entry(message_id.to_string()).or_insert_with(|| {
            Pending {
                sum,
                fragments: vec![None; sum as usize],
                created: Instant::now(),
            }
        });

        if seq >= 0 && (seq as usize) < entry.fragments.len() {
            entry.fragments[seq as usize] = Some(payload);
        }

        // Check if all fragments received
        if entry.fragments.iter().all(|f| f.is_some()) {
            let combined: Vec<u8> = entry
                .fragments
                .iter()
                .filter_map(|f| f.as_ref())
                .flat_map(|v| v.iter().copied())
                .collect();
            self.pending.remove(message_id);
            return Some(combined);
        }

        None
    }

    /// Remove expired entries. Call periodically.
    pub fn cleanup(&mut self) {
        let now = Instant::now();
        self.pending.retain(|_, v| now.duration_since(v.created) < self.ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_packet_no_reassembly() {
        let mut cache = ReassemblyCache::new(Duration::from_secs(5));
        let result = cache.add_fragment("msg-1", 1, 0, b"hello".to_vec());
        assert_eq!(result, Some(b"hello".to_vec()));
    }

    #[test]
    fn multipart_in_order() {
        let mut cache = ReassemblyCache::new(Duration::from_secs(5));
        assert!(cache.add_fragment("msg-2", 3, 0, b"hel".to_vec()).is_none());
        assert!(cache.add_fragment("msg-2", 3, 1, b"lo ".to_vec()).is_none());
        let result = cache.add_fragment("msg-2", 3, 2, b"world".to_vec());
        assert_eq!(result, Some(b"hello world".to_vec()));
    }

    #[test]
    fn multipart_out_of_order() {
        let mut cache = ReassemblyCache::new(Duration::from_secs(5));
        assert!(cache.add_fragment("msg-3", 3, 2, b"world".to_vec()).is_none());
        assert!(cache.add_fragment("msg-3", 3, 0, b"hel".to_vec()).is_none());
        let result = cache.add_fragment("msg-3", 3, 1, b"lo ".to_vec());
        assert_eq!(result, Some(b"hello world".to_vec()));
    }
}
