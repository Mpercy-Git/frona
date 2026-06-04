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
    Denied,
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
/// pending HITLs). `AlreadyResolved` is idempotent — callers can render
/// "already resolved" UX without raising an error.
#[derive(Debug, Clone)]
pub enum ResolveOutcome {
    Resolved {
        should_resume: bool,
        user_id: String,
        chat_id: String,
        message_id: String,
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
