use prost::Message;
use super::frame::Frame;

/// Decode protobuf bytes into a Frame.
pub fn decode_frame(data: &[u8]) -> Result<Frame, prost::DecodeError> {
    Frame::decode(data)
}

/// Encode a Frame into protobuf bytes.
pub fn encode_frame(frame: &Frame) -> Vec<u8> {
    let mut buf = Vec::with_capacity(frame.encoded_len());
    frame.encode(&mut buf).expect("Frame encode should not fail");
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::frame::{Frame, Header};

    #[test]
    fn decode_encode_roundtrip() {
        let original = Frame {
            seq_id: 1,
            log_id: 99,
            service: 5,
            method: 1,
            headers: vec![Header { key: "type".into(), value: "event".into() }],
            payload_encoding: Some("json".into()),
            payload_type: None,
            payload: Some(b"{\"hello\":\"world\"}".to_vec()),
            log_id_new: None,
        };
        let encoded = encode_frame(&original);
        let decoded = decode_frame(&encoded).unwrap();
        assert_eq!(original, decoded);
    }
}
