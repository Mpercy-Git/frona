//! Migrate `Agent.sandbox_config` (legacy `SandboxSettings` shape) to:
//!   - Cedar policies in the per-group canonical shape produced by the
//!     reconcile flow (per-resource simple, wildcard, or carveout)
//!   - `Agent.sandbox_limits` (current `SandboxLimits` shape)
//!     and `UNSET` the old `sandbox_config` field on the row.
//!
//! Operates on raw SurrealDB rows, not the typed `Agent` struct, since the
//! struct no longer carries `sandbox_config`.

use surrealdb::Surreal;
use surrealdb::engine::local::Db;

use frona_derive::migration;

use crate::policy::reconcile::{
    AccessGroup, AccessIntent, AccessOverride, CanonicalShape, EntityRef, build_carveout_text,
    build_per_resource_text, build_wildcard_text, canonical_shape, group_policy_name,
    per_resource_policy_name,
};

#[migration("2026-04-28T00:00:00Z")]
async fn migrate_agent_sandbox_config_to_policies(db: &Surreal<Db>) -> Result<(), surrealdb::Error> {
    let mut result = db
        .query("SELECT meta::id(id) as id, user_id, sandbox_config FROM agent WHERE sandbox_config IS NOT NONE")
        .await?;

    let rows: Vec<serde_json::Value> = result.take(0)?;

    for row in rows {
        let Some(agent_id) = row.get("id").and_then(|v| v.as_str()).map(str::to_string) else {
            continue;
        };
        let Some(user_id) = row.get("user_id").and_then(|v| v.as_str()).map(str::to_string) else {
            // Shared/built-in agents have no user_id; sandbox config doesn't apply to them.
            continue;
        };
        let Some(cfg) = row.get("sandbox_config") else {
            continue;
        };

        let network_access = cfg
            .get("network_access")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let network_destinations: Vec<String> = cfg
            .get("allowed_network_destinations")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let shared_paths: Vec<String> = cfg
            .get("shared_paths")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let principal = EntityRef::Agent(agent_id.clone());
        let groups = legacy_to_groups(&principal, &shared_paths, network_access, &network_destinations);

        for group in &groups {
            for (name, text) in canonical_emissions_for_group(group) {
                // Idempotent: skip if a policy with this name already exists for the user.
                let existing: Option<serde_json::Value> = db
                    .query("SELECT VALUE id FROM policy WHERE user_id = $user_id AND name = $name LIMIT 1")
                    .bind(("user_id", user_id.clone()))
                    .bind(("name", name.clone()))
                    .await?
                    .take(0)?;
                if existing.is_some() {
                    continue;
                }

                let now = chrono::Utc::now();
                db.query(
                    "CREATE policy CONTENT {
                        id: $id,
                        user_id: $user_id,
                        name: $name,
                        description: '',
                        policy_text: $policy_text,
                        enabled: true,
                        created_at: $now,
                        updated_at: $now,
                    }",
                )
                .bind(("id", crate::core::repository::new_id()))
                .bind(("user_id", user_id.clone()))
                .bind(("name", name))
                .bind(("policy_text", text))
                .bind(("now", now))
                .await?;
            }
        }

        let timeout_secs = cfg
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let max_cpu_pct = cfg
            .get("max_cpu_pct")
            .and_then(|v| v.as_f64())
            .unwrap_or(95.0);
        let max_memory_pct = cfg
            .get("max_memory_pct")
            .and_then(|v| v.as_f64())
            .unwrap_or(80.0);
        let any_quota_set = cfg.get("timeout_secs").is_some_and(|v| !v.is_null())
            || cfg.get("max_cpu_pct").is_some_and(|v| !v.is_null())
            || cfg.get("max_memory_pct").is_some_and(|v| !v.is_null());

        if any_quota_set {
            db.query(
                "UPDATE agent SET sandbox_limits = {
                    timeout_secs: $timeout_secs,
                    max_cpu_pct: $max_cpu_pct,
                    max_memory_pct: $max_memory_pct
                }, sandbox_config = NONE WHERE meta::id(id) = $id",
            )
            .bind(("timeout_secs", timeout_secs))
            .bind(("max_cpu_pct", max_cpu_pct))
            .bind(("max_memory_pct", max_memory_pct))
            .bind(("id", agent_id.clone()))
            .await?;
        } else {
            db.query("UPDATE agent SET sandbox_config = NONE WHERE meta::id(id) = $id")
                .bind(("id", agent_id.clone()))
                .await?;
        }

        tracing::info!(agent_id, user_id, "Migrated Agent.sandbox_config to reconciled policies + sandbox_limits");
    }

    Ok(())
}

