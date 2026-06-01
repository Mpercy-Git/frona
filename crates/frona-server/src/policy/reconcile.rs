use std::collections::HashSet;
use std::str::FromStr;

use cedar_policy::{
    ActionConstraint, Authorizer, Context, Decision, Effect, Entities, EntityUid, Policy,
    PolicyId, PolicySet, PrincipalConstraint, Request, ResourceConstraint, Schema,
};
use sha2::{Digest, Sha256};

use crate::core::error::AppError;

use super::models::Policy as StoredPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessIntent {
    Allow,
    Deny,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EntityRef {
    /// Cedar id `"{user_handle}/{agent_handle}"`. Construct via `Self::agent`.
    Agent(String),
    Mcp(String),
    App(String),
    User(String),
    Path(String),
    Tool(String),
    ToolGroup(String),
    NetworkDestination(String),
}

impl EntityRef {
    pub fn agent(user_handle: &crate::core::Handle, agent_handle: &crate::core::Handle) -> Self {
        Self::Agent(format!("{user_handle}/{agent_handle}"))
    }

    pub fn cedar_type(&self) -> &'static str {
        match self {
            Self::Agent(_) => "Policy::Agent",
            Self::Mcp(_) => "Policy::Mcp",
            Self::App(_) => "Policy::App",
            Self::User(_) => "Policy::User",
            Self::Path(_) => "Policy::Path",
            Self::Tool(_) => "Policy::Tool",
            Self::ToolGroup(_) => "Policy::ToolGroup",
            Self::NetworkDestination(_) => "Policy::NetworkDestination",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Agent(s)
            | Self::Mcp(s)
            | Self::App(s)
            | Self::User(s)
            | Self::Path(s)
            | Self::Tool(s)
            | Self::ToolGroup(s)
            | Self::NetworkDestination(s) => s,
        }
    }

    pub fn to_cedar_uid(&self) -> EntityUid {
        let s = format!("{}::\"{}\"", self.cedar_type(), self.id().replace('"', "\\\""));
        EntityUid::from_str(&s).expect("valid entity uid")
    }
}

#[derive(Debug, Clone)]
pub struct AccessOverride {
    pub resource: EntityRef,
    pub intent: AccessIntent,
}

#[derive(Debug, Clone)]
pub struct AccessGroup {
    pub principal: EntityRef,
    pub action: String,
    pub default: Option<AccessIntent>,
    pub overrides: Vec<AccessOverride>,
}

#[derive(Debug, Clone, Default)]
pub struct PolicyReconcileTarget {
    pub groups: Vec<AccessGroup>,
}

#[derive(Debug, Clone)]
pub enum Edit {
    Create { name: String, policy_text: String },
    Delete { policy_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictReason {
    AllowBlockedByForbid,
    DenyBlockedByPermit,
}

#[derive(Debug, Clone)]
pub struct BlockerPolicy {
    pub policy_id: String,
    pub policy_name: String,
}

#[derive(Debug, Clone)]
pub struct GroupConflict {
    pub principal: EntityRef,
    pub action: String,
    pub failing_resource: Option<EntityRef>,
    pub blockers: Vec<BlockerPolicy>,
    pub reason: ConflictReason,
}

#[derive(Debug, Clone)]
pub struct PolicyReconciliationPlan {
    pub user_id: String,
    pub fingerprint: u64,
    pub edits: Vec<Edit>,
    pub conflicts: Vec<GroupConflict>,
}

impl PolicyReconciliationPlan {
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }

    pub fn is_noop(&self) -> bool {
        self.edits.is_empty() && self.conflicts.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyReconciliationResult {
    pub created: usize,
    pub deleted: usize,
}

#[derive(Debug)]
pub enum PolicyReconciliationError {
    StalePlan,
    Conflicts(Vec<GroupConflict>),
    VerificationFailed {
        principal: EntityRef,
        action: String,
        resource: Option<EntityRef>,
        expected: AccessIntent,
        actual: AccessIntent,
    },
    Db(AppError),
}

impl std::fmt::Display for PolicyReconciliationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StalePlan => write!(f, "plan is stale; rerun reconcile"),
            Self::Conflicts(c) => write!(f, "{} group(s) have unresolvable conflicts", c.len()),
            Self::VerificationFailed { action, resource, expected, actual, .. } => write!(
                f,
                "post-commit verification failed for action {action} resource {resource:?}: expected {expected:?}, got {actual:?}"
            ),
            Self::Db(e) => write!(f, "db error: {e}"),
        }
    }
}

impl std::error::Error for PolicyReconciliationError {}

impl From<AppError> for PolicyReconciliationError {
    fn from(e: AppError) -> Self {
        Self::Db(e)
    }
}

