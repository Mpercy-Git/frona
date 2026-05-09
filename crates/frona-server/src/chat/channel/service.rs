use std::sync::Arc;

use chrono::{Duration, Utc};
use rand::Rng;
use uuid::Uuid;

use crate::chat::broadcast::{BroadcastService, EntityAction};
use crate::core::config::Config;
use crate::core::error::AppError;
use crate::core::principal::Principal;
use crate::credential::vault::models::{BindingScope, CredentialTarget};
use crate::credential::vault::service::{VaultService, project_target};
use crate::tool::mcp::models::CredentialBinding;

use super::models::{
    Channel, CreateChannelRequest, ChannelManifest, ChannelStatus, ConfigRef, UpdateChannelRequest,
    UserAddress,
};
use super::registry::ChannelRegistry;
use super::repository::ChannelRepository;

pub fn resolve_config_default(config: &Config, r: &ConfigRef) -> Option<String> {
    let v = serde_json::to_value(config).ok()?;
    v.get(&r.section)?
        .get(&r.field)?
        .as_str()
        .map(|s| s.to_string())
}

pub struct ChannelService {
    repo: Arc<dyn ChannelRepository>,
    registry: Arc<ChannelRegistry>,
    vault: Arc<VaultService>,
    broadcast: BroadcastService,
    config: Arc<Config>,
}

impl ChannelService {
    pub fn new(
        repo: Arc<dyn ChannelRepository>,
        registry: Arc<ChannelRegistry>,
        vault: Arc<VaultService>,
        broadcast: BroadcastService,
        config: Arc<Config>,
    ) -> Self {
        Self { repo, registry, vault, broadcast, config }
    }

    pub async fn list_for_user(&self, user_id: &str) -> Result<Vec<Channel>, AppError> {
        self.repo.find_by_user(user_id).await
    }

    pub async fn find_by_space(&self, space_id: &str) -> Result<Option<Channel>, AppError> {
        self.repo.find_by_space(space_id).await
    }

    pub async fn find_active(&self) -> Result<Vec<Channel>, AppError> {
        self.repo.find_active().await
    }

