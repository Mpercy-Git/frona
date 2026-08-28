//! HITL types and resolve dispatcher. Channel-agnostic; rendering and
//! callback parsing live in `chat::channel::hitl`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

use crate::credential::vault::models::GrantDuration;
use crate::inference::tool_call::ToolStatus;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct Hitl {
    pub prompt: String,
    /// Web frontend fallback URL — channels that can't render the affordance
    /// natively post this so the user can resolve via web.
    pub url: String,
    pub request: HitlRequest,
    pub status: ToolStatus,
    /// `None` iff `status == Pending`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<HitlResponse>,
    /// Delivery cursor uses this for retry idempotency (skips already-rendered
    /// HITLs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<HitlDelivery>,
}

/// Channels project this to `HitlKind` via `chat::channel::hitl::kind_for`
/// rather than matching directly, so new variants only need a `kind_for` arm.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[serde(tag = "type", content = "data")]
#[surreal(crate = "surrealdb::types", tag = "type", content = "data")]
pub enum HitlRequest {
    Question { options: Vec<String> },
    Takeover {
        reason: String,
        debugger_url: String,
    },
    App {
        action: String,
        manifest: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_manifest: Option<serde_json::Value>,
    },
    Credential {
        query: String,
        reason: String,
    },
    /// Batched credential request: the agent needs several secrets at once
    /// (e.g. an app key *and* a user key). Rendered as a single approval with
    /// one slot per item so the user provides them all in one interaction
    /// instead of the agent asking for one, resuming, then asking for the next.
    Credentials {
        items: Vec<CredentialRequest>,
        reason: String,
    },
    /// The agent found skills in the registry that aren't installed and wants
    /// to add them. Nothing is written until the user approves — the install
    /// happens in `add_skill`'s `on_resume`, mirroring the vault flow where
    /// the secret is only read after the grant.
    Skills {
        items: Vec<SkillCandidate>,
        scope: SkillInstallScope,
        reason: String,
    },
}

/// One skill in a [`HitlRequest::Skills`] approval. Carries enough for the
/// user to judge the install without leaving the chat, and enough for
/// `on_resume` to fetch it without re-searching the registry.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct SkillCandidate {
    pub name: String,
    /// `owner/repo` on GitHub the skill is installed from.
    pub repo: String,
    #[serde(default)]
    pub description: String,
}

/// Where an approved skill lands. `Agent` writes into the agent's own
/// workspace (only that agent sees it); `User` writes into the user's skill
/// directory (every agent of theirs can use it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "snake_case")]
#[surreal(crate = "surrealdb::types")]
pub enum SkillInstallScope {
    Agent,
    User,
}

impl SkillInstallScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::User => "user",
        }
    }
}

/// One secret in a batched [`HitlRequest::Credentials`]. `label` is an optional
/// human hint shown beside the slot so a user filling several at once can tell
/// them apart (e.g. "App key" vs "User key").
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct CredentialRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Channels can only emit `Approval` or `Choice` — the shapes a button tap
/// or text reply can carry. Variants beyond those are web-frontend-only.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[serde(tag = "type", content = "data")]
#[surreal(crate = "surrealdb::types", tag = "type", content = "data")]
pub enum HitlResponse {
    Approval(bool),
    Choice(String),
    Vault(VaultGrant),
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[serde(tag = "type", content = "data")]
#[surreal(crate = "surrealdb::types", tag = "type", content = "data")]
pub enum VaultGrant {
    Granted {
        connection_id: String,
        vault_item_id: String,
        grant_duration: GrantDuration,
        target: crate::credential::vault::models::CredentialTarget,
    },
    /// Resolution for a batched [`HitlRequest::Credentials`] — one grant per
    /// item the user filled in. A single `Denied` still denies the whole batch.
    GrantedMany {
        grants: Vec<VaultItemGrant>,
    },
    Denied,
}

/// One resolved item of a [`VaultGrant::GrantedMany`], tying a requested
/// `query` to the vault item and binding the user chose for it.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct VaultItemGrant {
    pub query: String,
    pub connection_id: String,
    pub vault_item_id: String,
    pub grant_duration: GrantDuration,
    pub target: crate::credential::vault::models::CredentialTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct HitlDelivery {
    pub channel_id: String,
    /// Provider-specific (Telegram `message_id`, SMS `MessageSid`, etc.).
    /// Used for editing the original prompt on resolution.
    pub external_message_id: String,
    pub delivered_at: DateTime<Utc>,
}

/// Synthesized result text persisted as `te.result` — what the LLM sees on
/// resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitlOutcome {
    Resolved(String),
    Denied(String),
}