impl From<PolicyReconciliationError> for AppError {
    fn from(e: PolicyReconciliationError) -> Self {
        match e {
            PolicyReconciliationError::Db(inner) => inner,
            other => AppError::Validation(other.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum CanonicalShape {
    /// `default == None` (or default matches baseline). Per-override emission;
    /// each override is its own row when needed.
    PerOverride,
    /// `default == Some` with no overrides → single wildcard policy.
    Wildcard { effect: Effect },
    /// `default == Some` with overrides whose intent != default → single
    /// `forbid/permit(... resource) unless { ... }` carveout policy.
    Carveout {
        effect: Effect,
        carveout_resources: Vec<EntityRef>,
    },
}

pub struct PlanCtx<'a> {
    pub live: &'a [&'a StoredPolicy],
    pub managed: &'a [Policy],
    pub schema: &'a Schema,
    pub entities: &'a Entities,
    pub hierarchy: &'a ResourceHierarchy,
}

/// Closed-world resource hierarchy used to collapse same-intent overrides into
/// a single ancestor rule (e.g., N tools → one ToolGroup rule). Currently
/// populated only for `Tool → ToolGroup`; generic shape so other ancestor
/// types can plug in later.
#[derive(Debug, Clone, Default)]
pub struct ResourceHierarchy {
    pub parent: std::collections::HashMap<EntityRef, EntityRef>,
    pub descendants: std::collections::HashMap<EntityRef, Vec<EntityRef>>,
}

impl ResourceHierarchy {
    pub fn add(&mut self, child: EntityRef, parent: EntityRef) {
        self.parent.insert(child.clone(), parent.clone());
        self.descendants.entry(parent).or_default().push(child);
    }

    pub fn parent_of(&self, child: &EntityRef) -> Option<&EntityRef> {
        self.parent.get(child)
    }

    pub fn descendants_of(&self, parent: &EntityRef) -> Option<&[EntityRef]> {
        self.descendants.get(parent).map(|v| v.as_slice())
    }
}

fn uses_in_constraint(r: &EntityRef) -> bool {
    matches!(r, EntityRef::ToolGroup(_))
}

struct GroupCtx<'a> {
    group: &'a AccessGroup,
    owned: Vec<&'a StoredPolicy>,
    principal_uid: EntityUid,
    action_uid: EntityUid,
}

/// Returns `GroupConflict` if intent isn't reachable using only
/// delete-our-rows + emit-our-rows.
pub fn plan_for_group(
    group: &AccessGroup,
    ctx: &PlanCtx,
) -> Result<Vec<Edit>, GroupConflict> {
    let collapsed = maybe_collapse_to_ancestors(group, ctx.hierarchy);
    let group = &collapsed;
    let gctx = GroupCtx {
        group,
        owned: ctx
            .live
            .iter()
            .copied()
            .filter(|p| p.enabled && p.user_id.is_some())
            .filter(|p| {
                let Ok(parsed) = parse_first(&p.policy_text) else {
                    return false;
                };
                is_owned(&parsed, &group.principal, &group.action, p.user_id.is_none())
            })
            .collect(),
        principal_uid: group.principal.to_cedar_uid(),
        action_uid: action_uid(&group.action),
    };

    let mut pending = PendingEdits::new();

    match canonical_shape(group) {
        CanonicalShape::Wildcard { effect } => {
            let name = group_policy_name(&group.principal, &group.action);
            let text = build_wildcard_text(&name, &group.principal, &group.action, effect);
            apply_canonical_emission(&mut pending, &gctx.owned, &name, &text);
        }
        CanonicalShape::Carveout {
            effect,
            ref carveout_resources,
        } => {
            let name = group_policy_name(&group.principal, &group.action);
            let text = build_carveout_text(&name, &group.principal, &group.action, effect, carveout_resources);
            apply_canonical_emission(&mut pending, &gctx.owned, &name, &text);
        }
        CanonicalShape::PerOverride => {
            let mut kept_names: HashSet<String> = HashSet::new();

            for ov in &group.overrides {
                if matches!(ov.intent, AccessIntent::Default) {
                    delete_owned_for_resource(&mut pending, &gctx.owned, &ov.resource);
                    continue;
                }
                plan_override(&gctx, ov, &mut pending, &mut kept_names, ctx)?;
            }

            for row in &gctx.owned {
                let already_marked = pending.deletes.iter().any(|id| id == &row.id)
                    || kept_names.contains(&row.name);
                if !already_marked {
                    pending.deletes.push(row.id.clone());
                }
            }
        }
    }

    verify_group(&gctx, &pending, ctx)?;

    Ok(pending_to_edits(pending))
}

/// Replaces N descendant overrides with one ancestor override when (a) every
/// descendant in the bucket shares the same intent and (b) the bucket covers
/// the ancestor's full known descendant set.
///
/// The "covers full set" check is closed-world correctness: collapsing when a
/// descendant exists outside the override list would shift its decision from
/// baseline to ancestor-rule. Closed-world callers like `reconcile_agent_tools`
/// always pass a complete universe, so this fires there. Partial submissions
/// fall back to per-descendant emission.
pub fn maybe_collapse_to_ancestors(
    group: &AccessGroup,
    hierarchy: &ResourceHierarchy,
) -> AccessGroup {
    use std::collections::HashMap;

    if hierarchy.parent.is_empty() {
        return group.clone();
    }

    let mut by_ancestor: HashMap<EntityRef, Vec<&AccessOverride>> = HashMap::new();
    let mut passthrough: Vec<AccessOverride> = Vec::new();

    for ov in &group.overrides {
        match hierarchy.parent_of(&ov.resource) {
            Some(ancestor) => by_ancestor.entry(ancestor.clone()).or_default().push(ov),
            None => passthrough.push(ov.clone()),
        }
    }

    let mut new_overrides = passthrough;

    for (ancestor, members) in by_ancestor {
        let first_intent = members[0].intent;
        let same_intent = members.iter().all(|ov| ov.intent == first_intent);
        let known: &[EntityRef] = hierarchy.descendants_of(&ancestor).unwrap_or(&[]);
        let covered: std::collections::HashSet<&EntityRef> =
            members.iter().map(|ov| &ov.resource).collect();
        let all_covered = !known.is_empty() && known.iter().all(|d| covered.contains(d));

        if same_intent && all_covered {
            new_overrides.push(AccessOverride {
                resource: ancestor,
                intent: first_intent,
            });
        } else {
            for ov in members {
                new_overrides.push(ov.clone());
            }
        }
    }

    AccessGroup {
        principal: group.principal.clone(),
        action: group.action.clone(),
        default: group.default,
        overrides: new_overrides,
    }
}

/// Drops redundant overrides (intent matching `default`) silently.
pub fn canonical_shape(group: &AccessGroup) -> CanonicalShape {
    let Some(default) = group.default else {
        return CanonicalShape::PerOverride;
    };

    let carveout: Vec<EntityRef> = group
        .overrides
        .iter()
        .filter(|ov| ov.intent != default && ov.intent != AccessIntent::Default)
        .map(|ov| ov.resource.clone())
        .collect();

    let effect = match default {
        AccessIntent::Allow => Effect::Permit,
        AccessIntent::Deny => Effect::Forbid,
        AccessIntent::Default => return CanonicalShape::PerOverride,
    };

    if carveout.is_empty() {
        CanonicalShape::Wildcard { effect }
    } else {
        CanonicalShape::Carveout {
            effect,
            carveout_resources: carveout,
        }
    }
}

pub fn build_per_resource_text(
    name: &str,
    principal: &EntityRef,
    action: &str,
    resource: &EntityRef,
    effect: Effect,
) -> String {
    let resource_clause = if uses_in_constraint(resource) {
        resource_in_json(resource)
    } else {
        resource_eq_json(resource)
    };
    let body = serde_json::json!({
        "effect": effect_keyword(effect),
        "principal": principal_eq_json(principal),
        "action": action_eq_json(action),
        "resource": resource_clause,
        "annotations": { "id": name },
        "conditions": [],
    });
    Policy::from_json(Some(PolicyId::new(name)), body)
        .expect("valid per-resource policy JSON")
        .to_string()
}

pub fn build_wildcard_text(
    name: &str,
    principal: &EntityRef,
    action: &str,
    effect: Effect,
) -> String {
    let body = serde_json::json!({
        "effect": effect_keyword(effect),
        "principal": principal_eq_json(principal),
        "action": action_eq_json(action),
        "resource": { "op": "All" },
        "annotations": { "id": name },
        "conditions": [],
    });
    Policy::from_json(Some(PolicyId::new(name)), body)
        .expect("valid wildcard policy JSON")
        .to_string()
}

pub fn build_carveout_text(
    name: &str,
    principal: &EntityRef,
    action: &str,
    effect: Effect,
    carveout_resources: &[EntityRef],
) -> String {
    let mut sorted: Vec<&EntityRef> = carveout_resources.iter().collect();
    sorted.sort_by(|a, b| (a.cedar_type(), a.id()).cmp(&(b.cedar_type(), b.id())));
    let body = serde_json::json!({
        "effect": effect_keyword(effect),
        "principal": principal_eq_json(principal),
        "action": action_eq_json(action),
        "resource": { "op": "All" },
        "annotations": { "id": name },
        "conditions": [{
            "kind": "unless",
            "body": resource_or_chain(&sorted),
        }],
    });
    Policy::from_json(Some(PolicyId::new(name)), body)
        .expect("valid carveout policy JSON")
        .to_string()
}

pub fn group_policy_name(principal: &EntityRef, action: &str) -> String {
    format!(
        "reconcile-{}-{}-{}",
        cedar_type_short(principal.cedar_type()),
        slug(principal.id()),
        action,
    )
}

pub fn per_resource_policy_name(
    principal: &EntityRef,
    action: &str,
    resource: &EntityRef,
    effect: Effect,
) -> String {
    let eff = match effect {
        Effect::Permit => "p",
        Effect::Forbid => "f",
    };
    format!(
        "reconcile-{}-{}-{}-{}-{}-{}",
        cedar_type_short(principal.cedar_type()),
        slug(principal.id()),
        action,
        eff,
        cedar_type_short(resource.cedar_type()),
        slug(resource.id()),
    )
}

/// True iff `policy` is a row reconcile is allowed to delete or replace for
/// the group `(principal, action)`. Four structural shapes qualify:
///   1. Per-resource Eq: `effect(P==principal, A==action, R==Eq);` no conditions.
///   2. Per-resource In: `effect(P==principal, A==action, R in Ancestor);` no conditions.
///   3. Wildcard:        `effect(P==principal, A==action, resource);` no conditions.
///   4. Carveout:        `effect(P==principal, A==action, resource) unless { R==v1 || R==v2 || ... };`
///      where every `||` term is a `resource == EntityType::"id"` of the same
///      Cedar entity type.
///
/// Plus `user_id IS NOT NONE` (system rows are never owned).
pub fn is_owned(policy: &Policy, principal: &EntityRef, action: &str, system_scope: bool) -> bool {
    if system_scope {
        return false;
    }
    if !principal_constraint_matches(policy, principal) {
        return false;
    }
    if !action_constraint_matches(policy, action) {
        return false;
    }
    match policy.resource_constraint() {
        ResourceConstraint::Eq(_) | ResourceConstraint::In(_) => has_no_conditions(policy),
        ResourceConstraint::Any => matches!(
            condition_kind(policy),
            ConditionKind::None | ConditionKind::CarveoutUnless { .. }
        ),
        _ => false,
    }
}

pub fn fingerprint(policies: &[&StoredPolicy]) -> u64 {
    let mut sorted: Vec<&&StoredPolicy> = policies.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let mut hasher = Sha256::new();
    for p in sorted {
        hasher.update(p.id.as_bytes());
        hasher.update(b"\x00");
        hasher.update(p.updated_at.to_rfc3339().as_bytes());
        hasher.update(b"\x00");
        hasher.update(p.policy_text.as_bytes());
        hasher.update(b"\x00");
    }
    let d = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&d[..8]);
    u64::from_be_bytes(bytes)
}

