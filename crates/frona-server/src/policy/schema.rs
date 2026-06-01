use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

use cedar_policy::{
    Entities, Entity, EntityId, EntityTypeName, EntityUid, PolicySet, RestrictedExpression, Schema,
};

use crate::core::Handle;
use crate::core::principal::{Principal, PrincipalKind};

pub const NAMESPACE: &str = "Policy";

pub fn build_schema() -> Arc<Schema> {
    let (schema, warnings) = Schema::from_cedarschema_str(
        include_str!("../../../../resources/policy/frona.cedarschema"),
    )
        .expect("Failed to parse built-in policy schema");

    for warning in warnings {
        tracing::warn!(%warning, "Policy schema warning");
    }

    Arc::new(schema)
}

pub fn entity_type_name(type_name: &str) -> EntityTypeName {
    format!("{NAMESPACE}::{type_name}")
        .parse()
        .expect("valid entity type name")
}

pub fn entity_uid(type_name: &str, id: &str) -> EntityUid {
    EntityUid::from_type_name_and_id(entity_type_name(type_name), EntityId::new(id))
}

pub fn agent_entity_uid(user_handle: &Handle, agent_handle: &Handle) -> EntityUid {
    entity_uid("Agent", &format!("{user_handle}/{agent_handle}"))
}

pub fn app_entity_uid(user_handle: &Handle, app_handle: &Handle) -> EntityUid {
    entity_uid("App", &format!("{user_handle}/{app_handle}"))
}

pub fn mcp_entity_uid(user_handle: &Handle, mcp_handle: &Handle) -> EntityUid {
    entity_uid("Mcp", &format!("{user_handle}/{mcp_handle}"))
}

pub fn user_entity_uid(user_id: &str) -> EntityUid {
    entity_uid("User", user_id)
}

pub fn user_entity_uid_from_handle(handle: &Handle) -> EntityUid {
    entity_uid("User", handle.as_ref())
}

pub fn user_group_entity_uid(group: &str) -> EntityUid {
    entity_uid("UserGroup", group)
}

/// Cedar requires parent entities to be present in the request's `Entities` set,
/// not just named as parents on the principal — otherwise `principal in
/// UserGroup::"x"` evaluates false.
pub fn build_user_action_entities(
    principal_id: &str,
    principal_groups: &[String],
    target_id: &str,
) -> Entities {
    let group_uids: HashSet<EntityUid> = principal_groups
        .iter()
        .map(|g| user_group_entity_uid(g))
        .collect();

    let principal_entity = Entity::new_no_attrs(user_entity_uid(principal_id), group_uids.clone());

    let mut all = vec![principal_entity];
    if target_id != principal_id {
        all.push(Entity::new_no_attrs(
            user_entity_uid(target_id),
            HashSet::new(),
        ));
    }
    for uid in group_uids {
        all.push(Entity::new_no_attrs(uid, HashSet::new()));
    }
    Entities::from_entities(all, None).unwrap_or_else(|_| Entities::empty())
}

/// For `Agent`, callers must pre-rewrite `principal.id` to `{user_handle}/{agent_handle}`.
pub fn principal_entity_uid(principal: &Principal) -> EntityUid {
    let type_name = match principal.kind {
        PrincipalKind::User => "User",
        PrincipalKind::Agent => "Agent",
        PrincipalKind::McpServer => "Mcp",
        PrincipalKind::App => "App",
        PrincipalKind::Channel => "Channel",
    };
    entity_uid(type_name, &principal.id)
}

fn tools_to_set(tools: &[String]) -> RestrictedExpression {
    let elements: Vec<RestrictedExpression> = tools
        .iter()
        .map(|t| RestrictedExpression::new_string(t.clone()))
        .collect();
    RestrictedExpression::new_set(elements)
}

