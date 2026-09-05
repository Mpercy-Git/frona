//! Cost-analysis tools for the `cost-analyst` agent.
//!
//! # Two gates, and why both are needed
//!
//! Cedar's `invoke_tool` takes an **Agent** principal, so the policy in
//! `resources/policy/frona.cedar` can only say *which agent* may reach this
//! tool group — not whether the human driving it is allowed to see the whole
//! server's spend. Built-in agents are cloned per user, so agent identity
//! alone is not an authorization answer.
//!
//! Hence every entry point below re-checks `PolicyAction::ViewUsageAnalytics`
//! against the **runner** (`ctx.user`), following the same belt-and-braces
//! shape the memory tools use in `tool/sandbox/mod.rs`. The runner rather than
//! the owner: this agent can be shared, and analytics access is the privilege
//! of whoever is driving, not of whoever created the agent. An admin demoted
//! after the agent was cloned is refused here.

use serde_json::{Value, json};

use crate::agent::prompt::PromptLoader;
use crate::core::config::ProviderBillingKind;
use crate::core::error::AppError;
use crate::cost::models::{Confidence, CostRecommendation, RecommendationKind};
use crate::cost::service::{CapabilityRequirements, CostService, ObservedMix, monthly_scale};
use crate::policy::models::PolicyAction;
use crate::policy::service::PolicyService;
use frona_derive::agent_tool;

use super::{InferenceContext, ToolOutput};

/// Default analysis window. A month lines up with how subscriptions renew,
/// which is the comparison the whole feature exists to make.
const DEFAULT_WINDOW_DAYS: u32 = 30;
/// Cap on the window so an unbounded request can't scan the entire table.
const MAX_WINDOW_DAYS: u32 = 365;

pub struct CostAnalysisTool {
    cost_service: CostService,
    policy_service: PolicyService,
    prompts: PromptLoader,
}

impl CostAnalysisTool {
    pub fn new(
        cost_service: CostService,
        policy_service: PolicyService,
        prompts: PromptLoader,
    ) -> Self {
        Self {
            cost_service,
            policy_service,
            prompts,
        }
    }

    /// The gate described in the module docs. Every tool arm calls this first.
    async fn require_analytics(&self, ctx: &InferenceContext) -> Result<(), AppError> {
        let decision = self
            .policy_service
            .authorize_user(&ctx.user, PolicyAction::ViewUsageAnalytics)
            .await?;
        if decision.allowed {
            return Ok(());
        }
        Err(AppError::Forbidden(
            "Reading instance-wide usage and cost requires the view_usage_analytics permission, \
             which is granted to the 'admins' group by default. This agent can only report on \
             spend for an operator who holds it."
                .into(),
        ))
    }

    fn window_days(arguments: &Value) -> u32 {
        arguments
            .get("window_days")
            .and_then(Value::as_u64)
            .map(|d| (d as u32).clamp(1, MAX_WINDOW_DAYS))
            .unwrap_or(DEFAULT_WINDOW_DAYS)
    }
}

#[agent_tool(
    name = "cost_analysis",
    files(
        "analyse_spend",
        "compare_models",
        "list_provider_billing",
        "save_cost_report"
    )
)]
impl CostAnalysisTool {
    async fn execute(
        &self,
        tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        self.require_analytics(ctx).await?;

        match tool_name {
            "analyse_spend" => self.analyse_spend(arguments).await,
            "compare_models" => self.compare_models(arguments).await,
            "list_provider_billing" => self.list_provider_billing(),
            "save_cost_report" => self.save_cost_report(arguments, ctx).await,
            other => Err(AppError::Validation(format!("Unknown tool: {other}"))),
        }
    }
}

