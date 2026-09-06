//! Cost analysis: what the instance spent, and what it would have spent on a
//! different model.
//!
//! Everything the agent is told about money originates here rather than from
//! the model's own memory of list prices. The repricing path shares
//! [`ModelEntry::cost_for`] with the live costing path in
//! `ModelCatalogStore::compute`, so a recommendation and an invoice cannot
//! drift apart — see `reprice_observed` and the round-trip test at the bottom
//! of this file.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use rig_core::completion::request::Usage;
use serde::Serialize;

use crate::core::config::{Config, ProviderBilling, ProviderBillingKind};
use crate::core::error::AppError;
use crate::core::repository::{Repository, new_id};
use crate::db::repo::generic::SurrealRepo;
use crate::inference::metadata::{ModelCatalogStore, ModelEntry};
use crate::inference::usage::{
    InferenceUsage, InferenceUsageRepository, ModelSpendRow, UsageRollup, UserCostRow,
};
use crate::notification::models::{NotificationData, NotificationLevel};
use crate::notification::service::NotificationService;

use super::models::{CostRecommendation, CostReport};
use super::repository::CostReportRepository;

/// Days in a nominal billing period. Pro-rating a monthly fee onto an
/// arbitrary window has no exact answer; a cost report is a decision aid, not
/// an invoice.
const NOMINAL_PERIOD_DAYS: f64 = 30.0;

#[derive(Clone)]
pub struct CostService {
    usage_repo: SurrealRepo<InferenceUsage>,
    report_repo: SurrealRepo<CostReport>,
    catalog: ModelCatalogStore,
    config: Arc<Config>,
    notifications: NotificationService,
}

impl CostService {
    pub fn new(
        usage_repo: SurrealRepo<InferenceUsage>,
        report_repo: SurrealRepo<CostReport>,
        catalog: ModelCatalogStore,
        config: Arc<Config>,
        notifications: NotificationService,
    ) -> Self {
        Self {
            usage_repo,
            report_repo,
            catalog,
            config,
            notifications,
        }
    }

