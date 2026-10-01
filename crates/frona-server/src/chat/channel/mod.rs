pub mod adapter;
pub mod attachment;
pub mod error;
pub mod hitl;
pub mod inbound;
pub mod models;
pub mod outbound;
pub mod registry;
pub mod render;
pub mod repository;
pub mod service;
pub mod signal;
pub mod supervisor;
pub mod typing;

pub const WEBHOOK_PATH_PREFIX: &str = "/api/webhooks/channels";

pub use error::{ChannelError, ChannelErrorKind, FailureKind};
pub use hitl::HitlDeliveryService;
pub use hitl::{
    Hitl, HitlDelivery, HitlKind, HitlOutcome, HitlRequest, HitlResponse, ResolveOutcome,
    VaultGrant, kind_for, render_default_text,
};
pub use inbound::{InboundDeliveryService, spawn_inference_dispatcher};
pub use models::{
    Channel, ChannelAdapter, ChannelCtx, ChannelFactory, ChannelManifest, ChannelStatus, ChatType,
    ConfigRef, CreateChannelRequest, DispatchMode, ExternalLink, SetupConfig, UpdateChannelRequest,
    external_chat_id,
};
pub use outbound::{CarrierStatus, OutboundDeliveryService};
pub use registry::ChannelRegistry;
pub use service::ChannelService;
pub use signal::{ChannelSignal, ChannelSignalSink};
pub use supervisor::ChannelSupervisor;
