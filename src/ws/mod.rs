// src/ws/mod.rs
pub mod proto;
pub mod business;

pub use proto::client::{WsClient, WsConfig};
pub use proto::error::WsError;
pub use proto::headers::MessageType;
pub use business::registry::HandlerRegistry;
pub use business::handler::MessageHandler;
pub use business::types::IncomingMessage;
pub use business::handlers::event::EventHandler;
pub use business::handlers::card::{CardActionHandler, CardResponse, Toast};
pub use business::event_types::{
    Event, EventHeader, CardActionBody, Operator, CardAction, CardContext, ActionValue,
};