impl CostAnalysisTool {
    async fn analyse_spend(&self, arguments: Value) -> Result<ToolOutput, AppError> {
        let days = Self::window_days(&arguments);
        let (since, until) = crate::cost::window_from_days(days);
        let analysis = self.cost_service.analyse(since, until).await?;

        let group_by = arguments
            .get("group_by")
            .and_then(Value::as_str)
            .unwrap_or("model");

        let breakdown = match group_by {
            "provider" => json!(analysis.providers),
            "model_group" => json!(analysis.by_model_group),
            "kind" => json!(analysis.by_kind),
            "user" => json!(analysis.top_users),
            _ => json!(analysis.models),
        };

        let mut out = json!({
            "window": {
                "since": since.to_rfc3339(),
                "until": until.to_rfc3339(),
                "days": days,
            },
            "totals": analysis.totals,
            "metered_cost_usd": analysis.metered_cost_usd,
            "subscription_cost_usd": analysis.subscription_cost_usd,
            "subscription_list_value_usd": analysis.subscription_list_value_usd,
            "self_hosted_list_value_usd": analysis.self_hosted_list_value_usd,
            "uncosted_calls": analysis.uncosted_calls,
            "pricing_version": analysis.pricing_version,
            // Providers are always included: no figure above can be read
            // correctly without knowing which of them is metered.
            "providers": analysis.providers,
            "group_by": group_by,
            "breakdown": breakdown,
            "models": analysis.models,
        });

        if analysis.uncosted_calls > 0 {
            out["note"] = json!(format!(
                "{} call(s) in this window have no catalogue price, so every cost figure here \
                 understates actual spend by an unknown amount. Report this rather than treating \
                 those calls as free.",
                analysis.uncosted_calls
            ));
        }
        if analysis.totals.calls == 0 {
            out["note"] = json!(
                "No inference was recorded in this window. Widen window_days before concluding \
                 anything about spend."
            );
        }

        Ok(ToolOutput::text(out.to_string()))
    }

    async fn compare_models(&self, arguments: Value) -> Result<ToolOutput, AppError> {
        let candidates: Vec<String> = arguments
            .get("candidates")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if candidates.is_empty() {
            return Err(AppError::Validation(
                "candidates must be a non-empty array of 'provider/model' references".into(),
            ));
        }

        let days = Self::window_days(&arguments);
        let (since, until) = crate::cost::window_from_days(days);
        let analysis = self.cost_service.analyse(since, until).await?;

        let baseline = arguments.get("baseline_model_ref").and_then(Value::as_str);
        let model_group = arguments.get("model_group").and_then(Value::as_str);

        // Reprice the traffic the baseline model actually served. Falling back
        // to the whole window when no baseline matches would silently value a
        // candidate against a mix it would never see, so an empty match is
        // reported as such instead.
        let rows: Vec<_> = analysis
            .models
            .iter()
            .filter(|m| baseline.is_none_or(|b| m.model_ref == b))
            .filter(|m| model_group.is_none_or(|g| m.model_group == g))
            .collect();
        let observed = ObservedMix::from_rows(rows.iter().copied());

        if observed.calls == 0 {
            return Ok(ToolOutput::text(
                json!({
                    "error": "no recorded traffic matches that baseline_model_ref / model_group in this window",
                    "hint": "call analyse_spend first and use a model_ref exactly as it appears in its `models` list",
                    "window_days": days,
                })
                .to_string(),
            ));
        }

        let requirements = CapabilityRequirements {
            needs_tool_calling: flag(&arguments, "needs_tool_calling"),
            needs_vision: flag(&arguments, "needs_vision"),
            needs_reasoning: flag(&arguments, "needs_reasoning"),
            needs_structured_output: flag(&arguments, "needs_structured_output"),
            min_context_tokens: arguments.get("min_context_tokens").and_then(Value::as_u64),
        };

        let set = self
            .cost_service
            .compare_models(observed, baseline, &candidates, &requirements);

        let scale = monthly_scale(analysis.window_days);
        let monthly = json!({
            "scale_from_window": scale,
            "baseline_monthly_usd": set.baseline_cost_usd.map(|c| c * scale),
        });

        Ok(ToolOutput::text(
            json!({
                "window_days": days,
                "observed_mix": set.observed,
                "baseline_model_ref": set.baseline_model_ref,
                "baseline_cost_usd": set.baseline_cost_usd,
                "monthly": monthly,
                "pricing_version": set.pricing_version,
                "comparisons": set.comparisons,
                "note": "Costs are for the observed window. Multiply by `monthly.scale_from_window` \
                         to quote a monthly figure. A candidate with a non-empty `rejected_because` \
                         must not be recommended.",
            })
            .to_string(),
        ))
    }

