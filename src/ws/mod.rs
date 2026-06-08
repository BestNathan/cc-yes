// src/ws/mod.rs
pub mod proto;
pub mod business;

pub use proto::client::{WsClient, WsConfig};
pub use business::registry::HandlerRegistry;
pub use business::handler::MessageHandler;
pub use business::types::IncomingMessage;
pub use business::event_types::{Event, EventHeader};
pub use business::handlers::event::EventHandler;
pub use business::handlers::card::{
    CardActionBody, Operator, CardAction, CardContext, ActionValue,
    CardActionHandler, CardResponse, Toast,
};
