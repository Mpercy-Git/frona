//! Instance-wide cost aggregation.
//!
//! Every other rollup in `InferenceUsageRepository` is scoped to one user;
//! these span the whole server, so the thing worth pinning is that they really
//! do cross user boundaries, and that the billing split keeps money actually
//! spent separate from list-price value covered by a fee.

mod helpers;

use chrono::{Duration, Utc};
use frona::core::repository::Repository;
use frona::db::repo::generic::SurrealRepo;
use frona::inference::usage::{InferenceUsage, InferenceUsageRepository, TimeBucket};
use surrealdb::Surreal;
use surrealdb::engine::local::{Db, Mem};

async fn fresh_db() -> Surreal<Db> {
    let db = Surreal::new::<Mem>(()).await.expect("test db");
    frona::db::init::setup_schema(&db).await.expect("schema");
    db
}

#[allow(clippy::too_many_arguments)]
fn usage_row(
    user_id: &str,
    provider: &str,
    model_id: &str,
    model_group: &str,
    billing_kind: &str,
    input: u64,
    cached: u64,
    output: u64,
    cost_usd: Option<f64>,
    age_days: i64,
) -> InferenceUsage {
    InferenceUsage {
        id: frona::core::repository::new_id(),
        user_id: user_id.to_string(),
        agent_id: Some("agent".into()),
        chat_id: Some("chat".into()),
        space_id: None,
        message_id: Some("msg".into()),
        turn_index: None,
        kind_tag: "Text".into(),
        model_group: model_group.to_string(),
        provider: provider.to_string(),
        model_id: model_id.to_string(),
        model_ref: format!("{provider}/{model_id}"),
        input_tokens: input,
        cached_input_tokens: cached,
        output_tokens: output,
        total_tokens: input + output,
        fallback_index: 0,
        duration_ms: 1_000,
        ttft_ms: Some(200),
        output_tokens_per_second: None,
        retry_overhead_ms: 0,
        retry_count: 0,
        cost_usd,
        pricing_version: "test".into(),
        billing_kind: billing_kind.to_string(),
        created_at: Utc::now() - Duration::days(age_days),
    }
}

/// Two users, three providers, three billing regimes, one row with no price.
async fn seeded_db() -> Surreal<Db> {
    let db = fresh_db().await;
    let repo: SurrealRepo<InferenceUsage> = SurrealRepo::new(db.clone());
    for row in [
        // alice, metered
        usage_row(
            "alice",
            "openai",
            "gpt-5",
            "primary",
            "metered",
            10_000,
            2_000,
            1_000,
            Some(6.0),
            1,
        ),
        usage_row(
            "alice",
            "openai",
            "gpt-5",
            "primary",
            "metered",
            5_000,
            0,
            500,
            Some(4.0),
            2,
        ),
        // bob, metered, different group
        usage_row(
            "bob",
            "openai",
            "gpt-5",
            "reasoning",
            "metered",
            20_000,
            0,
            2_000,
            Some(2.0),
            1,
        ),
        // bob, on a subscription — list-price value, not money spent
        usage_row(
            "bob",
            "anthropic",
            "claude-opus-5",
            "reasoning",
            "subscription",
            8_000,
            4_000,
            900,
            Some(18.0),
            1,
        ),
        // alice, self-hosted — never billed at all
        usage_row(
            "alice",
            "ollama",
            "llama-4",
            "cheap",
            "self_hosted",
            3_000,
            0,
            300,
            Some(1.5),
            1,
        ),
        // a model the catalogue has no price for: unmeasured, not free
        usage_row(
            "bob", "generic", "mystery", "cheap", "metered", 1_000, 0, 100, None, 1,
        ),
        // outside a 7-day window
        usage_row(
            "alice",
            "openai",
            "gpt-5",
            "primary",
            "metered",
            999_999,
            0,
            999,
            Some(500.0),
            40,
        ),
    ] {
        repo.create(&row).await.expect("seed row");
    }
    db
}

fn window() -> (Option<chrono::DateTime<Utc>>, Option<chrono::DateTime<Utc>>) {
    let now = Utc::now();
    (Some(now - Duration::days(7)), Some(now + Duration::days(1)))
}

#[tokio::test]
async fn aggregate_all_spans_every_user() {
    let db = seeded_db().await;
    let repo: SurrealRepo<InferenceUsage> = SurrealRepo::new(db.clone());
    let (since, until) = window();

    let all = repo.aggregate_all(since, until).await.unwrap();
    // Six rows inside the window; the 40-day-old one is excluded.
    assert_eq!(all.calls, 6);
    assert!((all.cost_usd - 31.5).abs() < 1e-9, "{}", all.cost_usd);

    // The per-user rollup must still see only its own slice, or the instance
    // query has leaked into the user-facing dashboards.
    let alice = repo.aggregate_by_user("alice", since, until).await.unwrap();
    assert_eq!(alice.calls, 3);
    assert!(alice.cost_usd < all.cost_usd);
}

#[tokio::test]
async fn provider_rollup_separates_billing_regimes() {
    let db = seeded_db().await;
    let repo: SurrealRepo<InferenceUsage> = SurrealRepo::new(db.clone());
    let (since, until) = window();

    let rows = repo.aggregate_by_provider_all(since, until).await.unwrap();

    let openai = rows
        .iter()
        .find(|r| r.provider == "openai" && r.billing_kind == "metered")
        .expect("openai metered");
    assert_eq!(openai.calls, 3);
    assert!((openai.cost_usd - 12.0).abs() < 1e-9);

    let anthropic = rows
        .iter()
        .find(|r| r.provider == "anthropic")
        .expect("anthropic");
    assert_eq!(anthropic.billing_kind, "subscription");

    let ollama = rows
        .iter()
        .find(|r| r.provider == "ollama")
        .expect("ollama");
    assert_eq!(ollama.billing_kind, "self_hosted");
}