    pub async fn find_by_id(&self, channel_id: &str) -> Result<Channel, AppError> {
        self.repo
            .find_by_id(channel_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("channel {channel_id} not found")))
    }

    pub async fn find_owned(&self, user_id: &str, channel_id: &str) -> Result<Channel, AppError> {
        let channel = self.find_by_id(channel_id).await?;
        if channel.user_id != user_id {
            return Err(AppError::Forbidden("not your channel".into()));
        }
        Ok(channel)
    }

    pub async fn create(
        &self,
        user_id: &str,
        req: CreateChannelRequest,
    ) -> Result<Channel, AppError> {
        let factory = self.registry.get_factory(&req.provider).ok_or_else(|| {
            AppError::Validation(format!("unknown channel provider {:?}", req.provider))
        })?;
        let manifest = factory.manifest();

        if let Some(existing) = self.repo.find_by_space(&req.space_id).await? {
            return Err(AppError::Validation(format!(
                "space {:?} already has a {:?} channel installed — uninstall it before installing {:?}",
                req.space_id, existing.provider, req.provider,
            )));
        }

        let missing = missing_required_fields(&manifest, &req.config, &req.credentials, &self.config);
        let (initial_status, initial_error) = if missing.is_empty() {
            (ChannelStatus::Disconnected, None)
        } else {
            (
                ChannelStatus::Setup,
                Some(format!("missing required field(s): {}", missing.join(", "))),
            )
        };

        let now = Utc::now();
        let channel = Channel {
            id: format!("channel:{}", Uuid::new_v4()),
            user_id: user_id.to_string(),
            space_id: req.space_id,
            provider: req.provider,
            agent_id: req.agent_id,
            config: req.config,
            dispatch_mode: req.dispatch_mode,
            status: initial_status,
            error_message: initial_error,
            last_started_at: None,
            user_address: None,
            created_at: now,
            updated_at: now,
        };
        let persisted = self.repo.create(&channel).await?;

        let principal = Principal::channel(&persisted.id);
        self.verify_grants(user_id, &req.credentials, &principal).await?;
        self.write_bindings(user_id, &persisted.id, req.credentials).await?;

        self.broadcast.broadcast_entity_updated(
            user_id,
            "channel",
            &persisted.id,
            EntityAction::Created,
            Some(persisted.space_id.clone()),
            None,
        );
        Ok(persisted)
    }

    pub async fn update(
        &self,
        user_id: &str,
        channel_id: &str,
        req: UpdateChannelRequest,
    ) -> Result<Channel, AppError> {
        let mut channel = self.find_owned(user_id, channel_id).await?;

        if let Some(agent_id) = req.agent_id {
            channel.agent_id = agent_id;
        }
        if let Some(config) = req.config {
            channel.config = config;
        }

        if let Some(credentials) = &req.credentials {
            let principal = Principal::channel(&channel.id);
            self.verify_grants(user_id, credentials, &principal).await?;
            self.vault
                .delete_bindings_for_principal(user_id, &principal)
                .await?;
            self.write_bindings(user_id, &channel.id, credentials.clone()).await?;
        }

        channel.updated_at = Utc::now();
        let persisted = self.repo.update(&channel).await?;

        self.broadcast.broadcast_entity_updated(
            user_id,
            "channel",
            &persisted.id,
            EntityAction::Updated,
            Some(persisted.space_id.clone()),
            None,
        );
        Ok(persisted)
    }

    pub async fn delete(
        &self,
        user_id: &str,
        channel_id: &str,
    ) -> Result<(), AppError> {
        let channel = self.find_owned(user_id, channel_id).await?;
        let principal = Principal::channel(&channel.id);
        self.vault.delete_bindings_for_principal(user_id, &principal).await?;
        self.repo.delete(&channel.id).await?;
        self.broadcast.broadcast_entity_updated(
            user_id,
            "channel",
            &channel.id,
            EntityAction::Deleted,
            Some(channel.space_id.clone()),
            None,
        );
        Ok(())
    }

    pub async fn start(
        &self,
        state: &crate::core::state::AppState,
        user_id: &str,
        channel_id: &str,
    ) -> Result<Channel, AppError> {
        let channel = self.find_owned(user_id, channel_id).await?;
        if channel.status == ChannelStatus::Connected {
            return Ok(channel);
        }
        self.mark_status(channel_id, ChannelStatus::Connecting, None).await?;
        state.channel_manager.start_channel(state, &channel).await?;
        self.find_by_id(channel_id).await
    }

    pub async fn stop(
        &self,
        state: &crate::core::state::AppState,
        user_id: &str,
        channel_id: &str,
    ) -> Result<Channel, AppError> {
        let channel = self.find_owned(user_id, channel_id).await?;
        if channel.status == ChannelStatus::Disconnected {
            return Ok(channel);
        }
        state.channel_manager.stop_channel(channel_id).await;
        self.mark_status(channel_id, ChannelStatus::Disconnected, None).await?;
        self.find_by_id(channel_id).await
    }

    pub async fn mark_status(
        &self,
        channel_id: &str,
        status: ChannelStatus,
        error: Option<String>,
    ) -> Result<(), AppError> {
        let mut channel = self.find_by_id(channel_id).await?;
        channel.status = status;
        channel.error_message = error;
        if matches!(status, ChannelStatus::Connected) {
            channel.last_started_at = Some(Utc::now());
        }
        channel.updated_at = Utc::now();
        let user_id = channel.user_id.clone();
        let space_id = channel.space_id.clone();
        let id = channel.id.clone();
        self.repo.update(&channel).await?;
        self.broadcast.broadcast_entity_updated(
            &user_id,
            "channel",
            &id,
            EntityAction::Updated,
            Some(space_id),
            None,
        );
        Ok(())
    }

    pub async fn resolve_config(
        &self,
        channel: &Channel,
    ) -> Result<serde_json::Value, AppError> {
        let factory = self.registry.get_factory(&channel.provider).ok_or_else(|| {
            AppError::Validation(format!("unknown channel provider {:?}", channel.provider))
        })?;
        let manifest = factory.manifest();

        let mut out = serde_json::Map::new();
        for field in &manifest.config_fields {
            if let Some(r) = &field.default_from
                && let Some(v) = resolve_config_default(&self.config, r)
            {
                out.insert(field.name.clone(), serde_json::Value::String(v));
            }
        }
        for (k, v) in &channel.config {
            out.insert(k.clone(), serde_json::Value::String(v.clone()));
        }

        let principal = Principal::channel(&channel.id);
        let bindings = self.vault
            .list_bindings_for_principal(&channel.user_id, &principal)
            .await?;
        for binding in bindings {
            let authorized = self.vault
                .has_grant_for_item(
                    &channel.user_id,
                    &principal,
                    &binding.connection_id,
                    &binding.vault_item_id,
                )
                .await?;
            if !authorized {
                return Err(AppError::Forbidden(format!(
                    "grant missing for vault item {} in connection {} — re-approve it",
                    binding.vault_item_id, binding.connection_id,
                )));
            }
            let secret = self.vault
                .get_secret(&channel.user_id, &binding.connection_id, &binding.vault_item_id)
                .await?;
            for (k, v) in project_target(&secret, &binding.target) {
                out.insert(k, serde_json::Value::String(v));
            }
        }

        for field in &manifest.config_fields {
            if field.is_required && !out.contains_key(&field.name) {
                return Err(AppError::Validation(format!(
                    "channel field {:?} is required but unresolved — supply it in config or bind a vault credential",
                    field.name,
                )));
            }
        }
        Ok(serde_json::Value::Object(out))
    }

    pub fn list_manifests_with_resolved_defaults(&self) -> Vec<ChannelManifest> {
        let mut manifests = self.registry.list_manifests();
        for manifest in &mut manifests {
            for field in &mut manifest.config_fields {
                if field.is_secret {
                    continue;
                }
                if let Some(r) = &field.default_from {
                    field.default_resolved = resolve_config_default(&self.config, r);
                }
            }
        }
        manifests
    }

    pub async fn missing_required(&self, channel: &Channel) -> Result<Vec<String>, AppError> {
        let factory = self.registry.get_factory(&channel.provider).ok_or_else(|| {
            AppError::Validation(format!("unknown channel provider {:?}", channel.provider))
        })?;
        let manifest = factory.manifest();
        let bindings = self
            .vault
            .list_bindings_for_principal(&channel.user_id, &Principal::channel(&channel.id))
            .await?;
        let virtual_bindings: Vec<CredentialBinding> = bindings
            .into_iter()
            .map(|b| CredentialBinding {
                connection_id: b.connection_id,
                vault_item_id: b.vault_item_id,
                env_var: match &b.target {
                    CredentialTarget::Single { env_var, .. } => env_var.clone(),
                    _ => String::new(),
                },
                field: match b.target {
                    CredentialTarget::Single { field, .. } => field,
                    _ => crate::credential::vault::models::VaultField::Password,
                },
            })
            .collect();
        Ok(missing_required_fields(
            &manifest,
            &channel.config,
            &virtual_bindings,
            &self.config,
        ))
    }

    async fn write_bindings(
        &self,
        user_id: &str,
        channel_id: &str,
        bindings: Vec<CredentialBinding>,
    ) -> Result<(), AppError> {
        let principal = Principal::channel(channel_id);
        for binding in bindings {
            self.vault
                .create_binding(
                    user_id,
                    principal.clone(),
                    &binding.env_var,
                    &binding.connection_id,
                    &binding.vault_item_id,
                    CredentialTarget::Single {
                        env_var: binding.env_var.clone(),
                        field: binding.field,
                    },
                    BindingScope::Durable,
                    None,
                )
                .await?;
        }
        Ok(())
    }

    async fn verify_grants(
        &self,
        user_id: &str,
        bindings: &[CredentialBinding],
        principal: &Principal,
    ) -> Result<(), AppError> {
        for binding in bindings {
            let ok = self.vault
                .has_grant_for_item(
                    user_id,
                    principal,
                    &binding.connection_id,
                    &binding.vault_item_id,
                )
                .await?;
            if !ok {
                return Err(AppError::Forbidden(format!(
                    "no grant for vault item {} in connection {} — approve it before installing",
                    binding.vault_item_id, binding.connection_id,
                )));
            }
        }
        Ok(())
    }

    pub async fn initiate_pairing(
        &self,
        user_id: &str,
        channel_id: &str,
    ) -> Result<String, AppError> {
        let mut channel = self.find_owned(user_id, channel_id).await?;
        let now = Utc::now();
        let code = generate_pair_code();
        let prior_address = channel
            .user_address
            .as_ref()
            .and_then(|ua| ua.address.clone());
        channel.user_address = Some(UserAddress {
            address: prior_address,
            pairing_code: Some(code.clone()),
            pairing_initiated_at: Some(now),
            paired_at: channel.user_address.and_then(|ua| ua.paired_at),
        });
        channel.status = ChannelStatus::Pairing;
        channel.updated_at = now;
        self.repo.update(&channel).await?;
        self.broadcast_update(&channel, EntityAction::Updated);
        Ok(code)
    }

    pub async fn cancel_pairing(
        &self,
        user_id: &str,
        channel_id: &str,
    ) -> Result<(), AppError> {
        let channel = self.find_owned(user_id, channel_id).await?;
        if channel.status != ChannelStatus::Pairing {
            return Ok(());
        }
        self.revert_pairing(channel).await?;
        Ok(())
    }

    pub async fn try_redeem_pairing(
        &self,
        channel_id: &str,
        sender_address: &str,
        message_body: &str,
    ) -> Result<bool, AppError> {
        let mut channel = self
            .repo
            .find_by_id(channel_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("channel {channel_id} not found")))?;
        if channel.status != ChannelStatus::Pairing {
            return Ok(false);
        }
        let Some(ua) = channel.user_address.as_ref() else {
            return Ok(false);
        };
        let Some(expected) = ua.pairing_code.as_deref() else {
            return Ok(false);
        };
        if message_body.trim() != expected {
            return Ok(false);
        }
        let now = Utc::now();
        channel.user_address = Some(UserAddress {
            address: Some(sender_address.to_string()),
            pairing_code: None,
            pairing_initiated_at: None,
            paired_at: Some(now),
        });
        channel.status = ChannelStatus::Connected;
        channel.updated_at = now;
        self.repo.update(&channel).await?;
        self.broadcast_update(&channel, EntityAction::Updated);
        Ok(true)
    }

    pub async fn revert_expired_pairings(&self) -> Result<u64, AppError> {
        let now = Utc::now();
        let cutoff = now - PAIRING_TTL;
        let pending = self.repo.find_in_status(ChannelStatus::Pairing).await?;
        let mut reverted = 0u64;
        for channel in pending {
            let initiated = channel
                .user_address
                .as_ref()
                .and_then(|ua| ua.pairing_initiated_at);
            if initiated.is_some_and(|t| t < cutoff) {
                self.revert_pairing(channel).await?;
                reverted += 1;
            }
        }
        Ok(reverted)
    }

    pub async fn revert_orphaned_pairings(&self) -> Result<u64, AppError> {
        let pending = self.repo.find_in_status(ChannelStatus::Pairing).await?;
        let count = pending.len() as u64;
        for channel in pending {
            self.revert_pairing(channel).await?;
        }
        Ok(count)
    }

    async fn revert_pairing(&self, mut channel: Channel) -> Result<(), AppError> {
        let prior_address = channel
            .user_address
            .as_ref()
            .and_then(|ua| ua.address.clone());
        let prior_paired_at = channel.user_address.as_ref().and_then(|ua| ua.paired_at);
        channel.status = if prior_address.is_some() {
            ChannelStatus::Connected
        } else {
            ChannelStatus::Disconnected
        };
        channel.user_address = if prior_address.is_some() {
            Some(UserAddress {
                address: prior_address,
                pairing_code: None,
                pairing_initiated_at: None,
                paired_at: prior_paired_at,
            })
        } else {
            None
        };
        channel.updated_at = Utc::now();
        self.repo.update(&channel).await?;
        self.broadcast_update(&channel, EntityAction::Updated);
        Ok(())
    }

    fn broadcast_update(&self, channel: &Channel, action: EntityAction) {
        self.broadcast.broadcast_entity_updated(
            &channel.user_id,
            "channel",
            &channel.id,
            action,
            Some(channel.space_id.clone()),
            None,
        );
    }
}

const PAIRING_TTL: Duration = Duration::minutes(5);

const PAIR_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const PAIR_CODE_LEN: usize = 6;

fn generate_pair_code() -> String {
    let mut rng = rand::thread_rng();
    (0..PAIR_CODE_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..PAIR_CODE_ALPHABET.len());
            PAIR_CODE_ALPHABET[idx] as char
        })
        .collect()
}

fn missing_required_fields(
    manifest: &ChannelManifest,
    config: &std::collections::BTreeMap<String, String>,
    credentials: &[CredentialBinding],
    server_config: &Config,
) -> Vec<String> {
    let mut missing = Vec::new();
    for field in &manifest.config_fields {
        if !field.is_required {
            continue;
        }
        let has_default = field
            .default_from
            .as_ref()
            .and_then(|r| resolve_config_default(server_config, r))
            .is_some();
        let satisfied = has_default
            || config.contains_key(&field.name)
            || (field.is_secret && credentials.iter().any(|b| b.env_var == field.name));
        if !satisfied {
            missing.push(field.name.clone());
        }
    }
    missing
}
