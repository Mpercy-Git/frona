use std::convert::Infallible;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use futures::stream::Stream;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio_stream::wrappers::UnboundedReceiverStream;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;
use crate::core::error::AppError;
use crate::core::state::AppState;
use crate::policy::models::PolicyAction;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/system/health", get(health_handler))
        .route("/healthz", get(health_handler))
        .route("/api/system/info", get(info_handler))
        .route("/api/system/version", get(version_handler))
        .route("/api/system/timezones", get(timezones_handler))
        .route("/api/system/logs/stream", get(logs_stream_handler))
        .route("/api/system/restart", post(restart_handler))
}

async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    if state.is_shutting_down() {
        (StatusCode::SERVICE_UNAVAILABLE, axum::Json(json!({"status": "draining"})))
    } else {
        (StatusCode::OK, axum::Json(json!({"status": "ok"})))
    }
}

async fn info_handler(_auth: AuthUser, State(state): State<AppState>) -> axum::Json<serde_json::Value> {
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_memory();
    let total_memory = sys.cgroup_limits()
        .map(|cg| cg.total_memory)
        .unwrap_or_else(|| sys.total_memory());
    let cpus = System::physical_core_count().unwrap_or(0);

    axum::Json(json!({
        "version": crate::core::app_version(),
        "cpus": cpus,
        "total_memory_bytes": total_memory,
        "sandbox_driver": state.sandbox_factory.driver_id(),
        "server_timezone": state.config.server.timezone,
    }))
}

async fn version_handler(_auth: AuthUser) -> axum::Json<serde_json::Value> {
    axum::Json(json!({"version": crate::core::app_version()}))
}

async fn timezones_handler(_auth: AuthUser) -> axum::Json<Vec<String>> {
    axum::Json(list_iana_timezones())
}

fn list_iana_timezones() -> Vec<String> {
    let mut zones: Vec<String> = chrono_tz::TZ_VARIANTS
        .iter()
        .map(|tz| tz.name().to_string())
        .filter(|s| s.contains('/'))
        .collect();
    zones.sort();
    zones
}

/// Stream the server's own log output (admin only) as SSE. On connect the most
/// recent buffered lines are replayed, then new lines are pushed as they occur.
/// Each `data:` frame is a JSON `LogLine` object.
async fn logs_stream_handler(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    // Server logs may span every user, so gate on the same admin capability
    // that guards the user-management surfaces.
    let caller = state
        .user_service
        .find_by_id(&auth.user_id)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;
    let decision = state
        .policy_service
        .authorize_user(&caller, PolicyAction::ListUsers)
        .await?;
    if !decision.allowed {
        return Err(AppError::Forbidden("Not permitted".into()).into());
    }

    use crate::core::log_stream;
    let mut rx = log_stream::subscribe();
    let backlog = log_stream::recent();

    let (tx, out_rx) = tokio::sync::mpsc::unbounded_channel::<Result<Event, Infallible>>();

    // Replay recent history first so the viewer isn't blank on connect.
    for line in backlog {
        if let Ok(json) = serde_json::to_string(&line) {
            let _ = tx.send(Ok(Event::default().data(json)));
        }
    }

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(line) => {
                    let Ok(json) = serde_json::to_string(&line) else {
                        continue;
                    };
                    if tx.send(Ok(Event::default().data(json))).is_err() {
                        break; // client disconnected
                    }
                }
                // A slow reader dropped some lines; keep going with the rest.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });

    let stream = UnboundedReceiverStream::new(out_rx);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn restart_handler(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> axum::Json<serde_json::Value> {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        tracing::info!("Restart requested, draining in-flight work...");
        crate::core::shutdown::graceful_drain(&state).await;
        re_exec_self();
    });
    axum::Json(json!({"status": "restarting"}))
}

fn re_exec_self() -> ! {
    let exe = std::env::current_exe().expect("failed to get current executable path");
    let args: Vec<String> = std::env::args().skip(1).collect();

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Replace the current process image so the PID and supervisor handle
        // are preserved across the restart.
        let err = std::process::Command::new(&exe).args(&args).exec();
        panic!("exec failed: {err}");
    }

    #[cfg(not(unix))]
    {
        // Windows has no exec(); spawn a fresh copy and exit so the caller
        // still observes a restart. (Frona runs on Linux in production; this
        // arm only exists so the crate compiles for local checks on Windows.)
        match std::process::Command::new(&exe).args(&args).spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => panic!("failed to spawn replacement process: {e}"),
        }
    }
}
