//! Instance-wide cost analysis: what the server spent, and what it would have
//! spent elsewhere.
//!
//! Nothing here is scoped to a user — every read spans the whole instance —
//! so every entry point is gated by `PolicyAction::ViewUsageAnalytics` at the
//! route or tool layer above.

pub mod models;
pub mod repository;
pub mod service;

pub use models::{Confidence, CostRecommendation, CostReport, RecommendationKind};
pub use repository::CostReportRepository;
pub use service::{
    AllowanceStatus, CapabilityRequirements, CostService, ModelComparison, ModelComparisonSet,
    ObservedMix, ProviderSpend, SpendAnalysis, monthly_scale, reprice_observed, window_from_days,
};