/// `should_resume` is true iff the per-message barrier cleared (no more
/// pending HITLs). `task_id` is `Some` when the chat is a task chat — the
/// dispatcher uses it to choose `task_executor.run_task` over
/// `harness.resume`. `AlreadyResolved` is idempotent — callers can render
/// "already resolved" UX without raising an error.
#[derive(Debug, Clone)]
pub enum ResolveOutcome {
    Resolved {
        should_resume: bool,
        user_id: String,
        chat_id: String,
        message_id: String,
        task_id: Option<String>,
    },
    AlreadyResolved,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential::vault::models::GrantDuration;

    #[test]
    fn hitl_request_question_round_trip() {
        let req = HitlRequest::Question {
            options: vec!["a".into(), "b".into()],
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: HitlRequest = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, HitlRequest::Question { options } if options == vec!["a", "b"]));
    }

    #[test]
    fn hitl_request_takeover_round_trip() {
        let req = HitlRequest::Takeover {
            reason: "manual debug".into(),
            debugger_url: "https://example/d/1".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: HitlRequest = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            HitlRequest::Takeover { ref reason, ref debugger_url }
            if reason == "manual debug" && debugger_url == "https://example/d/1"
        ));
    }

    #[test]
    fn hitl_request_service_approval_round_trip() {
        let req = HitlRequest::App {
            action: "deploy".into(),
            manifest: serde_json::json!({"handle": "notes", "name": "Notes"}),
            previous_manifest: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: HitlRequest = serde_json::from_str(&json).unwrap();
        match back {
            HitlRequest::App { action, manifest, .. } => {
                assert_eq!(action, "deploy");
                assert_eq!(manifest["handle"], "notes");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn hitl_request_vault_pick_round_trip() {
        let req = HitlRequest::Credential {
            query: "postgres-prod".into(),
            reason: "ETL job".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: HitlRequest = serde_json::from_str(&json).unwrap();
        match back {
            HitlRequest::Credential { query, reason } => {
                assert_eq!(query, "postgres-prod");
                assert_eq!(reason, "ETL job");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn hitl_request_credentials_round_trip() {
        let req = HitlRequest::Credentials {
            items: vec![
                CredentialRequest { query: "acme app key".into(), label: Some("App key".into()) },
                CredentialRequest { query: "acme user key".into(), label: None },
            ],
            reason: "Call the Acme API".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: HitlRequest = serde_json::from_str(&json).unwrap();
        match back {
            HitlRequest::Credentials { items, reason } => {
                assert_eq!(reason, "Call the Acme API");
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].query, "acme app key");
                assert_eq!(items[0].label.as_deref(), Some("App key"));
                assert_eq!(items[1].label, None);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn hitl_request_skills_round_trip() {
        let req = HitlRequest::Skills {
            items: vec![
                SkillCandidate {
                    name: "pdf".into(),
                    repo: "anthropics/skills".into(),
                    description: "Fill and merge PDF files.".into(),
                },
                SkillCandidate {
                    name: "xlsx".into(),
                    repo: "anthropics/skills".into(),
                    description: String::new(),
                },
            ],
            scope: SkillInstallScope::Agent,
            reason: "The user asked for a filled-in PDF form.".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: HitlRequest = serde_json::from_str(&json).unwrap();
        match back {
            HitlRequest::Skills { items, scope, reason } => {
                assert_eq!(scope, SkillInstallScope::Agent);
                assert_eq!(reason, "The user asked for a filled-in PDF form.");
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].name, "pdf");
                assert_eq!(items[0].repo, "anthropics/skills");
                assert_eq!(items[0].description, "Fill and merge PDF files.");
                assert_eq!(items[1].description, "");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn skill_install_scope_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&SkillInstallScope::User).unwrap(),
            "\"user\""
        );
        assert_eq!(SkillInstallScope::Agent.as_str(), "agent");
    }

    #[test]
    fn hitl_response_vault_granted_many_round_trip() {
        use crate::credential::vault::models::CredentialTarget;
        let r = HitlResponse::Vault(VaultGrant::GrantedMany {
            grants: vec![VaultItemGrant {
                query: "acme app key".into(),
                connection_id: "conn-1".into(),
                vault_item_id: "item-1".into(),
                grant_duration: GrantDuration::Once,
                target: CredentialTarget::Single {
                    env_var: "ACME_APP_KEY".into(),
                    field: crate::credential::vault::models::VaultField::Password,
                },
            }],
        });
        let json = serde_json::to_string(&r).unwrap();
        let back: HitlResponse = serde_json::from_str(&json).unwrap();
        match back {
            HitlResponse::Vault(VaultGrant::GrantedMany { grants }) => {
                assert_eq!(grants.len(), 1);
                assert_eq!(grants[0].query, "acme app key");
                assert_eq!(grants[0].connection_id, "conn-1");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn hitl_response_approval_round_trip() {
        let r = HitlResponse::Approval(true);
        let json = serde_json::to_string(&r).unwrap();
        let back: HitlResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, HitlResponse::Approval(true)));
    }

    #[test]
    fn hitl_response_choice_round_trip() {
        let r = HitlResponse::Choice("staging".into());
        let json = serde_json::to_string(&r).unwrap();
        let back: HitlResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, HitlResponse::Choice(s) if s == "staging"));
    }

    #[test]
    fn hitl_response_vault_granted_round_trip() {
        use crate::credential::vault::models::CredentialTarget;
        let r = HitlResponse::Vault(VaultGrant::Granted {
            connection_id: "conn-1".into(),
            vault_item_id: "item-1".into(),
            grant_duration: GrantDuration::Once,
            target: CredentialTarget::Prefix { env_var_prefix: "DB".into() },
        });
        let json = serde_json::to_string(&r).unwrap();
        let back: HitlResponse = serde_json::from_str(&json).unwrap();
        match back {
            HitlResponse::Vault(VaultGrant::Granted {
                connection_id,
                vault_item_id,
                target,
                ..
            }) => {
                use crate::credential::vault::models::CredentialTarget;
                assert_eq!(connection_id, "conn-1");
                assert_eq!(vault_item_id, "item-1");
                match target {
                    CredentialTarget::Prefix { env_var_prefix } => assert_eq!(env_var_prefix, "DB"),
                    _ => panic!("expected Prefix target"),
                }
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn hitl_response_vault_denied_round_trip() {
        let r = HitlResponse::Vault(VaultGrant::Denied);
        let json = serde_json::to_string(&r).unwrap();
        let back: HitlResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, HitlResponse::Vault(VaultGrant::Denied)));
    }

    #[test]
    fn hitl_delivery_round_trip() {
        let d = HitlDelivery {
            channel_id: "ch-1".into(),
            external_message_id: "42".into(),
            delivered_at: Utc::now(),
        };
        let json = serde_json::to_string(&d).expect("serialize");
        let back: HitlDelivery = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.channel_id, "ch-1");
        assert_eq!(back.external_message_id, "42");
    }

    #[test]
    fn hitl_struct_with_response_and_delivery_round_trip() {
        let h = Hitl {
            prompt: "Pick a region?".into(),
            url: "https://app/chats/abc".into(),
            request: HitlRequest::Question {
                options: vec!["us".into(), "eu".into()],
            },
            status: ToolStatus::Resolved,
            response: Some(HitlResponse::Choice("us".into())),
            delivery: Some(HitlDelivery {
                channel_id: "ch-1".into(),
                external_message_id: "42".into(),
                delivered_at: Utc::now(),
            }),
        };
        let json = serde_json::to_string(&h).expect("serialize");
        let back: Hitl = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.prompt, "Pick a region?");
        assert!(matches!(back.status, ToolStatus::Resolved));
        assert!(back.response.is_some());
        assert!(back.delivery.is_some());
    }

}