pub fn build_agent_principal_entity(user_handle: &Handle, agent_handle: &Handle, tools: &[String]) -> Entity {
    let attrs = [
        ("enabled".into(), RestrictedExpression::new_bool(true)),
        ("model_group".into(), RestrictedExpression::new_string("primary".into())),
        ("tools".into(), tools_to_set(tools)),
        ("handle".into(), RestrictedExpression::new_string(agent_handle.to_string())),
    ];
    let mut parents = HashSet::new();
    parents.insert(user_entity_uid_from_handle(user_handle));
    Entity::new(
        agent_entity_uid(user_handle, agent_handle),
        attrs.into_iter().collect(),
        parents,
    )
    .expect("valid agent principal entity")
}

pub fn build_agent_principal_entity_for_id(id: &str, tools: &[String]) -> Entity {
    let (parents, handle) = match id.split_once('/') {
        Some((username, handle)) => {
            let mut p = HashSet::new();
            p.insert(user_entity_uid(username));
            (p, handle.to_string())
        }
        None => (HashSet::new(), id.to_string()),
    };
    let attrs = [
        ("enabled".into(), RestrictedExpression::new_bool(true)),
        ("model_group".into(), RestrictedExpression::new_string("primary".into())),
        ("tools".into(), tools_to_set(tools)),
        ("handle".into(), RestrictedExpression::new_string(handle)),
    ];
    Entity::new(entity_uid("Agent", id), attrs.into_iter().collect(), parents)
        .expect("valid agent principal entity")
}

pub fn build_mcp_principal_entity(user_handle: &Handle, mcp_handle: &Handle) -> Entity {
    let mut parents = HashSet::new();
    parents.insert(user_entity_uid_from_handle(user_handle));
    Entity::new_no_attrs(mcp_entity_uid(user_handle, mcp_handle), parents)
}

pub fn build_mcp_principal_entity_for_id(id: &str) -> Entity {
    let parents = match id.split_once('/') {
        Some((user_handle, _)) => {
            let mut p = HashSet::new();
            p.insert(user_entity_uid(user_handle));
            p
        }
        None => HashSet::new(),
    };
    Entity::new_no_attrs(entity_uid("Mcp", id), parents)
}

pub fn build_app_principal_entity(user_handle: &Handle, app_handle: &Handle) -> Entity {
    let mut parents = HashSet::new();
    parents.insert(user_entity_uid_from_handle(user_handle));
    Entity::new_no_attrs(app_entity_uid(user_handle, app_handle), parents)
}

pub fn build_app_principal_entity_for_id(id: &str) -> Entity {
    let parents = match id.split_once('/') {
        Some((user_handle, _)) => {
            let mut p = HashSet::new();
            p.insert(user_entity_uid(user_handle));
            p
        }
        None => HashSet::new(),
    };
    Entity::new_no_attrs(entity_uid("App", id), parents)
}

pub fn tool_entity_uid(tool_name: &str) -> EntityUid {
    entity_uid("Tool", tool_name)
}

pub fn action_entity_uid(action_name: &str) -> EntityUid {
    entity_uid("Action", action_name)
}

fn tool_group_entity_uid(group: &str) -> EntityUid {
    entity_uid("ToolGroup", group)
}

pub fn build_tool_entities(tool_name: &str, tool_group: &str) -> Entities {
    let tool_uid = tool_entity_uid(tool_name);
    let group_uid = tool_group_entity_uid(tool_group);

    let tool_entity = cedar_policy::Entity::new_no_attrs(
        tool_uid,
        HashSet::from([group_uid.clone()]),
    );
    let group_entity = cedar_policy::Entity::new_no_attrs(
        group_uid,
        HashSet::new(),
    );

    Entities::from_entities([tool_entity, group_entity], None)
        .unwrap_or_else(|_| Entities::empty())
}

/// Both agents always share an owner — cross-user delegation isn't valid.
pub fn build_agent_entities(
    user_handle: &Handle,
    principal_handle: &Handle,
    principal_tools: &[String],
    target_handle: &Handle,
    target_tools: &[String],
) -> Entities {
    let principal_entity = build_agent_principal_entity(user_handle, principal_handle, principal_tools);
    let target_entity = build_agent_principal_entity(user_handle, target_handle, target_tools);
    Entities::from_entities([principal_entity, target_entity], None)
        .unwrap_or_else(|_| Entities::empty())
}

