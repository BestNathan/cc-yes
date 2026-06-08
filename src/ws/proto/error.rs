use std::fmt;

#[derive(Debug)]
pub enum WsError {
    /// HTTP bootstrap failed (network, auth, etc.)
    Bootstrap(String),
    /// WebSocket handshake rejected with status
    Handshake { status: u16, msg: String },
    /// WebSocket read/write I/O error (triggers reconnect for server errors)
    Io(std::io::Error),
    /// Protobuf frame decode failed (skip frame)
    Decode(prost::DecodeError),
    /// Message reassembly timed out (skip frame)
    ReassemblyTimeout,
    /// No handler registered for this message type (degraded, return 200)
    NoHandler(String),
    /// Handler task channel closed — task panicked or exited
    ChannelClosed,
}

impl fmt::Display for WsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bootstrap(msg) => write!(f, "bootstrap error: {}", msg),
            Self::Handshake { status, msg } => write!(f, "handshake error: {} {}", status, msg),
            Self::Io(e) => write!(f, "I/O error: {}", e),
            Self::Decode(e) => write!(f, "decode error: {}", e),
            Self::ReassemblyTimeout => write!(f, "reassembly timeout"),
            Self::NoHandler(t) => write!(f, "no handler for type: {}", t),
            Self::ChannelClosed => write!(f, "handler channel closed"),
        }
    }
}

impl std::error::Error for WsError {}

impl From<std::io::Error> for WsError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<prost::DecodeError> for WsError {
    fn from(e: prost::DecodeError) -> Self {
        Self::Decode(e)
    }
}

/// Severity determines whether to trigger reconnect or skip.
#[derive(Debug, PartialEq)]
pub enum Severity {
    /// Trigger reconnect
    Fatal,
    /// Skip current frame, continue
    Skip,
    /// Return default response, continue
    Degraded,
}

impl WsError {
    pub fn severity(&self) -> Severity {
        match self {
            Self::Io(_) | Self::ChannelClosed => Severity::Fatal,
            Self::Decode(_) | Self::ReassemblyTimeout => Severity::Skip,
            Self::Bootstrap(_) | Self::Handshake { .. } | Self::NoHandler(_) => Severity::Degraded,
        }
    }
}

/// Client-side error — stops reconnection.
pub struct ClientError {
    pub code: i32,
    pub message: String,
}

impl ClientError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "client error [{}]: {}", self.code, self.message)
    }
}

impl fmt::Debug for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for ClientError {}