    /// Instance-wide spend for a window, split by how each provider bills.
    ///
    /// **Callers must have checked `PolicyAction::ViewUsageAnalytics` first.**
    /// This reads every user's rows and performs no authorization of its own.
    pub async fn analyse(
        &self,
        since: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Result<SpendAnalysis, AppError> {
        let (s, u) = (Some(since), Some(until));
        let totals = self.usage_repo.aggregate_all(s, u).await?;
        let provider_rows = self.usage_repo.aggregate_by_provider_all(s, u).await?;
        let models = self.usage_repo.aggregate_by_model_all(s, u).await?;
        let by_model_group = self.usage_repo.aggregate_by_model_group_all(s, u).await?;
        let by_kind = self.usage_repo.aggregate_by_kind_all(s, u).await?;
        let top_users = self.usage_repo.top_users_by_cost(s, u, 20).await?;

        let window_days = ((until - since).num_seconds() as f64 / 86_400.0).max(f64::MIN_POSITIVE);

        // Fold the per-(provider, billing_kind) rows into one entry per
        // provider, keeping each billing regime's list value separate. A
        // provider that changed plans mid-window contributes to both.
        let mut providers: HashMap<String, ProviderSpend> = HashMap::new();
        for row in provider_rows {
            let billing = self.billing_for(&row.provider);
            let entry = providers
                .entry(row.provider.clone())
                .or_insert_with(|| ProviderSpend {
                    provider: row.provider.clone(),
                    billing: billing.clone(),
                    rollup: UsageRollup::default(),
                    recorded_kinds: Vec::new(),
                    prorated_fee_usd: billing.prorated_cost(window_days),
                    allowance: None,
                });
            entry.rollup.input_tokens += row.input_tokens;
            entry.rollup.cached_input_tokens += row.cached_input_tokens;
            entry.rollup.output_tokens += row.output_tokens;
            entry.rollup.cost_usd += row.cost_usd;
            entry.rollup.calls += row.calls;

            let recorded = ProviderBillingKind::from_str_or_metered(&row.billing_kind);
            if !entry.recorded_kinds.contains(&recorded) {
                entry.recorded_kinds.push(recorded);
            }
        }

        // Split list-price value by the kind the *rows* recorded, not by
        // today's config: a provider moved onto a subscription last week must
        // not have last month's metered spend retroactively reclassified.
        let mut metered_cost_usd = 0.0;
        let mut subscription_list_value_usd = 0.0;
        let mut self_hosted_list_value_usd = 0.0;
        for m in &models {
            match ProviderBillingKind::from_str_or_metered(&m.billing_kind) {
                ProviderBillingKind::Metered => metered_cost_usd += m.cost_usd,
                ProviderBillingKind::Subscription => subscription_list_value_usd += m.cost_usd,
                ProviderBillingKind::SelfHosted => self_hosted_list_value_usd += m.cost_usd,
            }
        }

        for entry in providers.values_mut() {
            entry.allowance = allowance_status(&entry.billing, &entry.rollup);
        }
        let subscription_cost_usd = providers.values().map(|p| p.prorated_fee_usd).sum();

        let mut providers: Vec<ProviderSpend> = providers.into_values().collect();
        providers.sort_by(|a, b| b.rollup.cost_usd.total_cmp(&a.rollup.cost_usd));

        let uncosted_calls = models.iter().map(|m| m.uncosted_calls).sum();

        Ok(SpendAnalysis {
            window_since: since,
            window_until: until,
            window_days,
            totals,
            metered_cost_usd,
            subscription_cost_usd,
            subscription_list_value_usd,
            self_hosted_list_value_usd,
            uncosted_calls,
            providers,
            models,
            by_model_group,
            by_kind,
            top_users,
            pricing_version: self.catalog.current().version.clone(),
        })
    }

    /// Configured billing terms for a provider, with the unstated cases filled
    /// in (see `ModelProviderConfig::effective_billing`).
    pub fn billing_for(&self, provider: &str) -> ProviderBilling {
        self.config
            .providers
            .get(provider)
            .map(|c| c.effective_billing(provider))
            .unwrap_or_else(|| ProviderBilling {
                kind: ProviderBillingKind::default_for_provider(provider),
                ..ProviderBilling::default()
            })
    }

    /// Every configured provider with its billing terms, whether or not it was
    /// used in the window.
    pub fn provider_billing(&self) -> Vec<(String, ProviderBilling, bool)> {
        let mut rows: Vec<(String, ProviderBilling, bool)> = self
            .config
            .providers
            .iter()
            .map(|(name, cfg)| (name.clone(), cfg.effective_billing(name), cfg.enabled))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    /// Reprice an observed token mix against candidate models, rejecting any
    /// that cannot do the job.
    ///
    /// The mix is the one actually served — real prompt sizes, real cache hit
    /// rate, real output lengths — so this answers "what would last month have
    /// cost on X?" rather than "what does X cost per million tokens?".
    pub fn compare_models(
        &self,
        observed: ObservedMix,
        baseline_model_ref: Option<&str>,
        candidates: &[String],
        requirements: &CapabilityRequirements,
    ) -> ModelComparisonSet {
        let snapshot = self.catalog.current();
        let baseline_cost = baseline_model_ref
            .and_then(|r| split_model_ref(r))
            .and_then(|(p, m)| snapshot.lookup_prefix(p, m))
            .and_then(|e| reprice_observed(e, &observed));

        let comparisons = candidates
            .iter()
            .map(|candidate| {
                let Some((provider, model_id)) = split_model_ref(candidate) else {
                    return ModelComparison::rejected(
                        candidate,
                        vec![
                            "not a 'provider/model' reference, e.g. 'anthropic/claude-opus-4-5'"
                                .to_string(),
                        ],
                    );
                };
                let Some(entry) = snapshot.lookup_prefix(provider, model_id) else {
                    return ModelComparison::rejected(
                        candidate,
                        vec![format!(
                            "not in the model catalogue (version {}), so it cannot be priced",
                            snapshot.version
                        )],
                    );
                };

                let blockers = requirements.blockers(entry);
                if !blockers.is_empty() {
                    return ModelComparison::rejected(candidate, blockers);
                }

                match reprice_observed(entry, &observed) {
                    Some(cost_usd) => ModelComparison {
                        model_ref: candidate.clone(),
                        cost_usd: Some(cost_usd),
                        delta_usd: baseline_cost.map(|b| cost_usd - b),
                        delta_pct: baseline_cost.and_then(|b| {
                            (b > 0.0).then(|| (cost_usd - b) / b * 100.0)
                        }),
                        max_input_tokens: entry.max_input_tokens(),
                        supports_tool_calling: entry.supports_function_calling(),
                        supports_vision: entry.supports_vision(),
                        supports_reasoning: entry.supports_reasoning(),
                        supports_prompt_caching: entry.supports_prompt_caching(),
                        supports_structured_output: entry.supports_response_schema(),
                        status: entry.status.clone(),
                        rejected_because: Vec::new(),
                    },
                    // Open-weights models are catalogued without hosted pricing.
                    None => ModelComparison::rejected(
                        candidate,
                        vec![
                            "the catalogue publishes no price for this model, so a saving cannot be estimated"
                                .to_string(),
                        ],
                    ),
                }
            })
            .collect();

        ModelComparisonSet {
            baseline_model_ref: baseline_model_ref.map(str::to_string),
            baseline_cost_usd: baseline_cost,
            observed,
            pricing_version: snapshot.version.clone(),
            comparisons,
        }
    }

    /// Persist a report and raise it in the notification feed.
    pub async fn save_report(
        &self,
        user_id: &str,
        analysis: &SpendAnalysis,
        summary: String,
        recommendations: Vec<CostRecommendation>,
    ) -> Result<CostReport, AppError> {
        let estimated_monthly_savings_usd = {
            let deltas: Vec<f64> = recommendations
                .iter()
                .filter_map(|r| r.estimated_monthly_delta_usd)
                .collect();
            (!deltas.is_empty()).then(|| deltas.iter().sum())
        };

        let report = CostReport {
            id: new_id(),
            user_id: user_id.to_string(),
            window_since: analysis.window_since,
            window_until: analysis.window_until,
            summary,
            recommendations,
            totals: analysis.totals.clone(),
            metered_cost_usd: analysis.metered_cost_usd,
            subscription_cost_usd: analysis.subscription_cost_usd,
            subscription_list_value_usd: analysis.subscription_list_value_usd,
            estimated_monthly_savings_usd,
            pricing_version: analysis.pricing_version.clone(),
            created_at: Utc::now(),
        };
        let report = self.report_repo.create(&report).await?;

        let body = match estimated_monthly_savings_usd {
            Some(d) if d < 0.0 => format!(
                "{} recommendation(s), up to ${:.2}/month in savings identified.",
                report.recommendations.len(),
                d.abs()
            ),
            _ => format!(
                "{} recommendation(s) from ${:.2} of metered spend.",
                report.recommendations.len(),
                report.metered_cost_usd
            ),
        };
        // A failed notification must not lose the report that is already saved.
        if let Err(e) = self
            .notifications
            .create_and_notify(
                user_id,
                NotificationData::CostReport {
                    report_id: report.id.clone(),
                },
                NotificationLevel::Info,
                "Cost report ready".to_string(),
                body,
            )
            .await
        {
            tracing::warn!(error = %e, report_id = %report.id, "cost report saved but notification failed");
        }

        Ok(report)
    }

    pub async fn list_reports(&self, limit: u32) -> Result<Vec<CostReport>, AppError> {
        self.report_repo.list_recent(limit).await
    }

    pub async fn get_report(&self, id: &str) -> Result<Option<CostReport>, AppError> {
        self.report_repo.find_by_id(id).await
    }
}

/// The FRESH-input view of a set of persisted usage rows — the shape
/// [`ModelEntry::cost_for`] consumes.
///
/// Persisted rows carry the TOTAL-prompt convention (`input_tokens` is the
/// whole prompt, `cached_input_tokens` a labelled subset), so fresh input is
/// the difference. Going through `cost_for` directly, rather than through
/// `ModelCatalogStore::compute`, is deliberate: `compute` first normalizes a
/// raw provider `Usage`, and inverting that per-provider normalization to
/// reprice against a *different* provider would be a second place for the
/// convention to drift. Both paths meet at `cost_for`.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ObservedMix {
    pub fresh_input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub calls: u64,
}

impl ObservedMix {
    pub fn from_rows<'a>(rows: impl IntoIterator<Item = &'a ModelSpendRow>) -> Self {
        let mut mix = Self::default();
        for r in rows {
            mix.fresh_input_tokens += r.input_tokens.saturating_sub(r.cached_input_tokens);
            mix.cached_input_tokens += r.cached_input_tokens;
            mix.output_tokens += r.output_tokens;
            mix.calls += r.calls;
        }
        mix
    }

