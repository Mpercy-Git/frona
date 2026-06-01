use serde_json::Value;

use crate::agent::prompt::PromptLoader;
use crate::inference::tool_call::{MessageTool, ToolStatus};
use crate::core::error::AppError;
use crate::credential::vault::service::VaultService;

use frona_derive::agent_tool;

use super::{InferenceContext, ToolOutput};

pub struct RequestCredentialsTool {
    vault_service: VaultService,
    prompts: PromptLoader,
}

impl RequestCredentialsTool {
    pub fn new(vault_service: VaultService, prompts: PromptLoader) -> Self {
        Self {
            vault_service,
            prompts,
        }
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
        let query = arguments
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation("Missing required parameter: query".into()))?
            .to_string();

        let reason = arguments
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("Credential access requested")
            .to_string();

        let env_var_prefix = arguments
            .get("env_var_prefix")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AppError::Validation("Missing required parameter: env_var_prefix".into())
            })?
            .to_string();

        let force = arguments
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let principal = crate::core::Principal::agent(ctx.agent.id.clone());
        if !force
            && let Some(binding) = self
                .vault_service
                .find_binding(&ctx.user.id, &principal, &query, Some(&ctx.chat.id))
                .await?
        {
            let secret = self
                .vault_service
                .get_secret(&ctx.user.id, &binding.connection_id, &binding.vault_item_id)
                .await?;

            self.vault_service
                .log_access(
                    &ctx.user.id,
                    principal.clone(),
                    &ctx.chat.id,
                    &binding.connection_id,
                    &binding.vault_item_id,
                    Some(&env_var_prefix),
                    &query,
                    &reason,
                )
                .await?;

            let env_vars =
                crate::credential::vault::service::project_target(&secret, &binding.target);
            let var_names: Vec<String> =
                env_vars.iter().map(|(k, _)| k.clone()).collect();
            let mut vault_vars = ctx.vault_env_vars.write().await;
            vault_vars.extend(env_vars);

            return Ok(ToolOutput::text(format!(
                "Credentials loaded into environment variables: {}. Use these in CLI commands.",
                var_names.join(", ")
            )));
        }

        let json = serde_json::json!({
            "tool_type": "VaultApproval",
            "query": query,
            "reason": reason,
            "env_var_prefix": env_var_prefix,
        });

        Ok(ToolOutput::text(json.to_string())
            .with_tool_data(MessageTool::VaultApproval {
                query,
                reason,
                env_var_prefix: Some(env_var_prefix),
                status: ToolStatus::Pending,
                response: None,
            })
            .as_pending_external())
    }
}