#[tokio::test]
async fn model_rollup_carries_the_repricing_inputs_and_flags_pricing_gaps() {
    let db = seeded_db().await;
    let repo: SurrealRepo<InferenceUsage> = SurrealRepo::new(db.clone());
    let (since, until) = window();

    let rows = repo.aggregate_by_model_all(since, until).await.unwrap();

    // Grouped by model group as well as model, because a recommendation acts
    // on a group.
    let primary = rows
        .iter()
        .find(|r| r.model_ref == "openai/gpt-5" && r.model_group == "primary")
        .expect("gpt-5 in primary");
    assert_eq!(primary.calls, 2);
    assert_eq!(primary.input_tokens, 15_000);
    assert_eq!(primary.cached_input_tokens, 2_000);
    assert_eq!(primary.output_tokens, 1_500);
    assert!(primary.duration_ms_mean.is_some());

    assert!(
        rows.iter()
            .any(|r| r.model_ref == "openai/gpt-5" && r.model_group == "reasoning"),
        "the same model in another group must be its own row"
    );

    // A call the catalogue could not price sums to 0 cost, so it must be
    // counted separately or it reads as free.
    let mystery = rows
        .iter()
        .find(|r| r.model_ref == "generic/mystery")
        .expect("unpriced model");
    assert_eq!(mystery.uncosted_calls, 1);
    assert_eq!(mystery.cost_usd, 0.0);
}

#[tokio::test]
async fn model_group_and_kind_rollups_cover_every_user() {
    let db = seeded_db().await;
    let repo: SurrealRepo<InferenceUsage> = SurrealRepo::new(db.clone());
    let (since, until) = window();

    let by_group = repo
        .aggregate_by_model_group_all(since, until)
        .await
        .unwrap();
    // `reasoning` has one row from bob on each of two providers.
    assert_eq!(by_group["reasoning"].calls, 2);
    assert_eq!(by_group["primary"].calls, 2);
    assert_eq!(by_group["cheap"].calls, 2);

    let by_kind = repo.aggregate_by_kind_all(since, until).await.unwrap();
    assert_eq!(by_kind["Text"].calls, 6);
}

#[tokio::test]
async fn top_users_ranks_by_spend() {
    let db = seeded_db().await;
    let repo: SurrealRepo<InferenceUsage> = SurrealRepo::new(db.clone());
    let (since, until) = window();

    let rows = repo.top_users_by_cost(since, until, 10).await.unwrap();
    assert_eq!(rows.len(), 2);
    // bob: 2 + 18 = 20; alice: 6 + 4 + 1.5 = 11.5.
    assert_eq!(rows[0].user_id, "bob");
    assert!(rows[0].cost_usd >= rows[1].cost_usd);
}

#[tokio::test]
async fn instance_buckets_bin_by_day() {
    let db = seeded_db().await;
    let repo: SurrealRepo<InferenceUsage> = SurrealRepo::new(db.clone());
    let now = Utc::now();

    let buckets = repo
        .aggregate_buckets_all(
            now - Duration::days(7),
            now + Duration::days(1),
            TimeBucket::Day,
        )
        .await
        .unwrap();
    assert!(!buckets.is_empty());
    let total: u64 = buckets.iter().map(|b| b.calls).sum();
    assert_eq!(total, 6);
}

/// Rows written before `billing_kind` existed carry no such field at all.
/// They must aggregate as metered rather than being dropped by the query.
#[tokio::test]
async fn rows_predating_the_billing_column_still_aggregate() {
    let db = fresh_db().await;
    db.query(
        "CREATE inference_usage SET user_id = 'legacy', agent_id = 'a', chat_id = 'c', \
         message_id = 'm', kind_tag = 'Text', model_group = 'primary', provider = 'openai', \
         model_id = 'gpt-5', model_ref = 'openai/gpt-5', input_tokens = 100, \
         cached_input_tokens = 0, output_tokens = 10, total_tokens = 110, fallback_index = 0, \
         duration_ms = 500, retry_overhead_ms = 0, retry_count = 0, cost_usd = 1.25, \
         pricing_version = 'old', created_at = time::now()",
    )
    .await
    .expect("legacy row");

    let repo: SurrealRepo<InferenceUsage> = SurrealRepo::new(db.clone());
    let all = repo.aggregate_all(None, None).await.unwrap();
    assert_eq!(all.calls, 1);

    let providers = repo.aggregate_by_provider_all(None, None).await.unwrap();
    assert_eq!(providers.len(), 1);
    // Coalesced to "", which `ProviderBillingKind::from_str_or_metered` reads
    // as metered — the only classification that was true when it was written.
    assert_eq!(providers[0].billing_kind, "");
    assert_eq!(
        frona::core::config::ProviderBillingKind::from_str_or_metered(&providers[0].billing_kind),
        frona::core::config::ProviderBillingKind::Metered
    );
}

#[tokio::test]
async fn an_empty_window_returns_zeroes_rather_than_failing() {
    let db = seeded_db().await;
    let repo: SurrealRepo<InferenceUsage> = SurrealRepo::new(db.clone());
    let far_past = Utc::now() - Duration::days(3650);

    let all = repo
        .aggregate_all(Some(far_past), Some(far_past + Duration::days(1)))
        .await
        .unwrap();
    assert_eq!(all.calls, 0);
    assert_eq!(all.cost_usd, 0.0);
    assert!(
        repo.aggregate_by_provider_all(Some(far_past), Some(far_past + Duration::days(1)))
            .await
            .unwrap()
            .is_empty()
    );
}