/// Translate the legacy field set into the new `AccessGroup`s. Mirrors
/// `policy::service::sandbox_policy_to_groups` for the subset of fields the
/// legacy `SandboxSettings` struct carried (`shared_paths`,
/// `network_access`, `allowed_network_destinations`).
fn legacy_to_groups(
    principal: &EntityRef,
    shared_paths: &[String],
    network_access: bool,
    network_destinations: &[String],
) -> Vec<AccessGroup> {
    let path_overrides = |action: &str| AccessGroup {
        principal: principal.clone(),
        action: action.into(),
        default: None,
        overrides: shared_paths
            .iter()
            .map(|p| AccessOverride {
                resource: EntityRef::Path(p.clone()),
                intent: AccessIntent::Allow,
            })
            .collect(),
    };

    let connect_default = if network_access { None } else { Some(AccessIntent::Deny) };
    let connect_overrides: Vec<AccessOverride> = if network_access {
        network_destinations
            .iter()
            .map(|d| AccessOverride {
                resource: EntityRef::NetworkDestination(d.clone()),
                intent: AccessIntent::Allow,
            })
            .collect()
    } else {
        Vec::new()
    };

    vec![
        path_overrides("read"),
        path_overrides("write"),
        AccessGroup {
            principal: principal.clone(),
            action: "connect".into(),
            default: connect_default,
            overrides: connect_overrides,
        },
    ]
}

/// Compute the canonical emissions for a group: a list of `(name, text)`
/// pairs the migration should insert. Mirrors the planner's emission logic
/// without consulting the live policy set (the migration runs once on legacy
/// data; there's nothing to diff against).
fn canonical_emissions_for_group(group: &AccessGroup) -> Vec<(String, String)> {
    match canonical_shape(group) {
        CanonicalShape::PerOverride => group
            .overrides
            .iter()
            .filter_map(|ov| match ov.intent {
                AccessIntent::Allow => Some((cedar_policy::Effect::Permit, &ov.resource)),
                AccessIntent::Deny => Some((cedar_policy::Effect::Forbid, &ov.resource)),
                AccessIntent::Default => None,
            })
            .map(|(effect, resource)| {
                let name =
                    per_resource_policy_name(&group.principal, &group.action, resource, effect);
                let text = build_per_resource_text(
                    &name,
                    &group.principal,
                    &group.action,
                    resource,
                    effect,
                );
                (name, text)
            })
            .collect(),
        CanonicalShape::Wildcard { effect } => {
            let name = group_policy_name(&group.principal, &group.action);
            let text = build_wildcard_text(&name, &group.principal, &group.action, effect);
            vec![(name, text)]
        }
        CanonicalShape::Carveout {
            effect,
            carveout_resources,
        } => {
            let name = group_policy_name(&group.principal, &group.action);
            let text = build_carveout_text(
                &name,
                &group.principal,
                &group.action,
                effect,
                &carveout_resources,
            );
            vec![(name, text)]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str) -> EntityRef {
        EntityRef::Agent(id.into())
    }

    #[test]
    fn legacy_with_shared_paths_emits_per_resource_permits() {
        let groups = legacy_to_groups(
            &agent("a"),
            &["/data".into(), "/work".into()],
            true,
            &[],
        );
        let read_group = groups.iter().find(|g| g.action == "read").unwrap();
        let emissions = canonical_emissions_for_group(read_group);
        assert_eq!(emissions.len(), 2);
        assert!(emissions.iter().all(|(_, text)| text.contains("permit")));
    }

    #[test]
    fn legacy_network_off_emits_wildcard_forbid_for_connect() {
        let groups = legacy_to_groups(&agent("a"), &[], false, &[]);
        let connect_group = groups.iter().find(|g| g.action == "connect").unwrap();
        let emissions = canonical_emissions_for_group(connect_group);
        assert_eq!(emissions.len(), 1);
        assert!(emissions[0].1.contains("forbid"));
    }

    #[test]
    fn legacy_network_on_with_destinations_emits_no_connect_emission() {
        // network_access=true with destinations → default=None so the
        // canonical shape is PerOverride with only Allow overrides. With no
        // baseline to consult, the migration emits per-resource permits.
        let groups = legacy_to_groups(&agent("a"), &[], true, &["gmail.com".into()]);
        let connect_group = groups.iter().find(|g| g.action == "connect").unwrap();
        let emissions = canonical_emissions_for_group(connect_group);
        assert_eq!(emissions.len(), 1);
        assert!(emissions[0].1.contains("permit"));
        assert!(emissions[0].1.contains("gmail.com"));
    }

    #[test]
    fn legacy_no_inputs_emits_nothing_for_paths() {
        let groups = legacy_to_groups(&agent("a"), &[], true, &[]);
        for group in &groups {
            let emissions = canonical_emissions_for_group(group);
            assert!(emissions.is_empty(), "expected no emissions for empty inputs, got {emissions:?}");
        }
    }
}
