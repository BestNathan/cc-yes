use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct Header {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct Frame {
    #[prost(uint64, tag = "1")]
    pub seq_id: u64,
    #[prost(uint64, tag = "2")]
    pub log_id: u64,
    #[prost(int32, tag = "3")]
    pub service: i32,
    #[prost(int32, tag = "4")]
    pub method: i32,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<Header>,
    #[prost(string, optional, tag = "6")]
    pub payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    pub payload_type: Option<String>,
    #[prost(bytes, optional, tag = "8")]
    pub payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    pub log_id_new: Option<String>,
}

impl Frame {
    /// Get a header value by key, returning empty string if not found.
    pub fn header(&self, key: &str) -> &str {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
            .unwrap_or("")
    }

    /// Get a header value as i32, defaulting to 0.
    pub fn header_int(&self, key: &str) -> i32 {
        self.header(key).parse().unwrap_or(0)
    }

    /// Message type from "type" header: "event", "card", "ping", "pong"
    pub fn msg_type(&self) -> &str {
        self.header("type")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_encode_decode_roundtrip() {
        let frame = Frame {
            seq_id: 42,
            log_id: 100,
            service: 33554678,
            method: 1,
            headers: vec![
                Header { key: "type".into(), value: "event".into() },
                Header { key: "message_id".into(), value: "msg-1".into() },
            ],
            payload_encoding: Some("json".into()),
            payload_type: None,
            payload: Some(b"{\"test\":true}".to_vec()),
            log_id_new: None,
        };

        let mut buf = Vec::new();
        frame.encode(&mut buf).unwrap();
        let decoded = Frame::decode(buf.as_slice()).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn control_frame_method_zero() {
        let frame = Frame {
            seq_id: 1,
            log_id: 0,
            service: 123,
            method: 0,
            headers: vec![Header { key: "type".into(), value: "ping".into() }],
            payload_encoding: None,
            payload_type: None,
            payload: None,
            log_id_new: None,
        };
        assert_eq!(frame.method, 0);
        assert_eq!(frame.msg_type(), "ping");
    }

    #[test]
    fn data_frame_method_one() {
        let frame = Frame {
            seq_id: 1,
            log_id: 0,
            service: 123,
            method: 1,
            headers: vec![
                Header { key: "type".into(), value: "event".into() },
                Header { key: "sum".into(), value: "1".into() },
                Header { key: "seq".into(), value: "0".into() },
            ],
            payload_encoding: Some("json".into()),
            payload_type: None,
            payload: Some(b"{}".to_vec()),
            log_id_new: None,
        };
        assert_eq!(frame.method, 1);
        assert_eq!(frame.header_int("sum"), 1);
    }
}