pub fn eval_request(
    policy_set: &PolicySet,
    schema: &Schema,
    principal: &EntityUid,
    action: &EntityUid,
    resource: &EntityUid,
    entities: &Entities,
) -> Decision {
    let Ok(req) = Request::new(
        principal.clone(),
        action.clone(),
        resource.clone(),
        Context::empty(),
        Some(schema),
    ) else {
        return Decision::Deny;
    };
    let authorizer = Authorizer::new();
    authorizer.is_authorized(&req, policy_set, entities).decision()
}

pub fn sentinel_resource(entity_type: &str) -> EntityRef {
    let id = "___reconcile_sentinel___".to_string();
    match entity_type {
        "Policy::Path" => EntityRef::Path(id),
        "Policy::NetworkDestination" => EntityRef::NetworkDestination(id),
        "Policy::Tool" => EntityRef::Tool(id),
        _ => EntityRef::Path(id),
    }
}

pub fn resource_type_for_action(action: &str) -> &'static str {
    match action {
        "read" | "write" => "Policy::Path",
        "connect" | "bind" => "Policy::NetworkDestination",
        "invoke_tool" => "Policy::Tool",
        _ => "Policy::Path",
    }
}

struct PendingEdits {
    deletes: Vec<String>,
    creates: Vec<NewPolicy>,
}

