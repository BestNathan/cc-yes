use super::frame::Header;

// Header key constants
pub const HEADER_TIMESTAMP: &str = "timestamp";
pub const HEADER_TYPE: &str = "type";
pub const HEADER_MESSAGE_ID: &str = "message_id";
pub const HEADER_SUM: &str = "sum";
pub const HEADER_SEQ: &str = "seq";
pub const HEADER_TRACE_ID: &str = "trace_id";
pub const HEADER_INSTANCE_ID: &str = "instance_id";
pub const HEADER_BIZ_RT: &str = "biz_rt";
pub const HEADER_HANDSHAKE_STATUS: &str = "Handshake-Status";
pub const HEADER_HANDSHAKE_MSG: &str = "Handshake-Msg";
pub const HEADER_HANDSHAKE_AUTH_ERR_CODE: &str = "Handshake-Autherrcode";

// Message types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageType {
    Event,
    Card,
}

impl MessageType {
    pub fn from_header(value: &str) -> Option<Self> {
        match value {
            "event" => Some(Self::Event),
            "card" => Some(Self::Card),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Event => "event",
            Self::Card => "card",
        }
    }
}

// Frame types
pub const FRAME_TYPE_CONTROL: i32 = 0;
pub const FRAME_TYPE_DATA: i32 = 1;

// Error codes
pub const ERR_OK: i32 = 0;
pub const ERR_SYSTEM_BUSY: i32 = 1;
pub const ERR_FORBIDDEN: i32 = 403;
pub const ERR_AUTH_FAILED: i32 = 514;
pub const ERR_EXCEED_CONN_LIMIT: i32 = 1000040350;
pub const ERR_INTERNAL: i32 = 1000040343;

/// Wrapper around Vec<Header> for convenient access.
pub struct Headers<'a>(pub &'a [Header]);

impl<'a> Headers<'a> {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find(|h| h.key == key).map(|h| h.value.as_str())
    }

    pub fn get_int(&self, key: &str) -> i32 {
        self.get(key).and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    pub fn msg_type(&self) -> Option<MessageType> {
        self.get(HEADER_TYPE).and_then(MessageType::from_header)
    }
}

/// Builder for constructing header lists.
pub struct HeaderBuilder {
    headers: Vec<Header>,
}

impl HeaderBuilder {
    pub fn new() -> Self {
        Self { headers: Vec::new() }
    }

    pub fn add(mut self, key: &str, value: &str) -> Self {
        self.headers.push(Header { key: key.into(), value: value.into() });
        self
    }

    pub fn msg_type(self, mt: MessageType) -> Self {
        self.add(HEADER_TYPE, mt.as_str())
    }

    pub fn build(self) -> Vec<Header> {
        self.headers
    }
}
