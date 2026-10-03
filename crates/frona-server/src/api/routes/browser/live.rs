//! Live view of the agent's browser, for human takeover.
//!
//! The page streams over a WebSocket on Frona's own origin, so it works
//! wherever the app does — behind Caddy, nginx or a tunnel — with nothing
//! extra to route: no Browserless port is exposed and no Browserless token
//! ever reaches the person's browser.

use std::collections::HashSet;
use std::time::Duration;

use axum::Json;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::response::Response;
use frona_browser::{BrowserConnection, LiveCommand, LiveTab, Screencast};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::MissedTickBehavior;

use super::super::super::error::ApiError;
use super::super::super::middleware::auth::{AuthUser, NavigableAuth};
use crate::core::Handle;
use crate::core::error::AppError;
use crate::core::state::AppState;
use crate::tool::browser::{active_profile, is_user_profile};

/// Presign tokens for the live view are scoped with this owner, and the
/// profile as the path, so a token for one profile can't open another.
const LIVE_OWNER: &str = "browser-live";
/// The token is fetched and used straight away to open the socket; it only
/// gates the upgrade, not the session that follows.
const LIVE_TOKEN_EXPIRY_SECS: u64 = 120;
/// How often to look for tabs opening or closing. A login's "Sign in with…"
/// popup is a new tab, and the viewer should follow it there.
const TAB_POLL: Duration = Duration::from_secs(1);
/// A page that isn't changing sends no frames. Pinging keeps proxies with an
/// idle timeout (Cloudflare, some nginx setups) from cutting the socket.
const PING_EVERY_TICKS: u32 = 25;

#[derive(Deserialize)]
pub(super) struct LiveQuery {
    #[serde(default)]
    profile: Option<String>,
}

/// Mint the short-lived token the viewer opens its WebSocket with. Browsers
/// can't put an `Authorization` header on a WebSocket, so it rides in the URL.
pub(super) async fn live_link(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<LiveQuery>,
) -> Result<Json<Value>, ApiError> {
    if state.browser_session_manager.config().is_none() {
        return Err(AppError::Browser("Browser is not configured".into()).into());
    }
    let profile = match query.profile.filter(|p| !p.is_empty()) {
        Some(profile) => profile,
        None => active_profile(&state.vault_service, &auth.user_id).await,
    };
    if !is_user_profile(&state.vault_service, &auth.user_id, &profile).await {
        return Err(AppError::NotFound("Browser profile not found".into()).into());
    }

    let token = state
        .presign_service
        .sign_scoped_token(LIVE_OWNER, &profile, &auth.user_id, LIVE_TOKEN_EXPIRY_SECS)
        .await?;
    Ok(Json(json!({ "profile": profile, "token": token })))
}

pub(super) async fn live_socket(
    auth: NavigableAuth,
    State(state): State<AppState>,
    Query(query): Query<LiveQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let profile = query
        .profile
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::Validation("profile is required".into()))?;
    auth.require_presign_scope(LIVE_OWNER, &profile)?;

    let user_id = auth.user_id().to_string();
    if !is_user_profile(&state.vault_service, &user_id, &profile).await {
        return Err(AppError::NotFound("Browser profile not found".into()).into());
    }
    let handle = match &auth {
        NavigableAuth::User { handle, .. } => handle.clone(),
        NavigableAuth::Presigned(_) => state.user_service.handle_of(&user_id).await?,
    };

    Ok(ws.on_upgrade(move |socket| live_session(socket, state, handle, profile)))
}