#[derive(Debug, Clone)]
struct NewPolicy {
    name: String,
    text: String,
}

impl PendingEdits {
    fn new() -> Self {
        Self {
            deletes: Vec::new(),
            creates: Vec::new(),
        }
    }
    fn deleted_ids(&self) -> HashSet<&str> {
        self.deletes.iter().map(String::as_str).collect()
    }
}

fn build_effective_policy_set(
    ctx: &PlanCtx,
    pending: &PendingEdits,
) -> Result<PolicySet, AppError> {
    let deleted = pending.deleted_ids();
    let mut combined = String::new();
    for p in ctx.live {
        if !p.enabled || deleted.contains(p.id.as_str()) {
            continue;
        }
        combined.push_str(&p.policy_text);
        combined.push('\n');
    }
    for c in &pending.creates {
        combined.push_str(&c.text);
        combined.push('\n');
    }
    let mut set = if combined.trim().is_empty() {
        PolicySet::new()
    } else {
        PolicySet::from_str(&combined)
            .map_err(|e| AppError::Internal(format!("Failed to parse policies: {e}")))?
    };
    for m in ctx.managed {
        set.add(m.clone())
            .map_err(|e| AppError::Internal(format!("Failed to add managed policy: {e}")))?;
    }
    Ok(set)
}

fn pending_to_edits(p: PendingEdits) -> Vec<Edit> {
    let mut out = Vec::with_capacity(p.deletes.len() + p.creates.len());
    for policy_id in p.deletes {
        out.push(Edit::Delete { policy_id });
    }
    for c in p.creates {
        out.push(Edit::Create {
            name: c.name,
            policy_text: c.text,
        });
    }
    out
}

fn apply_canonical_emission(
    pending: &mut PendingEdits,
    owned: &[&StoredPolicy],
    expected_name: &str,
    expected_text: &str,
) {
    let kept_id = owned
        .iter()
        .find(|p| p.policy_text == expected_text)
        .map(|p| p.id.clone());
    for row in owned {
        if Some(&row.id) == kept_id.as_ref() {
            continue;
        }
        pending.deletes.push(row.id.clone());
    }
    if kept_id.is_none() {
        pending.creates.push(NewPolicy {
            name: expected_name.to_string(),
            text: expected_text.to_string(),
        });
    }
}

fn delete_owned_for_resource(
    pending: &mut PendingEdits,
    owned: &[&StoredPolicy],
    resource: &EntityRef,
) {
    for row in owned {
        let Ok(parsed) = parse_first(&row.policy_text) else {
            continue;
        };
        if !has_no_conditions(&parsed) {
            continue;
        }
        let matches_resource = match parsed.resource_constraint() {
            ResourceConstraint::Eq(uid) | ResourceConstraint::In(uid) => {
                format!("{}", uid.type_name()) == resource.cedar_type()
                    && uid.id().unescaped() == resource.id()
            }
            _ => false,
        };
        if matches_resource {
            pending.deletes.push(row.id.clone());
        }
    }
}

