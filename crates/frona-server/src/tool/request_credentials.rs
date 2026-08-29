use serde_json::Value;

use crate::agent::prompt::PromptLoader;
use crate::core::Principal;
use crate::core::error::AppError;
use crate::credential::vault::models::{BindingScope, GrantDuration};
use crate::credential::vault::service::VaultService;
use crate::inference::hitl::{
    CredentialRequest, Hitl, HitlOutcome, HitlRequest, HitlResponse, VaultGrant,
};
use crate::inference::tool_call::ToolStatus;

use frona_derive::agent_tool;

use super::{InferenceContext, ToolOutput, active_chat};

pub struct RequestCredentialsTool {
    vault_service: VaultService,
    prompts: PromptLoader,
    public_base_url: String,
}

impl RequestCredentialsTool {
    pub fn new(
        vault_service: VaultService,
        prompts: PromptLoader,
        public_base_url: String,
    ) -> Self {
        Self {
            vault_service,
            prompts,
            public_base_url,
        }
    }

    fn scope_for(
        grant_duration: &GrantDuration,
        chat_id: &str,
    ) -> (BindingScope, Option<chrono::DateTime<chrono::Utc>>) {
        match grant_duration {
            GrantDuration::Once => (
                BindingScope::Chat {
                    chat_id: chat_id.to_string(),
                },
                None,
            ),
            GrantDuration::Hours(h) => (
                BindingScope::Durable,
                Some(chrono::Utc::now() + chrono::Duration::hours(*h as i64)),
            ),
            GrantDuration::Days(d) => (
                BindingScope::Durable,
                Some(chrono::Utc::now() + chrono::Duration::days(*d as i64)),
            ),
            GrantDuration::Permanent => (BindingScope::Durable, None),
        }
    }

    /// Normalize the tool arguments into the list of credentials the agent
    /// wants. Accepts a single `query` string (legacy) and/or a `queries`
    /// array whose elements are either plain strings or `{query, label}`
    /// objects, so the agent can ask for every secret an API needs in one call.
    /// Duplicate queries collapse to a single slot.
    fn parse_requested(arguments: &Value) -> Result<Vec<CredentialRequest>, AppError> {
        let mut items: Vec<CredentialRequest> = Vec::new();

        let mut push = |query: &str, label: Option<String>| {
            let query = query.trim();
            if query.is_empty() {
                return;
            }
            if items.iter().any(|i| i.query == query) {
                return;
            }
            items.push(CredentialRequest { query: query.to_string(), label });
        };

        if let Some(arr) = arguments.get("queries").and_then(|v| v.as_array()) {
            for el in arr {
                if let Some(s) = el.as_str() {
                    push(s, None);
                } else if let Some(obj) = el.as_object() {
                    if let Some(q) = obj.get("query").and_then(|v| v.as_str()) {
                        let label = obj
                            .get("label")
                            .and_then(|v| v.as_str())
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty());
                        push(q, label);
                    }
                }
            }
        }

        if let Some(q) = arguments.get("query").and_then(|v| v.as_str()) {
            push(q, None);
        }

        if items.is_empty() {
            return Err(AppError::Validation(
                "Missing required parameter: provide `query` or a non-empty `queries` array".into(),
            ));
        }
        Ok(items)
    }

    fn batch_prompt(items: &[CredentialRequest], reason: &str) -> String {
        if items.len() == 1 {
            return format!(
                "Allow access to credential matching '{}'?\n\n{reason}",
                items[0].query
            );
        }
        let lines: Vec<String> = items
            .iter()
            .map(|i| match &i.label {
                Some(label) => format!("• {label} — matching '{}'", i.query),
                None => format!("• matching '{}'", i.query),
            })
            .collect();
        format!(
            "Allow access to {} credentials?\n\n{reason}\n\n{}",
            items.len(),
            lines.join("\n"),
        )
    }
}