    fn as_usage(&self) -> Usage {
        Usage {
            input_tokens: self.fresh_input_tokens,
            cached_input_tokens: self.cached_input_tokens,
            // Persisted rows don't break cache writes out of `input_tokens`
            // (rig collapses them on the streaming path), so a repriced cache
            // write is charged at the candidate's fresh-input rate rather than
            // its cache-write premium. On Anthropic, which is the only vendor
            // publishing a separate write rate, that understates the candidate
            // slightly — worth knowing when a comparison is close.
            cache_creation_input_tokens: 0,
            output_tokens: self.output_tokens,
            total_tokens: self.fresh_input_tokens + self.cached_input_tokens + self.output_tokens,
            // `cost_for` prices only the four fields above. The rest of rig's
            // `Usage` is breakdown detail providers report inconsistently —
            // reasoning tokens are already inside `output_tokens` on every
            // provider we bill against — so defaulting them keeps this mix
            // priced exactly as the live path prices a real call.
            ..Usage::default()
        }
    }
}

/// Value an observed mix at a candidate model's published rates.
pub fn reprice_observed(entry: &ModelEntry, observed: &ObservedMix) -> Option<f64> {
    entry.cost_for(&observed.as_usage())
}

/// What a replacement model has to be able to do. Derived from what the model
/// group is actually used for, never guessed by the agent.
#[derive(Debug, Clone, Default, serde::Deserialize, Serialize)]
pub struct CapabilityRequirements {
    #[serde(default)]
    pub needs_tool_calling: bool,
    #[serde(default)]
    pub needs_vision: bool,
    #[serde(default)]
    pub needs_reasoning: bool,
    #[serde(default)]
    pub needs_structured_output: bool,
    /// Context window the group is configured for. A candidate below this
    /// would trigger compaction earlier and cost extra summarisation calls.
    #[serde(default)]
    pub min_context_tokens: Option<u64>,
}