fn plan_override(
    gctx: &GroupCtx,
    ov: &AccessOverride,
    pending: &mut PendingEdits,
    kept_names: &mut HashSet<String>,
    ctx: &PlanCtx,
) -> Result<(), GroupConflict> {
    let group = gctx.group;
    let resource_uid = ov.resource.to_cedar_uid();

    // An owned row "covers" this override's resource if it constrains the
    // exact resource OR an ancestor (e.g., a ToolGroup forbid covers each of
    // its Tool descendants). Ancestor rows must be deleted to flip the
    // descendant's decision since `forbid` wins in Cedar; the sibling tools
    // get re-emitted by their own override iterations.
    let ancestor = ctx.hierarchy.parent_of(&ov.resource);
    let owned_for_resource = |effect: Effect| -> Option<&StoredPolicy> {
        gctx.owned.iter().copied().find(|row| {
            let Ok(parsed) = parse_first(&row.policy_text) else {
                return false;
            };
            if parsed.effect() != effect || !has_no_conditions(&parsed) {
                return false;
            }
            let resource_match = match parsed.resource_constraint() {
                ResourceConstraint::Eq(u) | ResourceConstraint::In(u) => {
                    let row_type = format!("{}", u.type_name());
                    let row_id = u.id().unescaped().to_string();
                    let direct = row_type == ov.resource.cedar_type() && row_id == ov.resource.id();
                    let via_ancestor = ancestor
                        .is_some_and(|a| row_type == a.cedar_type() && row_id == a.id());
                    direct || via_ancestor
                }
                _ => false,
            };
            resource_match
                && matches!(parsed.principal_constraint(), PrincipalConstraint::Eq(ref u)
                    if format!("{}", u.type_name()) == group.principal.cedar_type()
                    && u.id().unescaped() == group.principal.id())
                && matches!(parsed.action_constraint(), ActionConstraint::Eq(ref u)
                    if u.id().unescaped() == group.action)
        })
    };

    let eval_now = |pending: &PendingEdits| -> Result<Decision, AppError> {
        let set = build_effective_policy_set(ctx, pending)?;
        Ok(eval_request(
            &set,
            ctx.schema,
            &gctx.principal_uid,
            &gctx.action_uid,
            &resource_uid,
            ctx.entities,
        ))
    };

    let intent_decision = match ov.intent {
        AccessIntent::Allow => Decision::Allow,
        AccessIntent::Deny => Decision::Deny,
        AccessIntent::Default => unreachable!("Default handled by caller"),
    };

    let current = eval_now(pending).map_err(|e| db_conflict(group, e))?;

    if current == intent_decision {
        if let Some(matching) = owned_for_resource(if intent_decision == Decision::Allow {
            Effect::Permit
        } else {
            Effect::Forbid
        }) {
            kept_names.insert(matching.name.clone());
        }
        if let Some(stale) = owned_for_resource(if intent_decision == Decision::Allow {
            Effect::Forbid
        } else {
            Effect::Permit
        }) {
            pending.deletes.push(stale.id.clone());
        }
        return Ok(());
    }

    let opposite = match intent_decision {
        Decision::Allow => owned_for_resource(Effect::Forbid),
        Decision::Deny => owned_for_resource(Effect::Permit),
    };
    if let Some(row) = opposite {
        pending.deletes.push(row.id.clone());
        let after = eval_now(pending).map_err(|e| db_conflict(group, e))?;
        if after == intent_decision {
            return Ok(());
        }
    }

    let effect = match intent_decision {
        Decision::Allow => Effect::Permit,
        Decision::Deny => Effect::Forbid,
    };
    let name = per_resource_policy_name(&group.principal, &group.action, &ov.resource, effect);
    let text = build_per_resource_text(&name, &group.principal, &group.action, &ov.resource, effect);
    pending.creates.push(NewPolicy {
        name: name.clone(),
        text,
    });
    kept_names.insert(name);

    let after = eval_now(pending).map_err(|e| db_conflict(group, e))?;
    if after == intent_decision {
        return Ok(());
    }

    let reason = if intent_decision == Decision::Allow {
        ConflictReason::AllowBlockedByForbid
    } else {
        ConflictReason::DenyBlockedByPermit
    };
    let blockers = collect_blockers(ctx, &pending.deleted_ids(), &group.principal, &group.action, &ov.resource, intent_decision);
    Err(GroupConflict {
        principal: group.principal.clone(),
        action: group.action.clone(),
        failing_resource: Some(ov.resource.clone()),
        blockers,
        reason,
    })
}

fn verify_group(
    gctx: &GroupCtx,
    pending: &PendingEdits,
    ctx: &PlanCtx,
) -> Result<(), GroupConflict> {
    let group = gctx.group;
    let set = build_effective_policy_set(ctx, pending)
        .map_err(|e| db_conflict(group, e))?;

    if let Some(default) = group.default {
        let want = match default {
            AccessIntent::Allow => Decision::Allow,
            AccessIntent::Deny => Decision::Deny,
            AccessIntent::Default => return Ok(()),
        };
        let resource_type = resource_type_for_action(&group.action);
        let sentinel = sentinel_resource(resource_type);
        let sentinel_uid = sentinel.to_cedar_uid();
        let actual = eval_request(&set, ctx.schema, &gctx.principal_uid, &gctx.action_uid, &sentinel_uid, ctx.entities);
        if actual != want {
            let reason = if want == Decision::Allow {
                ConflictReason::AllowBlockedByForbid
            } else {
                ConflictReason::DenyBlockedByPermit
            };
            return Err(GroupConflict {
                principal: group.principal.clone(),
                action: group.action.clone(),
                failing_resource: None,
                blockers: collect_blockers(ctx, &pending.deleted_ids(), &group.principal, &group.action, &sentinel, want),
                reason,
            });
        }
    }

    for ov in &group.overrides {
        let want = match ov.intent {
            AccessIntent::Allow => Decision::Allow,
            AccessIntent::Deny => Decision::Deny,
            AccessIntent::Default => continue,
        };
        let resource_uid = ov.resource.to_cedar_uid();
        let actual = eval_request(&set, ctx.schema, &gctx.principal_uid, &gctx.action_uid, &resource_uid, ctx.entities);
        if actual != want {
            let reason = if want == Decision::Allow {
                ConflictReason::AllowBlockedByForbid
            } else {
                ConflictReason::DenyBlockedByPermit
            };
            return Err(GroupConflict {
                principal: group.principal.clone(),
                action: group.action.clone(),
                failing_resource: Some(ov.resource.clone()),
                blockers: collect_blockers(ctx, &pending.deleted_ids(), &group.principal, &group.action, &ov.resource, want),
                reason,
            });
        }
    }

    Ok(())
}

