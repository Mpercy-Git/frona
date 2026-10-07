//! Waking an agent from outside Frona: a doorbell press, a webhook, a sensor.
//!
//! `POST /api/agents/{id}/trigger` opens a fresh chat with the agent, posts the
//! caller's message (and any images, saved as ordinary attachments so a vision
//! model sees them on the first turn) and starts the agent's turn straight away.
//! Unlike `POST /api/tasks`, it needs no full-account token: the owner mints a
//! trigger token for one agent with `POST /api/agents/{id}/trigger-tokens`, and
//! that token is refused by every other route (see `AGENT_TRIGGER_SCOPE`). A
//! token sitting in a doorbell bridge's config can therefore wake that one agent
//! and nothing else. Trigger tokens are PATs, so they are listed and revoked with
//! the user's other tokens.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{FromRequestParts, Path, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::error::ApiError;
use super::super::middleware::auth::{AuthUser, extract_token};
use crate::agent::service::AgentAccess;
use crate::auth::models::Claims;
use crate::auth::token::models::{AGENT_TRIGGER_SCOPE, CreatePatRequest, PatListItem, PatResponse};
use crate::auth::token::service::is_trigger_only;
use crate::chat::models::{CreateChatRequest, PUSH_IMAGE_METADATA_KEY};
use crate::core::Principal;
use crate::core::error::AppError;
use crate::core::principal::PrincipalKind;
use crate::core::state::AppState;
use crate::inference::conversation::DefaultConversationBuilder;
use crate::storage::{Attachment, dedup_filename};
use crate::tool::files::read::is_supported_image;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/agents/{id}/trigger", post(trigger_agent))
        .route(
            "/api/agents/{id}/trigger-tokens",
            post(create_trigger_token).get(list_trigger_tokens),
        )
}

const MAX_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_IMAGES: usize = 4;
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
/// Per agent. A doorbell rings a handful of times a minute at most; anything
/// above this is a stuck sender or a leaked token running up model spend.
const MAX_TRIGGERS_PER_MINUTE: usize = 6;
const DEFAULT_TOKEN_DAYS: u64 = 365;

#[derive(Debug, Deserialize)]
pub struct TriggerRequest {
    /// What happened, in words the agent reads as the opening message.
    pub message: String,
    /// Chat title. Defaults to "<agent name> trigger".
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub images: Vec<TriggerImage>,
}

#[derive(Debug, Deserialize)]
pub struct TriggerImage {
    /// `image/jpeg`, `image/png`, `image/gif` or `image/webp`.
    pub media_type: String,
    /// Base64, standard alphabet.
    pub data: String,
}

#[derive(Debug, Serialize)]
pub struct TriggerResponse {
    pub chat_id: String,
    pub message_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTriggerTokenRequest {
    pub name: String,
    #[serde(default)]
    pub expires_in_days: Option<u64>,
}

/// The caller of the trigger route: a trigger token, or an ordinary user token.
struct TriggerAuth(Claims);

impl FromRequestParts<AppState> for TriggerAuth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = extract_token(parts)?;
        let claims = state
            .token_service
            .validate_agent_trigger(&state.keypair_service, token)
            .await?;
        Ok(Self(claims))
    }
}

/// A trigger token may wake only the agent it was minted for; a user token may
/// wake any agent its user owns (checked by the caller).
fn authorize(claims: &Claims, agent_id: &str) -> Result<(), AppError> {
    if is_trigger_only(claims) {
        if claims.principal.kind == PrincipalKind::Agent && claims.principal.id == agent_id {
            return Ok(());
        }
        return Err(AppError::Forbidden(
            "This trigger token belongs to a different agent".into(),
        ));
    }
    if claims.principal.kind == PrincipalKind::User {
        return Ok(());
    }
    Err(AppError::Forbidden(
        "Only a user token or a trigger token can trigger an agent".into(),
    ))
}

fn allow_trigger(agent_id: &str) -> bool {
    static RECENT: OnceLock<Mutex<HashMap<String, VecDeque<Instant>>>> = OnceLock::new();
    let mut recent = RECENT
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let window = recent.entry(agent_id.to_string()).or_default();
    while window
        .front()
        .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60))
    {
        window.pop_front();
    }
    if window.len() >= MAX_TRIGGERS_PER_MINUTE {
        return false;
    }
    window.push_back(now);
    true
}

fn extension_for(media_type: &str) -> &'static str {
    match media_type {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    }
}