impl CapabilityRequirements {
    /// Reasons this model cannot do the job. Empty means it can.
    pub fn blockers(&self, entry: &ModelEntry) -> Vec<String> {
        let mut out = Vec::new();
        if self.needs_tool_calling && !entry.supports_function_calling() {
            out.push("does not support tool calling".to_string());
        }
        if self.needs_vision && !entry.supports_vision() {
            out.push("does not accept image input".to_string());
        }
        if self.needs_reasoning && !entry.supports_reasoning() {
            out.push("does not support reasoning".to_string());
        }
        if self.needs_structured_output && !entry.supports_response_schema() {
            out.push("does not support structured output".to_string());
        }
        if let Some(required) = self.min_context_tokens {
            match entry.max_input_tokens() {
                Some(have) if have < required => out.push(format!(
                    "context window {have} is below the {required} the model group is configured for"
                )),
                None => out.push("catalogue publishes no context window".to_string()),
                _ => {}
            }
        }
        if entry.status.as_deref() == Some("deprecated") {
            out.push("marked deprecated upstream".to_string());
        }
        out
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelComparisonSet {
    pub baseline_model_ref: Option<String>,
    pub baseline_cost_usd: Option<f64>,
    pub observed: ObservedMix,
    pub pricing_version: String,
    pub comparisons: Vec<ModelComparison>,
}

/// A candidate model, priced against the observed mix — or the reasons it was
/// rejected. A rejected candidate still appears, so the reason is visible
/// rather than the model silently vanishing from the shortlist.
#[derive(Debug, Clone, Serialize)]
pub struct ModelComparison {
    pub model_ref: String,
    pub cost_usd: Option<f64>,
    pub delta_usd: Option<f64>,
    pub delta_pct: Option<f64>,
    pub max_input_tokens: Option<u64>,
    pub supports_tool_calling: bool,
    pub supports_vision: bool,
    pub supports_reasoning: bool,
    pub supports_prompt_caching: bool,
    pub supports_structured_output: bool,
    pub status: Option<String>,
    /// Non-empty means this model must not be recommended.
    pub rejected_because: Vec<String>,
}

impl ModelComparison {
    fn rejected(model_ref: &str, reasons: Vec<String>) -> Self {
        Self {
            model_ref: model_ref.to_string(),
            cost_usd: None,
            delta_usd: None,
            delta_pct: None,
            max_input_tokens: None,
            supports_tool_calling: false,
            supports_vision: false,
            supports_reasoning: false,
            supports_prompt_caching: false,
            supports_structured_output: false,
            status: None,
            rejected_because: reasons,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SpendAnalysis {
    pub window_since: DateTime<Utc>,
    pub window_until: DateTime<Utc>,
    pub window_days: f64,
    pub totals: UsageRollup,
    /// Money actually spent: list-price cost of metered-provider calls.
    pub metered_cost_usd: f64,
    /// Subscription fees attributable to the window.
    pub subscription_cost_usd: f64,
    /// List-price value of subscription-covered calls. Against
    /// `subscription_cost_usd`, this is whether the plan earns its fee.
    pub subscription_list_value_usd: f64,
    /// List-price value of self-hosted calls — never billed, but it is what
    /// running the same work on a hosted provider would have cost.
    pub self_hosted_list_value_usd: f64,
    /// Calls the catalogue had no price for. Their spend is unmeasured, not
    /// zero, and every total above understates by that amount.
    pub uncosted_calls: u64,
    pub providers: Vec<ProviderSpend>,
    pub models: Vec<ModelSpendRow>,
    pub by_model_group: HashMap<String, UsageRollup>,
    pub by_kind: HashMap<String, UsageRollup>,
    pub top_users: Vec<UserCostRow>,
    pub pricing_version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderSpend {
    pub provider: String,
    /// Terms configured *now*.
    pub billing: ProviderBilling,
    /// Terms the rows in this window were written under. More than one entry
    /// means the plan changed mid-window.
    pub recorded_kinds: Vec<ProviderBillingKind>,
    pub rollup: UsageRollup,
    pub prorated_fee_usd: f64,
    pub allowance: Option<AllowanceStatus>,
}

/// How much of a subscription's allowance the window consumed. `None` when the
/// provider declares no allowance — the common case, and not a problem.
#[derive(Debug, Clone, Serialize)]
pub struct AllowanceStatus {
    pub included_tokens: Option<u64>,
    pub tokens_used: u64,
    pub included_spend_usd: Option<f64>,
    pub list_value_used_usd: f64,
    /// Fraction of the allowance consumed, as a percentage. Over 100 means the
    /// window exceeded it.
    pub used_pct: f64,
    pub exceeded: bool,
    pub overage_is_metered: bool,
}

fn allowance_status(billing: &ProviderBilling, rollup: &UsageRollup) -> Option<AllowanceStatus> {
    if billing.kind != ProviderBillingKind::Subscription {
        return None;
    }
    let tokens_used = rollup.input_tokens + rollup.output_tokens;
    let list_value_used_usd = rollup.cost_usd;

    // Whichever allowance the operator declared decides the percentage; if
    // both, the tighter one binds, since that is the one that runs out first.
    let token_pct = billing
        .included_tokens
        .filter(|t| *t > 0)
        .map(|t| tokens_used as f64 / t as f64 * 100.0);
    let spend_pct = billing
        .included_spend_usd
        .filter(|s| *s > 0.0)
        .map(|s| list_value_used_usd / s * 100.0);
    let used_pct = match (token_pct, spend_pct) {
        (Some(a), Some(b)) => a.max(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return None,
    };

    Some(AllowanceStatus {
        included_tokens: billing.included_tokens,
        tokens_used,
        included_spend_usd: billing.included_spend_usd,
        list_value_used_usd,
        used_pct,
        exceeded: used_pct > 100.0,
        overage_is_metered: billing.overage_is_metered,
    })
}

/// Split a `"provider/model"` reference. The model half may itself contain
/// slashes (`openrouter/anthropic/claude-sonnet-4.5`), which `lookup_prefix`
/// handles, so only the first separator is significant.
fn split_model_ref(model_ref: &str) -> Option<(&str, &str)> {
    model_ref
        .split_once('/')
        .filter(|(p, m)| !p.is_empty() && !m.is_empty())
}

/// Convenience for callers naming a window in days rather than timestamps.
pub fn window_from_days(days: u32) -> (DateTime<Utc>, DateTime<Utc>) {
    let until = Utc::now();
    (until - Duration::days(days.max(1) as i64), until)
}

/// Nominal-period scaling factor for a window, so a 7-day observation can be
/// quoted as a monthly figure.
pub fn monthly_scale(window_days: f64) -> f64 {
    if window_days <= 0.0 {
        return 0.0;
    }
    NOMINAL_PERIOD_DAYS / window_days
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::ModelProviderConfig;
    use crate::inference::metadata::catalog::{Cost, Limit, ModelEntry};
    use crate::inference::metadata::{ModelCatalogSnapshot, ModelCatalogStore};
    use crate::inference::provider::ModelRef;

    fn priced_entry() -> ModelEntry {
        ModelEntry {
            tool_call: true,
            attachment: true,
            reasoning: true,
            structured_output: true,
            limit: Limit {
                context: 200_000,
                output: 32_000,
                input: None,
            },
            cost: Some(Cost {
                input: 3.0,
                output: 15.0,
                cache_read: Some(0.3),
                cache_write: Some(3.75),
            }),
            ..Default::default()
        }
    }

    fn row(model_ref: &str, input: u64, cached: u64, output: u64, calls: u64) -> ModelSpendRow {
        ModelSpendRow {
            model_ref: model_ref.to_string(),
            provider: model_ref.split('/').next().unwrap_or("openai").to_string(),
            model_group: "primary".to_string(),
            billing_kind: "metered".to_string(),
            input_tokens: input,
            cached_input_tokens: cached,
            output_tokens: output,
            cost_usd: 0.0,
            calls,
            duration_ms_mean: None,
            uncosted_calls: 0,
        }
    }

    /// The drift guard. A recommendation and an invoice must price the same
    /// traffic identically, so repricing a stored row has to land on exactly
    /// what `ModelCatalogStore::compute` charged for the live call that wrote
    /// it. Both meet at `ModelEntry::cost_for`; this pins that they still do,
    /// through the persisted row's TOTAL-prompt convention and back.
    #[test]
    fn repricing_agrees_with_live_costing_for_every_provider_shape() {
        for provider in ["anthropic", "openai"] {
            let entry = priced_entry();
            let mut snapshot = ModelCatalogSnapshot::empty();
            snapshot
                .entries
                .insert(format!("{provider}/test-model"), entry.clone());
            let store = ModelCatalogStore::new(snapshot);
            let model_ref = ModelRef {
                model_id: "test-model".to_string(),
                provider: crate::core::config::ProviderModel::from_name(provider),
            };

            // A raw provider response, in that provider's native shape.
            let raw = match provider {
                // Anthropic reports the three prompt figures as disjoint.
                "anthropic" => Usage {
                    input_tokens: 8_000,
                    cached_input_tokens: 2_000,
                    output_tokens: 1_500,
                    total_tokens: 11_500,
                    ..Usage::default()
                },
                // OpenAI-shaped: `input_tokens` is the whole prompt.
                _ => Usage {
                    input_tokens: 10_000,
                    cached_input_tokens: 2_000,
                    output_tokens: 1_500,
                    total_tokens: 11_500,
                    ..Usage::default()
                },
            };

            let (live_cost, _) = store.compute(&model_ref, &raw);
            let live_cost = live_cost.expect("catalog entry is priced");

            // What `build_row` would have persisted for that call.
            let persisted = crate::inference::metadata::total_prompt_usage(provider, &raw);
            let stored = row(
                &format!("{provider}/test-model"),
                persisted.input_tokens,
                persisted.cached_input_tokens,
                persisted.output_tokens,
                1,
            );

            let repriced = reprice_observed(&entry, &ObservedMix::from_rows([&stored]))
                .expect("repricing a priced entry");

            assert!(
                (live_cost - repriced).abs() < 1e-12,
                "{provider}: live {live_cost} vs repriced {repriced}"
            );
        }
    }

    #[test]
    fn observed_mix_derives_fresh_input_from_the_total_prompt_convention() {
        // Stored rows carry the whole prompt in `input_tokens` with
        // `cached_input_tokens` as a labelled subset of it.
        let mix = ObservedMix::from_rows([&row("openai/a", 10_000, 2_000, 500, 3)]);
        assert_eq!(mix.fresh_input_tokens, 8_000);
        assert_eq!(mix.cached_input_tokens, 2_000);
        assert_eq!(mix.output_tokens, 500);
        assert_eq!(mix.calls, 3);
    }

    /// A cached count that somehow exceeds the total must not wrap a u64.
    #[test]
    fn observed_mix_saturates_rather_than_underflowing() {
        let mix = ObservedMix::from_rows([&row("openai/a", 100, 500, 10, 1)]);
        assert_eq!(mix.fresh_input_tokens, 0);
    }

    #[test]
    fn capability_gate_rejects_a_model_that_cannot_do_the_job() {
        let mut entry = priced_entry();
        entry.tool_call = false;
        let reqs = CapabilityRequirements {
            needs_tool_calling: true,
            ..Default::default()
        };
        let blockers = reqs.blockers(&entry);
        assert_eq!(blockers.len(), 1);
        assert!(blockers[0].contains("tool calling"), "{blockers:?}");
    }

    #[test]
    fn capability_gate_rejects_too_small_a_context_window() {
        let entry = priced_entry(); // 200k context, 32k output -> 168k input
        let reqs = CapabilityRequirements {
            min_context_tokens: Some(1_000_000),
            ..Default::default()
        };
        let blockers = reqs.blockers(&entry);
        assert_eq!(blockers.len(), 1);
        assert!(blockers[0].contains("below the 1000000"), "{blockers:?}");
    }

    #[test]
    fn capability_gate_rejects_a_deprecated_model() {
        let mut entry = priced_entry();
        entry.status = Some("deprecated".to_string());
        let blockers = CapabilityRequirements::default().blockers(&entry);
        assert_eq!(blockers, vec!["marked deprecated upstream".to_string()]);
    }

    #[test]
    fn capability_gate_passes_a_capable_model() {
        let reqs = CapabilityRequirements {
            needs_tool_calling: true,
            needs_vision: true,
            needs_reasoning: true,
            needs_structured_output: true,
            min_context_tokens: Some(100_000),
        };
        assert!(reqs.blockers(&priced_entry()).is_empty());
    }

    // ---- Billing ---------------------------------------------------------

    #[test]
    fn a_legacy_row_with_no_billing_kind_reads_as_metered() {
        // Every provider was metered before the column existed; treating an
        // empty string as anything else would silently reclassify history.
        assert_eq!(
            ProviderBillingKind::from_str_or_metered(""),
            ProviderBillingKind::Metered
        );
        assert_eq!(
            ProviderBillingKind::from_str_or_metered("subscription"),
            ProviderBillingKind::Subscription
        );
        assert_eq!(
            ProviderBillingKind::from_str_or_metered("self_hosted"),
            ProviderBillingKind::SelfHosted
        );
    }

    #[test]
    fn local_runtimes_default_to_self_hosted_and_everything_else_to_metered() {
        assert_eq!(
            ProviderBillingKind::default_for_provider("ollama"),
            ProviderBillingKind::SelfHosted
        );
        assert_eq!(
            ProviderBillingKind::default_for_provider("llamafile"),
            ProviderBillingKind::SelfHosted
        );
        assert_eq!(
            ProviderBillingKind::default_for_provider("anthropic"),
            ProviderBillingKind::Metered
        );
    }

    #[test]
    fn an_unconfigured_provider_keeps_pre_existing_metered_behaviour() {
        let cfg = ModelProviderConfig::default();
        assert_eq!(
            cfg.effective_billing("openai").kind,
            ProviderBillingKind::Metered
        );
        assert_eq!(
            cfg.effective_billing("ollama").kind,
            ProviderBillingKind::SelfHosted
        );
    }

    #[test]
    fn only_a_subscription_pro_rates_a_fee() {
        let sub = ProviderBilling {
            kind: ProviderBillingKind::Subscription,
            monthly_cost: Some(30.0),
            ..Default::default()
        };
        assert!((sub.prorated_cost(30.0) - 30.0).abs() < 1e-9);
        assert!((sub.prorated_cost(15.0) - 15.0).abs() < 1e-9);

        let metered = ProviderBilling {
            kind: ProviderBillingKind::Metered,
            monthly_cost: Some(30.0),
            ..Default::default()
        };
        assert_eq!(metered.prorated_cost(30.0), 0.0);
    }

    #[test]
    fn allowance_is_reported_only_for_a_subscription_that_declares_one() {
        let rollup = UsageRollup {
            input_tokens: 400_000,
            output_tokens: 100_000,
            cost_usd: 12.0,
            ..Default::default()
        };

        // Metered: no allowance concept at all.
        let metered = ProviderBilling {
            kind: ProviderBillingKind::Metered,
            included_spend_usd: Some(20.0),
            ..Default::default()
        };
        assert!(allowance_status(&metered, &rollup).is_none());

        // Subscription with no declared allowance: nothing to report, and
        // that is normal rather than an error.
        let bare = ProviderBilling {
            kind: ProviderBillingKind::Subscription,
            monthly_cost: Some(20.0),
            ..Default::default()
        };
        assert!(allowance_status(&bare, &rollup).is_none());

        // Spend allowance: 12 of 20 consumed.
        let with_credit = ProviderBilling {
            kind: ProviderBillingKind::Subscription,
            monthly_cost: Some(20.0),
            included_spend_usd: Some(20.0),
            ..Default::default()
        };
        let status = allowance_status(&with_credit, &rollup).expect("declared allowance");
        assert!((status.used_pct - 60.0).abs() < 1e-9);
        assert!(!status.exceeded);
    }

    /// With both allowances declared, the tighter one binds — it is the one
    /// that actually runs out first.
    #[test]
    fn the_tighter_of_two_allowances_decides_consumption() {
        let rollup = UsageRollup {
            input_tokens: 400_000,
            output_tokens: 100_000, // 500k tokens
            cost_usd: 5.0,
            ..Default::default()
        };
        let billing = ProviderBilling {
            kind: ProviderBillingKind::Subscription,
            included_spend_usd: Some(20.0),   // 25% consumed
            included_tokens: Some(1_000_000), // 50% consumed
            ..Default::default()
        };
        let status = allowance_status(&billing, &rollup).expect("declared allowance");
        assert!((status.used_pct - 50.0).abs() < 1e-9, "{}", status.used_pct);
        assert!(!status.exceeded);
    }

    #[test]
    fn exceeding_an_allowance_is_flagged() {
        let rollup = UsageRollup {
            cost_usd: 45.0,
            ..Default::default()
        };
        let billing = ProviderBilling {
            kind: ProviderBillingKind::Subscription,
            included_spend_usd: Some(20.0),
            overage_is_metered: true,
            ..Default::default()
        };
        let status = allowance_status(&billing, &rollup).expect("declared allowance");
        assert!(status.exceeded);
        assert!(status.overage_is_metered);
    }

    #[test]
    fn split_model_ref_keeps_a_vendor_prefixed_model_id_intact() {
        // OpenRouter-style ids carry their own vendor prefix; only the first
        // separator is the provider boundary.
        assert_eq!(
            split_model_ref("openrouter/anthropic/claude-sonnet-4-5"),
            Some(("openrouter", "anthropic/claude-sonnet-4-5"))
        );
        assert_eq!(split_model_ref("no-slash"), None);
        assert_eq!(split_model_ref("/leading"), None);
        assert_eq!(split_model_ref("trailing/"), None);
    }

    #[test]
    fn monthly_scale_projects_a_window_onto_a_nominal_period() {
        assert!((monthly_scale(30.0) - 1.0).abs() < 1e-9);
        assert!((monthly_scale(7.0) - 30.0 / 7.0).abs() < 1e-9);
        // A degenerate window must not produce infinity.
        assert_eq!(monthly_scale(0.0), 0.0);
    }
}