fn collect_blockers(
    ctx: &PlanCtx,
    deleted_ids: &HashSet<&str>,
    principal: &EntityRef,
    action: &str,
    resource: &EntityRef,
    intent: Decision,
) -> Vec<BlockerPolicy> {
    let want_effect = match intent {
        Decision::Allow => Effect::Forbid,
        Decision::Deny => Effect::Permit,
    };
    let mut out = Vec::new();
    for row in ctx.live {
        if !row.enabled || deleted_ids.contains(row.id.as_str()) {
            continue;
        }
        let Ok(parsed) = parse_first(&row.policy_text) else {
            continue;
        };
        if parsed.effect() != want_effect {
            continue;
        }
        let principal_match = match parsed.principal_constraint() {
            PrincipalConstraint::Eq(u) => {
                format!("{}", u.type_name()) == principal.cedar_type()
                    && u.id().unescaped() == principal.id()
            }
            PrincipalConstraint::Any => true,
            _ => false,
        };
        let action_match = match parsed.action_constraint() {
            ActionConstraint::Eq(u) => u.id().unescaped() == action,
            ActionConstraint::Any => true,
            _ => false,
        };
        let resource_match = match parsed.resource_constraint() {
            ResourceConstraint::Eq(u) => {
                format!("{}", u.type_name()) == resource.cedar_type()
                    && u.id().unescaped() == resource.id()
            }
            ResourceConstraint::Any => true,
            _ => false,
        };
        if principal_match && action_match && resource_match {
            out.push(BlockerPolicy {
                policy_id: row.id.clone(),
                policy_name: row.name.clone(),
            });
        }
    }
    out
}

fn db_conflict(group: &AccessGroup, e: AppError) -> GroupConflict {
    tracing::error!(error = %e, "policy plan eval failed");
    GroupConflict {
        principal: group.principal.clone(),
        action: group.action.clone(),
        failing_resource: None,
        blockers: vec![],
        reason: ConflictReason::AllowBlockedByForbid,
    }
}

fn principal_constraint_matches(policy: &Policy, principal: &EntityRef) -> bool {
    let PrincipalConstraint::Eq(uid) = policy.principal_constraint() else {
        return false;
    };
    format!("{}", uid.type_name()) == principal.cedar_type()
        && uid.id().unescaped() == principal.id()
}

fn action_constraint_matches(policy: &Policy, action: &str) -> bool {
    let ActionConstraint::Eq(uid) = policy.action_constraint() else {
        return false;
    };
    uid.id().unescaped() == action
}

fn has_no_conditions(policy: &Policy) -> bool {
    matches!(condition_kind(policy), ConditionKind::None)
}

#[derive(Debug, PartialEq, Eq)]
enum ConditionKind {
    None,
    /// `unless { resource == T::"v1" || resource == T::"v2" || ... }` where
    /// every term is a resource-eq with the same entity type.
    CarveoutUnless { resources: Vec<(String, String)> },
    Other,
}

fn condition_kind(policy: &Policy) -> ConditionKind {
    let Ok(json) = policy.to_json() else {
        return ConditionKind::Other;
    };
    let Some(conditions) = json.get("conditions").and_then(|c| c.as_array()) else {
        return ConditionKind::Other;
    };
    if conditions.is_empty() {
        return ConditionKind::None;
    }
    if conditions.len() != 1 {
        return ConditionKind::Other;
    }
    let cond = &conditions[0];
    if cond.get("kind").and_then(|k| k.as_str()) != Some("unless") {
        return ConditionKind::Other;
    }
    let Some(body) = cond.get("body") else {
        return ConditionKind::Other;
    };
    let mut resources = Vec::new();
    if !collect_resource_or_terms(body, &mut resources) {
        return ConditionKind::Other;
    }
    if resources.is_empty() {
        return ConditionKind::Other;
    }
    let first_type = resources[0].0.clone();
    if !resources.iter().all(|(t, _)| t == &first_type) {
        return ConditionKind::Other;
    }
    ConditionKind::CarveoutUnless { resources }
}

fn collect_resource_or_terms(node: &serde_json::Value, out: &mut Vec<(String, String)>) -> bool {
    if let Some(or) = node.get("||") {
        let l = or.get("left").map(|v| collect_resource_or_terms(v, out)).unwrap_or(false);
        let r = or.get("right").map(|v| collect_resource_or_terms(v, out)).unwrap_or(false);
        return l && r;
    }
    if let Some(eq) = node.get("==") {
        let (left, right) = (eq.get("left"), eq.get("right"));
        if let (Some(l), Some(r)) = (left, right)
            && let Some(entity) = match_resource_var_eq(l, r)
        {
            out.push(entity);
            return true;
        }
    }
    false
}