async fn live_session(mut socket: WebSocket, state: AppState, handle: Handle, profile: String) {
    let manager = &state.browser_session_manager;
    let claim = manager.claim_live_view(&handle, &profile);

    send(
        &mut socket,
        json!({ "type": "status", "state": "connecting" }),
    )
    .await;
    let conn = match manager.connection(&handle, &profile).await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::warn!(error = %e, user = %handle, profile = %profile, "live view: could not connect to browser");
            end(&mut socket, "Could not connect to the browser.").await;
            return;
        }
    };
    let mut cast = match conn.start_screencast(None).await {
        Ok(cast) => cast,
        Err(e) => {
            tracing::warn!(error = %e, "live view: could not start screencast");
            end(&mut socket, "Could not start the live view.").await;
            return;
        }
    };
    send(&mut socket, json!({ "type": "status", "state": "live" })).await;

    let mut tabs = TabWatch::default();
    let mut ticker = tokio::time::interval(TAB_POLL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut ticks: u32 = 0;

    let reason = loop {
        tokio::select! {
            _ = claim.replaced.notified() => {
                break Some("This browser was opened in another window.");
            }
            msg = socket.recv() => match msg {
                Some(Ok(Message::Text(text))) => {
                    if let Err(message) = handle_message(&conn, &mut cast, &text).await {
                        send(&mut socket, json!({ "type": "error", "message": message })).await;
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break None,
                Some(Ok(_)) => {}
            },
            frame = cast.next_frame() => match frame {
                Some(frame) => {
                    let delivered = socket
                        .send(Message::Text(
                            json!({
                                "type": "frame",
                                "data": frame.data,
                                "width": frame.width,
                                "height": frame.height,
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .is_ok();
                    if !delivered {
                        break None;
                    }
                    // Acked only once delivered, so a slow connection gets
                    // fewer, fresher frames instead of a growing backlog.
                    let _ = cast.ack(frame.ack).await;
                }
                // The tab closed under us: carry on with whichever the agent
                // would use next.
                None => {
                    if let Err(e) = switch_tab(&conn, &mut cast, None).await {
                        tracing::debug!(error = %e, "live view: no tab to follow");
                        break Some("The browser tab closed.");
                    }
                }
            },
            _ = ticker.tick() => {
                if !conn.is_alive() {
                    break Some("The browser session ended.");
                }
                ticks = ticks.wrapping_add(1);
                if ticks.is_multiple_of(PING_EVERY_TICKS)
                    && socket.send(Message::Ping(Default::default())).await.is_err()
                {
                    break None;
                }
                let Ok(current) = conn.live_tabs().await else { continue };
                if let Some(follow) = tabs.tab_to_follow(&current, cast.tab_id())
                    && let Err(e) = switch_tab(&conn, &mut cast, follow.as_deref()).await
                {
                    tracing::debug!(error = %e, "live view: could not follow tab");
                }
                if tabs.changed(&current, cast.tab_id()) {
                    send(&mut socket, json!({
                        "type": "tabs",
                        "tabs": current,
                        "current": cast.tab_id(),
                    }))
                    .await;
                }
            }
        }
    };

    let _ = cast.stop().await;
    match reason {
        Some(reason) => end(&mut socket, reason).await,
        None => {
            let _ = socket.send(Message::Close(None)).await;
        }
    }
}

/// Apply one message from the viewer. Errors are reported back to the viewer
/// rather than ending the session: a mistyped address shouldn't cost the
/// person their place.
async fn handle_message(
    conn: &BrowserConnection,
    cast: &mut Screencast,
    text: &str,
) -> Result<(), String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if value.get("type").and_then(Value::as_str) == Some("switch_tab") {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or("switch_tab needs an id")?;
        return switch_tab(conn, cast, Some(id))
            .await
            .map_err(|e| e.to_string());
    }
    let command: LiveCommand = serde_json::from_value(value).map_err(|e| e.to_string())?;
    cast.apply(command).await.map_err(|e| e.to_string())
}

async fn switch_tab(
    conn: &BrowserConnection,
    cast: &mut Screencast,
    tab_id: Option<&str>,
) -> frona_browser::Result<()> {
    if tab_id == Some(cast.tab_id()) {
        return Ok(());
    }
    let next = conn.start_screencast(tab_id).await?;
    let previous = std::mem::replace(cast, next);
    // The old tab may be the one that just closed; nothing to stop then.
    let _ = previous.stop().await;
    Ok(())
}

/// What the viewer was last told about the tabs, and which tabs it has seen.
#[derive(Default)]
struct TabWatch {
    seen: HashSet<String>,
    last_sent: Option<(Vec<LiveTab>, String)>,
    started: bool,
}

impl TabWatch {
    /// The tab to switch the stream to, if any: `Some(Some(id))` for a tab that
    /// just opened (a login popup), `Some(None)` when the streamed tab has gone
    /// and the agent's active tab should take over.
    fn tab_to_follow(&mut self, tabs: &[LiveTab], current: &str) -> Option<Option<String>> {
        let opened: Vec<&LiveTab> = tabs.iter().filter(|t| !self.seen.contains(&t.id)).collect();
        let first_look = !self.started;
        self.started = true;
        self.seen = tabs.iter().map(|t| t.id.clone()).collect();

        if !first_look && let Some(newest) = opened.last() {
            return Some(Some(newest.id.clone()));
        }
        if !tabs.iter().any(|t| t.id == current) {
            return Some(None);
        }
        None
    }

    fn changed(&mut self, tabs: &[LiveTab], current: &str) -> bool {
        let state = (tabs.to_vec(), current.to_string());
        if self.last_sent.as_ref() == Some(&state) {
            return false;
        }
        self.last_sent = Some(state);
        true
    }
}

async fn send(socket: &mut WebSocket, value: Value) {
    let _ = socket.send(Message::Text(value.to_string().into())).await;
}

async fn end(socket: &mut WebSocket, reason: &str) {
    send(
        socket,
        json!({ "type": "status", "state": "ended", "message": reason }),
    )
    .await;
    let _ = socket.send(Message::Close(None)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: &str) -> LiveTab {
        LiveTab {
            id: id.into(),
            url: format!("https://{id}.example"),
            title: id.into(),
        }
    }

    #[test]
    fn tabs_open_at_start_are_not_followed() {
        let mut watch = TabWatch::default();
        assert_eq!(watch.tab_to_follow(&[tab("a"), tab("b")], "a"), None);
    }

    #[test]
    fn a_newly_opened_tab_is_followed() {
        let mut watch = TabWatch::default();
        watch.tab_to_follow(&[tab("a")], "a");
        assert_eq!(
            watch.tab_to_follow(&[tab("a"), tab("popup")], "a"),
            Some(Some("popup".into()))
        );
        // Only once: it's no longer new on the next poll.
        assert_eq!(
            watch.tab_to_follow(&[tab("a"), tab("popup")], "popup"),
            None
        );
    }

    #[test]
    fn a_closed_tab_falls_back_to_the_active_one() {
        let mut watch = TabWatch::default();
        watch.tab_to_follow(&[tab("a"), tab("popup")], "popup");
        assert_eq!(watch.tab_to_follow(&[tab("a")], "popup"), Some(None));
    }

    #[test]
    fn tab_list_is_only_resent_when_it_changes() {
        let mut watch = TabWatch::default();
        assert!(watch.changed(&[tab("a")], "a"));
        assert!(!watch.changed(&[tab("a")], "a"));
        assert!(watch.changed(&[tab("a"), tab("b")], "a"));
        assert!(watch.changed(&[tab("a"), tab("b")], "b"));
    }
}