pub fn channel_entity_uid(user_handle: &Handle, channel_handle: &Handle) -> EntityUid {
    entity_uid("Channel", &format!("{user_handle}/{channel_handle}"))
}

pub fn contact_entity_uid(contact_id: &str) -> EntityUid {
    entity_uid("Contact", contact_id)
}

pub fn message_source_entity_uid(connector_id: &str, address: &str) -> EntityUid {
    let id = format!("{}:{}", connector_id, address);
    entity_uid("MessageSource", &id)
}

pub fn build_message_source_entities(
    user_handle: &Handle,
    agent_handle: &Handle,
    agent_tools: &[String],
    connector_id: &str,
    channel_handle: &Handle,
    sender: &super::models::PolicyContact,
) -> Entities {
    let principal_entity = build_agent_principal_entity(user_handle, agent_handle, agent_tools);

    let channel_uid = channel_entity_uid(user_handle, channel_handle);
    let channel_entity = Entity::new_no_attrs(
        channel_uid.clone(),
        HashSet::from([user_entity_uid_from_handle(user_handle)]),
    );

    let user_uid = user_entity_uid(&sender.user_id);
    let user_entity = Entity::new_no_attrs(user_uid.clone(), HashSet::new());

    let contact_uid = contact_entity_uid(&sender.id);
    let contact_addresses = RestrictedExpression::new_set(
        sender
            .addresses
            .iter()
            .cloned()
            .map(RestrictedExpression::new_string),
    );
    let contact_attrs = [
        (
            "address".into(),
            RestrictedExpression::new_string(sender.address.clone()),
        ),
        ("addresses".into(), contact_addresses),
        (
            "name".into(),
            RestrictedExpression::new_string(sender.name.clone()),
        ),
    ];
    let contact_entity = Entity::new(
        contact_uid.clone(),
        contact_attrs.into_iter().collect(),
        HashSet::from([user_uid.clone()]),
    )
    .expect("valid contact entity");

    let source_attrs = [
        (
            "connector_id".into(),
            RestrictedExpression::new_string(connector_id.to_string()),
        ),
        (
            "sender".into(),
            RestrictedExpression::new_entity_uid(contact_uid),
        ),
        (
            "user".into(),
            RestrictedExpression::new_entity_uid(user_uid),
        ),
    ];
    let resource_entity = Entity::new(
        message_source_entity_uid(connector_id, &sender.address),
        source_attrs.into_iter().collect(),
        HashSet::from([channel_uid]),
    )
    .expect("valid message source entity");

    Entities::from_entities(
        [
            principal_entity,
            channel_entity,
            user_entity,
            contact_entity,
            resource_entity,
        ],
        None,
    )
    .unwrap_or_else(|_| Entities::empty())
}


pub fn prepend_annotations(id: &str, description: &str, policy_text: &str) -> String {
    format!("@id(\"{id}\")\n@description(\"{description}\")\n{policy_text}")
}

fn resource_to_cedar_clause(resource: &super::models::PolicyResource) -> String {
    match resource {
        super::models::PolicyResource::Tool { id, .. } => {
            format!("resource == {NAMESPACE}::Tool::\"{id}\"")
        }
        super::models::PolicyResource::ToolGroup { group } => {
            format!("resource in {NAMESPACE}::ToolGroup::\"{group}\"")
        }
    }
}

pub fn build_tool_policy_text(
    user_handle: &Handle,
    agent_handle: &Handle,
    resource: &super::models::PolicyResource,
    effect: &str,
    policy_name: &str,
    description: &str,
) -> String {
    let resource_cedar = resource_to_cedar_clause(resource);
    let agent_id = format!("{user_handle}/{agent_handle}");
    format!(
        "@id(\"{policy_name}\")\n@description(\"{description}\")\n{effect}(\n  principal == {NAMESPACE}::Agent::\"{agent_id}\",\n  action == {NAMESPACE}::Action::\"invoke_tool\",\n  {resource_cedar}\n);"
    )
}