fn match_resource_var_eq(
    left: &serde_json::Value,
    right: &serde_json::Value,
) -> Option<(String, String)> {
    let is_resource_var = |v: &serde_json::Value| -> bool {
        if v.get("Var").and_then(|s| s.as_str()) == Some("resource") {
            return true;
        }
        if let Some(arr) = v.get("unknown").and_then(|u| u.as_array())
            && let Some(name) = arr.first().and_then(|v| v.get("Value")).and_then(|v| v.as_str())
            && name == "resource"
        {
            return true;
        }
        false
    };
    let extract_entity = |v: &serde_json::Value| -> Option<(String, String)> {
        let entity = v
            .get("Value")
            .and_then(|v| v.get("__entity"))
            .or_else(|| v.get("__entity"))?;
        let t = entity.get("type").and_then(|v| v.as_str())?.to_string();
        let id = entity.get("id").and_then(|v| v.as_str())?.to_string();
        Some((t, id))
    };
    if is_resource_var(left) {
        return extract_entity(right);
    }
    if is_resource_var(right) {
        return extract_entity(left);
    }
    None
}

fn principal_eq_json(p: &EntityRef) -> serde_json::Value {
    serde_json::json!({
        "op": "==",
        "entity": { "type": p.cedar_type(), "id": p.id() }
    })
}

fn action_eq_json(action: &str) -> serde_json::Value {
    serde_json::json!({
        "op": "==",
        "entity": { "type": "Policy::Action", "id": action }
    })
}

fn resource_eq_json(r: &EntityRef) -> serde_json::Value {
    serde_json::json!({
        "op": "==",
        "entity": { "type": r.cedar_type(), "id": r.id() }
    })
}

fn resource_in_json(r: &EntityRef) -> serde_json::Value {
    serde_json::json!({
        "op": "in",
        "entity": { "type": r.cedar_type(), "id": r.id() }
    })
}

fn resource_or_chain(resources: &[&EntityRef]) -> serde_json::Value {
    fn eq_clause(r: &EntityRef) -> serde_json::Value {
        serde_json::json!({
            "==": {
                "left": { "Var": "resource" },
                "right": { "Value": { "__entity": { "type": r.cedar_type(), "id": r.id() } } },
            }
        })
    }
    let mut iter = resources.iter();
    let first = iter.next().expect("at least one resource");
    let mut acc = eq_clause(first);
    for r in iter {
        acc = serde_json::json!({
            "||": { "left": acc, "right": eq_clause(r) }
        });
    }
    acc
}

fn effect_keyword(e: Effect) -> &'static str {
    match e {
        Effect::Permit => "permit",
        Effect::Forbid => "forbid",
    }
}

fn cedar_type_short(t: &str) -> &str {
    t.strip_prefix("Policy::").unwrap_or(t)
}