/// Decode and check every image before anything is written, so a bad request
/// leaves no files or chats behind.
fn decode_images(images: &[TriggerImage]) -> Result<Vec<(&str, Vec<u8>)>, AppError> {
    if images.len() > MAX_IMAGES {
        return Err(AppError::Validation(format!(
            "At most {MAX_IMAGES} images per trigger"
        )));
    }
    images
        .iter()
        .enumerate()
        .map(|(i, image)| {
            let media_type = image.media_type.as_str();
            if !is_supported_image(media_type) {
                return Err(AppError::Validation(format!(
                    "images[{i}]: unsupported type {media_type}"
                )));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(image.data.trim())
                .map_err(|_| AppError::Validation(format!("images[{i}]: invalid base64")))?;
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(AppError::Validation(format!(
                    "images[{i}]: larger than {} MB",
                    MAX_IMAGE_BYTES / 1024 / 1024
                )));
            }
            Ok((media_type, bytes))
        })
        .collect()
}

async fn trigger_agent(
    TriggerAuth(claims): TriggerAuth,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<TriggerRequest>,
) -> Result<(StatusCode, Json<TriggerResponse>), ApiError> {
    authorize(&claims, &agent_id)?;
    let (agent, access) = state
        .agent_service
        .get_accessible(&claims.sub, &agent_id)
        .await?;
    if access != AgentAccess::Owner {
        return Err(AppError::Forbidden("Only the agent's owner can trigger it".into()).into());
    }

    let message = req.message.trim();
    if message.is_empty() {
        return Err(AppError::Validation("message must not be empty".into()).into());
    }
    if message.len() > MAX_MESSAGE_BYTES {
        return Err(AppError::Validation(format!(
            "message must be at most {MAX_MESSAGE_BYTES} bytes"
        ))
        .into());
    }
    let images = decode_images(&req.images)?;

    if !allow_trigger(&agent_id) {
        return Err(AppError::Http {
            status: 429,
            message: format!(
                "This agent was triggered {MAX_TRIGGERS_PER_MINUTE} times in the last minute; try again shortly"
            ),
        }
        .into());
    }

    // Saved in the owner's files, like an upload, so the chat shows them and
    // the conversation builder hands them to the model.
    let user_id = claims.sub.clone();
    let owner = format!("user:{user_id}");
    let relative_dir = format!("triggers/{}", agent.handle.as_str());
    let dir = state
        .storage_service
        .user_workspace(&claims.handle)
        .base_path()
        .join(&relative_dir);
    let mut attachments = Vec::with_capacity(images.len());
    if !images.is_empty() {
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| AppError::Internal(format!("could not create {relative_dir}: {e}")))?;
    }
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    for (i, (media_type, bytes)) in images.into_iter().enumerate() {
        let filename = dedup_filename(&dir, &format!("{stamp}-{i}.{}", extension_for(media_type)));
        tokio::fs::write(dir.join(&filename), &bytes)
            .await
            .map_err(|e| AppError::Internal(format!("could not save trigger image: {e}")))?;
        attachments.push(Attachment {
            path: format!("{relative_dir}/{filename}"),
            filename,
            content_type: media_type.to_string(),
            size_bytes: bytes.len() as u64,
            owner: owner.clone(),
            url: None,
            transcript: None,
        });
    }

    let mut metadata = BTreeMap::new();
    metadata.insert(
        "trigger".to_string(),
        json!({ "token_id": claims.token_id, "token_type": claims.token_type }),
    );
    if let Some(first) = attachments.first() {
        metadata.insert(
            PUSH_IMAGE_METADATA_KEY.to_string(),
            json!({ "owner": first.owner, "path": first.path }),
        );
    }

    let title = req
        .title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| format!("{} trigger", agent.name));
    let chat = state
        .chat_service
        .create_chat(
            &user_id,
            CreateChatRequest {
                space_id: None,
                task_id: None,
                agent_id: agent_id.clone(),
                title: Some(title),
                metadata: Some(metadata),
            },
        )
        .await?;

    state
        .chat_service
        .create_stream_user_message(&user_id, &chat.id, message, attachments, None)
        .await?;
    let agent_msg = state
        .chat_service
        .create_executing_agent_message(&chat.id, &agent_id)
        .await?;

    // Same start-up as a message sent from the web chat: register the turn so
    // Stop in the UI reaches it, then run it in the background.
    let harness = state.harness.clone();
    let chat_id = chat.id.clone();
    let agent_msg_id = agent_msg.id.clone();
    let (session_id, cancel_token) = state.active_sessions.register(&chat_id).await;
    let builder = Box::new(DefaultConversationBuilder {
        user_service: state.user_service.clone(),
        storage_service: state.storage_service.clone(),
        agent_service: state.agent_service.clone(),
    });
    let active_sessions = state.active_sessions.clone();
    {
        let chat_id = chat_id.clone();
        let agent_msg_id = agent_msg_id.clone();
        tokio::spawn(async move {
            harness
                .run_turn(
                    &user_id,
                    &chat_id,
                    &agent_msg_id,
                    cancel_token,
                    builder,
                    &[],
                    None,
                    Some(session_id),
                )
                .await;
            active_sessions.remove(&chat_id, session_id).await;
        });
    }

    tracing::info!(agent_id = %agent_id, chat_id = %chat_id, "Agent triggered");
    Ok((
        StatusCode::ACCEPTED,
        Json(TriggerResponse {
            chat_id,
            message_id: agent_msg_id,
        }),
    ))
}