pub fn references_agent(policy_text: &str, user_handle: &Handle, agent_handle: &Handle) -> bool {
    let Ok(policy_set) = PolicySet::from_str(policy_text) else {
        return false;
    };
    let target = agent_entity_uid(user_handle, agent_handle);

    policy_set.policies().any(|p| {
        matches!(
            p.principal_constraint(),
            cedar_policy::PrincipalConstraint::Eq(ref uid) if *uid == target
        ) || matches!(
            p.resource_constraint(),
            cedar_policy::ResourceConstraint::Eq(ref uid) if *uid == target
        )
    })
}

pub fn extract_annotations(policy_text: &str) -> (Option<String>, Option<String>) {
    let Ok(policy_set) = PolicySet::from_str(policy_text) else {
        return (None, None);
    };

    let first = policy_set.policies().next();
    let Some(policy) = first else {
        return (None, None);
    };

    let id = policy.annotation("id").map(|s| s.to_string());
    let description = policy.annotation("description").map(|s| s.to_string());

    (id, description)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_parses_without_error() {
        let schema = build_schema();
        assert!(Arc::strong_count(&schema) == 1);
    }

    #[test]
    fn test_references_agent() {
        let text = "permit(principal == Policy::Agent::\"alice/dev\", action, resource);";
        assert!(references_agent(text, &crate::handle!("alice"), &crate::handle!("dev")));
        assert!(!references_agent(text, &crate::handle!("alice"), &crate::handle!("other")));
        assert!(!references_agent(text, &crate::handle!("bob"), &crate::handle!("dev")));
    }

    #[test]
    fn test_extract_annotations() {
        let text = "@id(\"my-policy\")\n@description(\"A test policy\")\npermit(principal, action, resource);";
        let (id, desc) = extract_annotations(text);
        assert_eq!(id.as_deref(), Some("my-policy"));
        assert_eq!(desc.as_deref(), Some("A test policy"));
    }

    #[test]
    fn test_extract_annotations_none() {
        let text = "permit(principal, action, resource);";
        let (id, desc) = extract_annotations(text);
        assert!(id.is_none());
        assert!(desc.is_none());
    }

    #[test]
    fn schema_validates_default_network_access_managed_policy() {
        let schema = build_schema();
        let policy = cedar_policy::Policy::from_json(
            Some(cedar_policy::PolicyId::new("default-network-access")),
            serde_json::json!({
                "effect": "permit",
                "principal": { "op": "All" },
                "action": { "op": "==", "entity": { "type": "Policy::Action", "id": "connect" } },
                "resource": { "op": "All" },
                "annotations": {},
                "conditions": []
            }),
        )
        .expect("default-network-access policy parses");

        let mut policy_set = cedar_policy::PolicySet::new();
        policy_set.add(policy).expect("add policy to set");

        let validator = cedar_policy::Validator::new((*schema).clone());
        let result = validator.validate(&policy_set, cedar_policy::ValidationMode::default());
        assert!(
            result.validation_passed(),
            "default-network-access must validate against the schema, got: {:?}",
            result.validation_errors().collect::<Vec<_>>()
        );
    }

    #[test]
    fn schema_validates_mcp_principal() {
        let schema = build_schema();
        let text = r#"permit(principal == Policy::Mcp::"x", action == Policy::Action::"connect", resource);"#;
        let policy_set = cedar_policy::PolicySet::from_str(text).expect("parse");
        let validator = cedar_policy::Validator::new((*schema).clone());
        let result = validator.validate(&policy_set, cedar_policy::ValidationMode::default());
        assert!(
            result.validation_passed(),
            "Mcp connect policy must validate, got: {:?}",
            result.validation_errors().collect::<Vec<_>>()
        );
    }

    #[test]
    fn schema_validates_app_principal() {
        let schema = build_schema();
        let text = r#"permit(principal == Policy::App::"x", action == Policy::Action::"read", resource == Policy::Path::"/data");"#;
        let policy_set = cedar_policy::PolicySet::from_str(text).expect("parse");
        let validator = cedar_policy::Validator::new((*schema).clone());
        let result = validator.validate(&policy_set, cedar_policy::ValidationMode::default());
        assert!(
            result.validation_passed(),
            "App read policy must validate, got: {:?}",
            result.validation_errors().collect::<Vec<_>>()
        );
    }
}
