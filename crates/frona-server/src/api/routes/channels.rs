use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::chat::channel::models::{Channel, CreateChannelRequest, ChannelManifest, UpdateChannelRequest};
use crate::core::error::AppError;
use crate::core::state::AppState;

use crate::api::error::ApiError;
use crate::api::middleware::auth::AuthUser;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            &format!(
                "{}/{{provider}}/{{channel_id}}",
                crate::chat::channel::WEBHOOK_PATH_PREFIX,
            ),
            post(channel_webhook).get(channel_webhook),
        )
        .route("/api/channels/manifests", get(list_manifests))
        .route("/api/channels", get(list_channels).post(create_channel))
        .route(
            "/api/channels/{id}",
            get(get_channel).patch(update_channel).delete(delete_channel),
        )
        .route("/api/channels/{id}/start", post(start_channel))
        .route("/api/channels/{id}/stop", post(stop_channel))
        .route(
            "/api/channels/{id}/pair",
            post(initiate_pairing).delete(cancel_pairing),
        )
        .route("/api/channels/{id}/setup/refresh", post(refresh_setup))
}

const MAX_WEBHOOK_BYTES: usize = 10 * 1024 * 1024;

async fn channel_webhook(
    State(state): State<AppState>,
    Path((provider, channel_id)): Path<(String, String)>,
    request: Request,
) -> Result<Response, ApiError> {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, MAX_WEBHOOK_BYTES)
        .await
        .map_err(|e| AppError::Validation(format!("failed to read webhook body: {e}")))?;
    let request = axum::http::Request::from_parts(parts, bytes);

    let channel = match state.channel_service.find_by_id(&channel_id).await {
        Ok(c) => c,
        Err(AppError::NotFound(_)) => {
            tracing::warn!(
                provider = %provider,
                channel_id = %channel_id,
                "inbound webhook for unknown channel — likely an orphaned upstream registration \
                 still pointing at this server after the channel row was deleted; clear it on \
                 the provider side",
            );
            return Err(AppError::NotFound(format!("channel {channel_id} not found")).into());
        }
        Err(e) => return Err(e.into()),
    };
    if channel.provider != provider {
        tracing::warn!(
            url_provider = %provider,
            channel_id = %channel_id,
            channel_provider = ?channel.provider,
            "inbound webhook provider does not match the channel's provider",
        );
        return Err(AppError::NotFound(format!(
            "channel {channel_id} provider {:?} does not match URL provider {provider:?}",
            channel.provider,
        ))
        .into());
    }

    let response = state
        .channel_manager
        .dispatch_inbound_webhook(&channel_id, request)
        .await?;
    Ok(response)
}

async fn list_channels(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<Channel>>, ApiError> {
    let channels = state.channel_service.list_for_user(&auth.user_id).await?;
    Ok(Json(channels))
}

async fn create_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateChannelRequest>,
) -> Result<Json<Channel>, ApiError> {
    let space = state
        .space_service
        .find_by_id(&req.space_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Space {} not found", req.space_id)))?;
    if space.user_id != auth.user_id {
        return Err(AppError::Forbidden("not your space".into()).into());
    }
    let channel = state.channel_service.create(&auth.user_id, req).await?;
    Ok(Json(channel))
}

async fn get_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Channel>, ApiError> {
    let mut channel = state.channel_service.find_owned(&auth.user_id, &id).await?;
    if let Some(manifest) = state.channel_registry.get_manifest(&channel.provider)
        && manifest.webhook_url_visible
    {
        channel.webhook_url = Some(build_webhook_url(&state, &channel));
    }
    Ok(Json(channel))
}

fn build_webhook_url(state: &AppState, channel: &Channel) -> String {
    let base = state
        .config
        .server
        .external_base_url()
        .unwrap_or_else(|| format!("http://localhost:{}", state.config.server.port));
    format!(
        "{}{}/{}/{}",
        base.trim_end_matches('/'),
        crate::chat::channel::WEBHOOK_PATH_PREFIX,
        channel.provider,
        channel.id,
    )
}

async fn update_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateChannelRequest>,
) -> Result<Json<Channel>, ApiError> {
    let channel = state.channel_service.update(&auth.user_id, &id, req).await?;
    Ok(Json(channel))
}

async fn delete_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.channel_service.delete(&state, &auth.user_id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn start_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Channel>, ApiError> {
    let channel = state.channel_service.start(&auth.user_id, &id).await?;
    Ok(Json(channel))
}

async fn stop_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Channel>, ApiError> {
    let channel = state.channel_service.stop(&state, &auth.user_id, &id).await?;
    Ok(Json(channel))
}

async fn list_manifests(
    State(state): State<AppState>,
) -> Json<Vec<ChannelManifest>> {
    Json(state.channel_service.list_manifests_with_resolved_defaults())
}

#[derive(serde::Serialize)]
struct PairingResponse {
    code: String,
}

async fn initiate_pairing(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PairingResponse>, ApiError> {
    let code = state
        .channel_service
        .initiate_pairing(&auth.user_id, &id)
        .await?;
    Ok(Json(PairingResponse { code }))
}

async fn cancel_pairing(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state
        .channel_service
        .cancel_pairing(&auth.user_id, &id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn refresh_setup(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Channel>, ApiError> {
    // Authorise: must own the channel.
    let _channel = state.channel_service.find_owned(&auth.user_id, &id).await?;
    state.channel_manager.stop_channel(&id).await;
    let channel = state.channel_service.find_by_id(&id).await?;
    state.channel_manager.start_channel(&state, &channel).await?;
    let channel = state.channel_service.find_by_id(&id).await?;
    Ok(Json(channel))
}
