//! A cost report is the durable output of one cost-analysis run: the numbers
//! the analysis was based on, and the recommendations drawn from them.
//!
//! It is deliberately a *record*, not a command. Nothing in this module can
//! change a model group, a provider, or a config file — the whole point is
//! that a human reads it and decides.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use frona_derive::Entity;

use crate::inference::usage::UsageRollup;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "cost_report")]
pub struct CostReport {
    pub id: String,
    /// The admin whose run produced this. The *content* is instance-wide; this
    /// is who to show it to and who it cascades with on account deletion.
    pub user_id: String,

    pub window_since: DateTime<Utc>,
    pub window_until: DateTime<Utc>,

    /// Markdown narrative written by the analyst.
    pub summary: String,
    pub recommendations: Vec<CostRecommendation>,

    /// Instance-wide totals for the window, at list price.
    pub totals: UsageRollup,

    /// Money that actually left the account: list-price cost of calls served by
    /// providers configured as metered.
    pub metered_cost_usd: f64,
    /// Subscription fees attributable to the window, pro-rated from the
    /// configured monthly cost.
    pub subscription_cost_usd: f64,
    /// List-price value of everything served under a subscription. Compared
    /// against `subscription_cost_usd` this is what answers "is the plan worth
    /// its fee?" — the single reason the billing model exists at all.
    pub subscription_list_value_usd: f64,

    /// Sum of the recommendations' monthly deltas, when the analyst quantified
    /// them. Negative means a saving. `None` when nothing was quantifiable.
    pub estimated_monthly_savings_usd: Option<f64>,

    /// Catalogue version the repricing used, so a report can be read against
    /// the prices that were current when it was written.
    pub pricing_version: String,

    pub created_at: DateTime<Utc>,
}

/// One actionable suggestion. `estimated_monthly_delta_usd` is signed:
/// negative saves money, positive costs more (which a recommendation may still
/// legitimately propose — a faster or more capable model).
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct CostRecommendation {
    pub kind: RecommendationKind,
    /// The model group the change would apply to. This is the unit an operator
    /// can actually edit in config, which is why the analysis groups by it.
    #[serde(default)]
    pub model_group: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    pub rationale: String,
    #[serde(default)]
    pub estimated_monthly_delta_usd: Option<f64>,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "snake_case")]
#[surreal(crate = "surrealdb::types", snake_case)]
pub enum RecommendationKind {
    /// Point a model group at a different model.
    SwitchModel,
    /// Move traffic between providers serving comparable models.
    RebalanceProvider,
    /// Prompt caching is off (or ineffective) on a workload that would benefit.
    EnableCaching,
    /// A subscription is costing more than the usage it covers is worth.
    SubscriptionUnderused,
    /// Usage has outgrown a subscription's allowance and is spilling into
    /// metered overage.
    SubscriptionOverrun,
    /// Calls are recording no cost at all because the catalogue has no pricing
    /// for the model — spend is invisible rather than zero.
    PricingGap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "snake_case")]
#[surreal(crate = "surrealdb::types", snake_case)]
pub enum Confidence {
    High,
    Medium,
    Low,
}