/// Mint a token that can trigger this one agent and do nothing else. Signed-in
/// owners only: a PAT can't mint tokens, matching `POST /api/auth/tokens`.
async fn create_trigger_token(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<CreateTriggerTokenRequest>,
) -> Result<(StatusCode, Json<PatResponse>), ApiError> {
    if auth.is_pat() {
        return Err(AppError::Forbidden("PATs cannot create other tokens".into()).into());
    }
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Validation("name must not be empty".into()).into());
    }
    let (_, access) = state
        .agent_service
        .get_accessible(&auth.user_id, &agent_id)
        .await?;
    if access != AgentAccess::Owner {
        return Err(AppError::Forbidden(
            "Only the agent's owner can mint its trigger tokens".into(),
        )
        .into());
    }
    let user = state
        .user_service
        .find_by_id(&auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;

    let pat = state
        .token_service
        .create_pat(
            &state.keypair_service,
            &user,
            CreatePatRequest {
                name,
                expires_in_days: Some(req.expires_in_days.unwrap_or(DEFAULT_TOKEN_DAYS)),
                scopes: Some(vec![AGENT_TRIGGER_SCOPE.to_string()]),
                principal: Some(Principal::agent(&agent_id)),
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(pat)))
}

/// The agent's trigger tokens, for its settings page. Revoking goes through
/// `DELETE /api/auth/tokens/{id}` like any other PAT.
async fn list_trigger_tokens(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<Vec<PatListItem>>, ApiError> {
    let (_, access) = state
        .agent_service
        .get_accessible(&auth.user_id, &agent_id)
        .await?;
    if access != AgentAccess::Owner {
        return Err(AppError::Forbidden(
            "Only the agent's owner can see its trigger tokens".into(),
        )
        .into());
    }
    let tokens = state
        .token_service
        .list_agent_trigger_tokens(&auth.user_id, &agent_id)
        .await?;
    Ok(Json(tokens))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(kind: PrincipalKind, id: &str, scopes: Option<Vec<String>>) -> Claims {
        Claims {
            sub: "user-1".into(),
            handle: crate::handle!("owner"),
            email: "owner@example.com".into(),
            exp: 0,
            iat: 0,
            token_id: "tok".into(),
            token_type: "pat".into(),
            principal: Principal {
                kind,
                id: id.into(),
            },
            scopes,
            extensions: None,
        }
    }

    fn trigger_scope() -> Option<Vec<String>> {
        Some(vec![AGENT_TRIGGER_SCOPE.to_string()])
    }

    #[test]
    fn a_trigger_token_wakes_only_its_own_agent() {
        let token = claims(PrincipalKind::Agent, "front-door", trigger_scope());
        assert!(authorize(&token, "front-door").is_ok());
        assert!(authorize(&token, "finance").is_err());
    }

    #[test]
    fn a_user_token_may_trigger_and_other_principals_may_not() {
        assert!(authorize(&claims(PrincipalKind::User, "user-1", None), "any").is_ok());
        // An agent-principal PAT minted without the trigger scope is not a
        // trigger token.
        assert!(
            authorize(
                &claims(PrincipalKind::Agent, "front-door", None),
                "front-door"
            )
            .is_err()
        );
        assert!(authorize(&claims(PrincipalKind::App, "app", None), "front-door").is_err());
    }

    #[test]
    fn images_are_checked_before_anything_is_saved() {
        let ok = TriggerImage {
            media_type: "image/jpeg".into(),
            data: base64::engine::general_purpose::STANDARD.encode([0xff, 0xd8, 0xff]),
        };
        assert_eq!(decode_images(std::slice::from_ref(&ok)).unwrap().len(), 1);

        let wrong_type = TriggerImage {
            media_type: "image/tiff".into(),
            data: ok.data.clone(),
        };
        assert!(decode_images(&[wrong_type]).is_err());

        let bad_base64 = TriggerImage {
            media_type: "image/png".into(),
            data: "%%%".into(),
        };
        assert!(decode_images(&[bad_base64]).is_err());

        let too_many: Vec<TriggerImage> = (0..=MAX_IMAGES)
            .map(|_| TriggerImage {
                media_type: ok.media_type.clone(),
                data: ok.data.clone(),
            })
            .collect();
        assert!(decode_images(&too_many).is_err());
    }

    #[test]
    fn the_rate_limit_is_per_agent() {
        let agent = format!("rate-{}", crate::core::repository::new_id());
        for _ in 0..MAX_TRIGGERS_PER_MINUTE {
            assert!(allow_trigger(&agent));
        }
        assert!(!allow_trigger(&agent));
        assert!(allow_trigger(&format!("{agent}-other")));
    }
}