fn slug(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let d = hasher.finalize();
    let mut out = String::with_capacity(12);
    for b in &d[..6] {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn parse_first(text: &str) -> Result<Policy, ()> {
    let set = PolicySet::from_str(text).map_err(|_| ())?;
    set.policies().next().cloned().ok_or(())
}

fn action_uid(action: &str) -> EntityUid {
    EntityUid::from_str(&format!("Policy::Action::\"{action}\"")).expect("valid action uid")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str) -> EntityRef {
        EntityRef::Agent(id.into())
    }

    fn dir(p: &str) -> EntityRef {
        EntityRef::Path(p.into())
    }

    fn parse_one(text: &str) -> Policy {
        PolicySet::from_str(text)
            .expect("parse")
            .policies()
            .next()
            .expect("one policy")
            .clone()
    }

    fn stored(name: &str, text: &str) -> StoredPolicy {
        let now = chrono::Utc::now();
        StoredPolicy {
            id: crate::core::repository::new_id(),
            user_id: Some("u".into()),
            name: name.into(),
            description: String::new(),
            policy_text: text.into(),
            enabled: true,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn entity_ref_to_cedar_type() {
        assert_eq!(EntityRef::Agent("a".into()).cedar_type(), "Policy::Agent");
        assert_eq!(EntityRef::Path("/x".into()).cedar_type(), "Policy::Path");
    }

    #[test]
    fn canonical_shape_default_none_no_overrides() {
        let group = AccessGroup {
            principal: agent("a"),
            action: "read".into(),
            default: None,
            overrides: vec![],
        };
        assert!(matches!(canonical_shape(&group), CanonicalShape::PerOverride));
    }

    #[test]
    fn canonical_shape_default_deny_with_allow_overrides_emits_carveout() {
        let group = AccessGroup {
            principal: agent("a"),
            action: "connect".into(),
            default: Some(AccessIntent::Deny),
            overrides: vec![AccessOverride {
                resource: EntityRef::NetworkDestination("gmail.com".into()),
                intent: AccessIntent::Allow,
            }],
        };
        match canonical_shape(&group) {
            CanonicalShape::Carveout { effect, carveout_resources } => {
                assert_eq!(effect, Effect::Forbid);
                assert_eq!(carveout_resources.len(), 1);
            }
            _ => panic!("expected Carveout"),
        }
    }

    #[test]
    fn canonical_shape_default_deny_no_overrides_emits_wildcard() {
        let group = AccessGroup {
            principal: agent("a"),
            action: "connect".into(),
            default: Some(AccessIntent::Deny),
            overrides: vec![],
        };
        match canonical_shape(&group) {
            CanonicalShape::Wildcard { effect } => assert_eq!(effect, Effect::Forbid),
            _ => panic!("expected Wildcard"),
        }
    }

    #[test]
    fn canonical_shape_drops_redundant_overrides_matching_default() {
        let group = AccessGroup {
            principal: agent("a"),
            action: "connect".into(),
            default: Some(AccessIntent::Deny),
            overrides: vec![
                AccessOverride {
                    resource: EntityRef::NetworkDestination("evil.com".into()),
                    intent: AccessIntent::Deny,
                },
                AccessOverride {
                    resource: EntityRef::NetworkDestination("gmail.com".into()),
                    intent: AccessIntent::Allow,
                },
            ],
        };
        match canonical_shape(&group) {
            CanonicalShape::Carveout { carveout_resources, .. } => {
                assert_eq!(carveout_resources.len(), 1);
                assert_eq!(carveout_resources[0].id(), "gmail.com");
            }
            _ => panic!("expected Carveout"),
        }
    }

    #[test]
    fn is_owned_recognizes_per_resource_simple() {
        let p = parse_one(
            r#"@id("x")
            permit(principal == Policy::Agent::"a", action == Policy::Action::"read", resource == Policy::Path::"/x");"#,
        );
        assert!(is_owned(&p, &agent("a"), "read", false));
    }

    #[test]
    fn is_owned_recognizes_per_resource_in() {
        let p = parse_one(
            r#"@id("x")
            forbid(principal == Policy::Agent::"a", action == Policy::Action::"invoke_tool", resource in Policy::ToolGroup::"github");"#,
        );
        assert!(is_owned(&p, &agent("a"), "invoke_tool", false));
    }

    #[test]
    fn is_owned_rejects_in_with_when_clause() {
        let p = parse_one(
            r#"@id("x")
            forbid(principal == Policy::Agent::"a", action == Policy::Action::"invoke_tool", resource in Policy::ToolGroup::"github")
            when { principal.enabled };"#,
        );
        assert!(!is_owned(&p, &agent("a"), "invoke_tool", false));
    }

    #[test]
    fn is_owned_recognizes_wildcard() {
        let p = parse_one(
            r#"@id("x")
            forbid(principal == Policy::Agent::"a", action == Policy::Action::"connect", resource);"#,
        );
        assert!(is_owned(&p, &agent("a"), "connect", false));
    }

    #[test]
    fn is_owned_recognizes_carveout() {
        let p = parse_one(
            r#"@id("x")
            forbid(principal == Policy::Agent::"a", action == Policy::Action::"connect", resource)
            unless { resource == Policy::NetworkDestination::"gmail.com" || resource == Policy::NetworkDestination::"api.github.com" };"#,
        );
        assert!(is_owned(&p, &agent("a"), "connect", false));
    }

    #[test]
    fn is_owned_rejects_when_clause() {
        let p = parse_one(
            r#"@id("x")
            permit(principal == Policy::Agent::"a", action == Policy::Action::"read", resource == Policy::Path::"/x")
            when { principal.enabled };"#,
        );
        assert!(!is_owned(&p, &agent("a"), "read", false));
    }

    #[test]
    fn is_owned_rejects_principal_in_group() {
        let p = parse_one(
            r#"@id("x")
            permit(principal in Policy::User::"alice", action == Policy::Action::"read", resource == Policy::Path::"/x");"#,
        );
        assert!(!is_owned(&p, &agent("a"), "read", false));
    }

    #[test]
    fn is_owned_rejects_wildcard_principal() {
        let p = parse_one(
            r#"@id("x")
            permit(principal, action == Policy::Action::"read", resource == Policy::Path::"/x");"#,
        );
        assert!(!is_owned(&p, &agent("a"), "read", false));
    }

    #[test]
    fn is_owned_rejects_system_scoped_row() {
        let p = parse_one(
            r#"@id("x")
            permit(principal == Policy::Agent::"a", action == Policy::Action::"read", resource == Policy::Path::"/x");"#,
        );
        assert!(!is_owned(&p, &agent("a"), "read", true));
    }

    #[test]
    fn is_owned_rejects_carveout_with_mixed_resource_types() {
        let p = parse_one(
            r#"@id("x")
            forbid(principal == Policy::Agent::"a", action == Policy::Action::"connect", resource)
            unless { resource == Policy::NetworkDestination::"gmail.com" || resource == Policy::Path::"/x" };"#,
        );
        assert!(!is_owned(&p, &agent("a"), "connect", false));
    }

    #[test]
    fn is_owned_rejects_wrong_action() {
        let p = parse_one(
            r#"@id("x")
            permit(principal == Policy::Agent::"a", action == Policy::Action::"write", resource == Policy::Path::"/x");"#,
        );
        assert!(!is_owned(&p, &agent("a"), "read", false));
    }

    #[test]
    fn fingerprint_stable() {
        let p = stored("n", "permit(principal, action, resource);");
        let a = fingerprint(&[&p]);
        let b = fingerprint(&[&p]);
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_changes_when_text_changes() {
        let mut p = stored("n", "permit(principal, action, resource);");
        let a = fingerprint(&[&p]);
        p.policy_text = "forbid(principal, action, resource);".into();
        assert_ne!(a, fingerprint(&[&p]));
    }

    #[test]
    fn build_per_resource_text_emits_canonical() {
        let text = build_per_resource_text(
            "test",
            &agent("a"),
            "read",
            &dir("/x"),
            Effect::Permit,
        );
        assert!(text.contains("permit"));
        assert!(text.contains("Policy::Agent"));
        assert!(text.contains("Policy::Path"));
    }

    #[test]
    fn group_policy_name_is_deterministic() {
        let n1 = group_policy_name(&agent("a"), "read");
        let n2 = group_policy_name(&agent("a"), "read");
        assert_eq!(n1, n2);
        assert!(n1.starts_with("reconcile-Agent-"));
    }
}