#[agent_tool]
impl RequestCredentialsTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let chat = active_chat(ctx)?;
        let reason = arguments
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("Credential access requested")
            .to_string();

        let force = arguments
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let requested = Self::parse_requested(&arguments)?;
        let principal = Principal::agent(ctx.agent.id.clone());

        // Split the batch into items an existing grant already covers and those
        // that still need the user's approval. Only prompt for the remainder,
        // so re-runs (or partially-granted batches) don't re-ask for secrets
        // the chat already has.
        let mut satisfied: Vec<(CredentialRequest, _)> = Vec::new();
        // Owner-delegated items on a shared agent: (request, owner's binding,
        // owner id). Loaded from the owner's vault, never prompted.
        let mut delegated: Vec<(CredentialRequest, _, String)> = Vec::new();
        let mut pending: Vec<CredentialRequest> = Vec::new();
        for item in requested {
            let binding = if force {
                None
            } else {
                self.vault_service
                    .find_binding(&ctx.user.id, &principal, &item.query, Some(&chat.id))
                    .await?
            };
            if let Some(b) = binding {
                satisfied.push((item, b));
                continue;
            }
            // Shared agent with credential delegation: fall back to the owner's
            // durable binding for the same agent principal.
            if !force
                && let Some(owner_id) = ctx.delegated_credential_owner.as_deref()
            {
                let owner_binding = self
                    .vault_service
                    .find_binding(owner_id, &principal, &item.query, None)
                    .await?;
                if let Some(ob) = owner_binding {
                    delegated.push((item, ob, owner_id.to_string()));
                    continue;
                }
            }
            pending.push(item);
        }

        // When nothing needs approval, load every already-granted secret and
        // return immediately without pausing the turn.
        if pending.is_empty() {
            let mut var_names: Vec<String> = Vec::new();
            for (item, binding) in satisfied {
                let secret = self
                    .vault_service
                    .get_secret(&ctx.user.id, &binding.connection_id, &binding.vault_item_id)
                    .await?;

                self.vault_service
                    .log_access(
                        &ctx.user.id,
                        principal.clone(),
                        &chat.id,
                        &binding.connection_id,
                        &binding.vault_item_id,
                        None,
                        &item.query,
                        &reason,
                    )
                    .await?;

                let env_vars =
                    crate::credential::vault::service::project_target(&secret, &binding.target);
                var_names.extend(env_vars.iter().map(|(k, _)| k.clone()));
                let mut vault_vars = ctx.vault_env_vars.write().await;
                vault_vars.extend(env_vars);
            }

            // Owner-delegated secrets are read from the OWNER's vault and logged
            // under the owner (for their audit). No approval — the owner opted
            // into delegation when sharing.
            for (item, binding, owner_id) in delegated {
                let secret = self
                    .vault_service
                    .get_secret(&owner_id, &binding.connection_id, &binding.vault_item_id)
                    .await?;

                self.vault_service
                    .log_access(
                        &owner_id,
                        principal.clone(),
                        &chat.id,
                        &binding.connection_id,
                        &binding.vault_item_id,
                        None,
                        &item.query,
                        &reason,
                    )
                    .await?;

                let env_vars =
                    crate::credential::vault::service::project_target(&secret, &binding.target);
                var_names.extend(env_vars.iter().map(|(k, _)| k.clone()));
                let mut vault_vars = ctx.vault_env_vars.write().await;
                vault_vars.extend(env_vars);
            }

            return Ok(ToolOutput::text(format!(
                "Credentials loaded into environment variables: {}. Use these in CLI commands.",
                var_names.join(", ")
            )));
        }

        // Anything already-granted will be re-hydrated on resume by the session
        // builder, so we only carry the pending items into the approval. The
        // user fills every slot in one interaction.
        let prompt = Self::batch_prompt(&pending, &reason);
        Ok(ToolOutput::text("").with_hitl(Hitl {
            prompt,
            url: format!("{}/chat?id={}", self.public_base_url, chat.id),
            request: HitlRequest::Credentials { items: pending, reason },
            status: ToolStatus::Pending,
            response: None,
            delivery: None,
        }))
    }

    async fn on_resume(
        &self,
        _tool_name: &str,
        request: &HitlRequest,
        response: HitlResponse,
        ctx: &InferenceContext,
    ) -> Result<HitlOutcome, AppError> {
        let chat = active_chat(ctx)?;
        let reason = match request {
            HitlRequest::Credential { reason, .. } | HitlRequest::Credentials { reason, .. } => {
                reason.clone()
            }
            _ => {
                return Err(AppError::Validation(
                    "request_credentials on_resume: expected Credential(s) request".into(),
                ));
            }
        };

        let principal = Principal::agent(ctx.agent.id.clone());

        match response {
            HitlResponse::Vault(VaultGrant::GrantedMany { grants }) => {
                let mut var_names: Vec<String> = Vec::new();
                for grant in grants {
                    let secret = self
                        .vault_service
                        .get_secret(&ctx.user.id, &grant.connection_id, &grant.vault_item_id)
                        .await?;

                    if !matches!(grant.grant_duration, GrantDuration::Once) {
                        self.vault_service
                            .create_grant(
                                &ctx.user.id,
                                principal.clone(),
                                &grant.connection_id,
                                &grant.vault_item_id,
                                &grant.query,
                                &grant.grant_duration,
                            )
                            .await?;
                    }

                    let (scope, expires_at) = Self::scope_for(&grant.grant_duration, &chat.id);

                    self.vault_service
                        .create_binding(
                            &ctx.user.id,
                            principal.clone(),
                            &grant.query,
                            &grant.connection_id,
                            &grant.vault_item_id,
                            grant.target.clone(),
                            scope,
                            expires_at,
                        )
                        .await?;

                    self.vault_service
                        .log_access(
                            &ctx.user.id,
                            principal.clone(),
                            &chat.id,
                            &grant.connection_id,
                            &grant.vault_item_id,
                            None,
                            &grant.query,
                            &reason,
                        )
                        .await?;

                    let env_vars =
                        crate::credential::vault::service::project_target(&secret, &grant.target);
                    var_names.extend(env_vars.iter().map(|(k, _)| k.clone()));
                    let mut vault_vars = ctx.vault_env_vars.write().await;
                    vault_vars.extend(env_vars);
                }

                Ok(HitlOutcome::Resolved(format!(
                    "Credentials loaded into environment variables: {}. Use these in CLI commands.",
                    var_names.join(", "),
                )))
            }
            HitlResponse::Vault(VaultGrant::Granted {
                connection_id,
                vault_item_id,
                grant_duration,
                target,
            }) => {
                // Legacy single-item path — only a `Credential` request carries
                // the query this grant resolves.
                let HitlRequest::Credential { query, .. } = request else {
                    return Err(AppError::Validation(
                        "request_credentials on_resume: single Granted response requires a Credential request".into(),
                    ));
                };

                let secret = self
                    .vault_service
                    .get_secret(&ctx.user.id, &connection_id, &vault_item_id)
                    .await?;

                if !matches!(grant_duration, GrantDuration::Once) {
                    self.vault_service
                        .create_grant(
                            &ctx.user.id,
                            principal.clone(),
                            &connection_id,
                            &vault_item_id,
                            query,
                            &grant_duration,
                        )
                        .await?;
                }

                let (scope, expires_at) = Self::scope_for(&grant_duration, &chat.id);

                self.vault_service
                    .create_binding(
                        &ctx.user.id,
                        principal.clone(),
                        query,
                        &connection_id,
                        &vault_item_id,
                        target.clone(),
                        scope,
                        expires_at,
                    )
                    .await?;

                self.vault_service
                    .log_access(
                        &ctx.user.id,
                        principal,
                        &chat.id,
                        &connection_id,
                        &vault_item_id,
                        None,
                        query,
                        &reason,
                    )
                    .await?;

                let env_vars = crate::credential::vault::service::project_target(&secret, &target);
                let var_names: Vec<String> = env_vars.iter().map(|(k, _)| k.clone()).collect();
                let mut vault_vars = ctx.vault_env_vars.write().await;
                vault_vars.extend(env_vars);

                Ok(HitlOutcome::Resolved(format!(
                    "Credentials loaded into environment variables: {}. Use these in CLI commands.",
                    var_names.join(", "),
                )))
            }
            HitlResponse::Vault(VaultGrant::Denied) => {
                let label = match request {
                    HitlRequest::Credential { query, .. } => query.clone(),
                    HitlRequest::Credentials { items, .. } => items
                        .iter()
                        .map(|i| i.query.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                    _ => "requested credentials".to_string(),
                };
                Ok(HitlOutcome::Denied(format!(
                    "User denied access to credentials for: {label}.",
                )))
            }
            _ => Err(AppError::Validation(
                "request_credentials on_resume: expected Vault response".into(),
            )),
        }
    }
}
