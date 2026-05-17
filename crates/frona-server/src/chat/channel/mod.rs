pub mod adapter;
pub mod manager;
pub mod models;
pub mod registry;
pub mod repository;
pub mod service;

pub const WEBHOOK_PATH_PREFIX: &str = "/api/webhooks/channels";

pub use manager::{ChannelManager, spawn_inference_dispatcher};
pub use models::{
    Channel, ChannelAdapter, ChannelCtx, ChannelFactory, ChannelManifest, ChannelStatus, ChatType,
    ConfigRef, CreateChannelRequest, DispatchMode, ExternalLink, SetupConfig,
    UpdateChannelRequest, external_chat_id,
};
pub use registry::ChannelRegistry;
pub use service::ChannelService;