    fn list_provider_billing(&self) -> Result<ToolOutput, AppError> {
        let rows: Vec<Value> = self
            .cost_service
            .provider_billing()
            .into_iter()
            .map(|(name, billing, enabled)| {
                json!({
                    "provider": name,
                    "enabled": enabled,
                    "kind": billing.kind.as_str(),
                    "monthly_cost": billing.monthly_cost,
                    "currency": billing.currency_or_usd(),
                    "included_tokens": billing.included_tokens,
                    "included_spend_usd": billing.included_spend_usd,
                    "overage_is_metered": billing.overage_is_metered,
                    "renewal_day": billing.renewal_day,
                    "notes": billing.notes,
                    "explicitly_configured": matches!(
                        billing.kind,
                        ProviderBillingKind::Subscription
                    ) || billing.monthly_cost.is_some(),
                })
            })
            .collect();

        Ok(ToolOutput::text(
            json!({
                "providers": rows,
                "note": "A provider with no billing block is reported at its default: local \
                         runtimes as self_hosted, everything else as metered. If a provider here \
                         is actually on a subscription, the operator has not told the server, and \
                         its recorded cost will be read as money spent when it is not.",
            })
            .to_string(),
        ))
    }

    async fn save_cost_report(
        &self,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let summary = arguments
            .get("summary")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                AppError::Validation("summary is required and must not be empty".into())
            })?
            .to_string();

        let recommendations = parse_recommendations(&arguments)?;

        let days = Self::window_days(&arguments);
        let (since, until) = crate::cost::window_from_days(days);
        // Recompute rather than trusting figures echoed back through the model:
        // the stored report's totals are then always the database's answer.
        let analysis = self.cost_service.analyse(since, until).await?;

        let report = self
            .cost_service
            .save_report(&ctx.user.id, &analysis, summary, recommendations)
            .await?;

        Ok(ToolOutput::text(
            json!({
                "message": format!(
                    "Cost report saved with {} recommendation(s). It is visible to admins under \
                     Settings → Costs.",
                    report.recommendations.len()
                ),
                "report_id": report.id,
                "window_since": report.window_since.to_rfc3339(),
                "window_until": report.window_until.to_rfc3339(),
                "metered_cost_usd": report.metered_cost_usd,
                "estimated_monthly_savings_usd": report.estimated_monthly_savings_usd,
                "created_at": report.created_at.to_rfc3339(),
            })
            .to_string(),
        ))
    }
}

fn flag(arguments: &Value, key: &str) -> bool {
    arguments.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn parse_recommendations(arguments: &Value) -> Result<Vec<CostRecommendation>, AppError> {
    let items = arguments
        .get("recommendations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::Validation(
                "recommendations is required; pass an empty array when the setup is already sound"
                    .into(),
            )
        })?;

    items
        .iter()
        .map(|item| {
            let kind = item
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::Validation("each recommendation needs a kind".into()))?;
            let rationale = item
                .get("rationale")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| {
                    AppError::Validation("each recommendation needs a non-empty rationale".into())
                })?;
            let confidence = item
                .get("confidence")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AppError::Validation("each recommendation needs a confidence".into())
                })?;

            Ok(CostRecommendation {
                kind: parse_kind(kind)?,
                model_group: str_field(item, "model_group"),
                from: str_field(item, "from"),
                to: str_field(item, "to"),
                rationale: rationale.to_string(),
                estimated_monthly_delta_usd: item
                    .get("estimated_monthly_delta_usd")
                    .and_then(Value::as_f64),
                confidence: parse_confidence(confidence)?,
            })
        })
        .collect()
}

fn str_field(item: &Value, key: &str) -> Option<String> {
    item.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

fn parse_kind(s: &str) -> Result<RecommendationKind, AppError> {
    Ok(match s {
        "switch_model" => RecommendationKind::SwitchModel,
        "rebalance_provider" => RecommendationKind::RebalanceProvider,
        "enable_caching" => RecommendationKind::EnableCaching,
        "subscription_underused" => RecommendationKind::SubscriptionUnderused,
        "subscription_overrun" => RecommendationKind::SubscriptionOverrun,
        "pricing_gap" => RecommendationKind::PricingGap,
        other => {
            return Err(AppError::Validation(format!(
                "unknown recommendation kind '{other}'; expected one of switch_model, \
                 rebalance_provider, enable_caching, subscription_underused, \
                 subscription_overrun, pricing_gap"
            )));
        }
    })
}

fn parse_confidence(s: &str) -> Result<Confidence, AppError> {
    Ok(match s {
        "high" => Confidence::High,
        "medium" => Confidence::Medium,
        "low" => Confidence::Low,
        other => {
            return Err(AppError::Validation(format!(
                "unknown confidence '{other}'; expected high, medium or low"
            )));
        }
    })
}
