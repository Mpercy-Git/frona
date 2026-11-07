use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use axum::body::Body;
use http_body_util::BodyExt;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use tokio::sync::{Mutex, Notify, RwLock};

use crate::core::config::BrowserConfig;
use crate::core::error::AppError;
use frona_browser::BrowserConnection;

/// Browserless treats `<=0` as `0ms` and 408s the upgrade instantly, so we
/// pass a long positive instead to defeat its 30s per-job default.
const BROWSERLESS_SESSION_TIMEOUT: Duration = Duration::from_secs(24 * 3600);
/// Self-evict before Browserless's hard limit so the next request rebuilds
/// cleanly instead of hitting a forced close mid-op.
const SELF_EVICT_MARGIN: Duration = Duration::from_secs(60);
const CONNECT_RETRY_DELAYS: [Duration; 2] =
    [Duration::from_millis(100), Duration::from_millis(200)];
/// The startup sweep runs as soon as the server boots, which under
/// `docker compose up` is while Browserless is still opening its HTTP
/// listener. Retry connect errors on the admin API so that race is absorbed
/// instead of reported as a failure to reach Browserless.
const SESSION_LIST_CONNECT_RETRIES: u32 = 4;
const SESSION_LIST_RETRY_MIN_DELAY: Duration = Duration::from_millis(500);
const SESSION_LIST_RETRY_MAX_DELAY: Duration = Duration::from_secs(4);

#[derive(serde::Deserialize)]
pub(crate) struct BrowserlessSession {
    #[serde(rename = "browserId")]
    browser_id: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(rename = "browserWSEndpoint", default)]
    pub browser_ws_endpoint: Option<String>,
    #[serde(rename = "type")]
    pub session_type: Option<String>,
    #[serde(rename = "userDataDir", default)]
    user_data_dir: Option<String>,
}

/// Per profile: the attached live viewer's id and its "you were replaced" signal.
type LiveViewers = Arc<std::sync::Mutex<HashMap<String, (u64, Arc<Notify>)>>>;

#[derive(Clone)]
pub struct BrowserSessionManager {
    config: Option<BrowserConfig>,
    sessions: Arc<RwLock<HashMap<String, BrowserConnection>>>,
    /// One lock per `(user, provider)` profile, pruned once no request holds it.
    /// Serializes connect/close so racing requests cannot launch Chrome twice
    /// against the same `--user-data-dir`.
    profile_locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
    /// The live viewer currently attached to each profile. A tab has one
    /// screencast, so a second viewer takes over from the first instead of the
    /// two stopping each other's stream.
    live_viewers: LiveViewers,
    next_live_viewer: Arc<AtomicU64>,
}

/// A live viewer's hold on a profile. `replaced` fires when another viewer
/// opens the same profile; dropping the claim releases it.
pub struct LiveViewClaim {
    pub replaced: Arc<Notify>,
    key: String,
    id: u64,
    viewers: LiveViewers,
}

impl Drop for LiveViewClaim {
    fn drop(&mut self) {
        if let Ok(mut viewers) = self.viewers.lock()
            && viewers.get(&self.key).is_some_and(|(id, _)| *id == self.id)
        {
            viewers.remove(&self.key);
        }
    }
}

impl BrowserSessionManager {
    pub fn new(config: Option<BrowserConfig>) -> Self {
        Self {
            config,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            profile_locks: Arc::new(Mutex::new(HashMap::new())),
            live_viewers: Arc::new(std::sync::Mutex::new(HashMap::new())),
            next_live_viewer: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Register a live viewer for a profile, telling any viewer already on it
    /// to stand down.
    pub fn claim_live_view(
        &self,
        user_handle: &crate::core::Handle,
        provider: &str,
    ) -> LiveViewClaim {
        let key = Self::profile_key(user_handle, provider);
        let id = self.next_live_viewer.fetch_add(1, Ordering::Relaxed);
        let replaced = Arc::new(Notify::new());
        let previous = self
            .live_viewers
            .lock()
            .ok()
            .and_then(|mut viewers| viewers.insert(key.clone(), (id, replaced.clone())));
        if let Some((_, previous)) = previous {
            // `notify_one` keeps the permit if the old viewer isn't waiting at
            // this instant, so it can't miss being replaced.
            previous.notify_one();
        }
        LiveViewClaim {
            replaced,
            key,
            id,
            viewers: self.live_viewers.clone(),
        }
    }

    pub fn config(&self) -> Option<&BrowserConfig> {
        self.config.as_ref()
    }

    /// Viewing a preview must never launch or reconnect a browser.
    pub async fn existing_connection(
        &self,
        user_handle: &crate::core::Handle,
        provider: &str,
    ) -> Option<BrowserConnection> {
        self.sessions
            .read()
            .await
            .get(&Self::profile_key(user_handle, provider))
            .filter(|connection| connection.is_alive())
            .cloned()
    }

    fn profile_key(user_handle: &crate::core::Handle, provider: &str) -> String {
        format!("{user_handle}/{provider}")
    }

    async fn profile_lock(&self, key: &str) -> Arc<Mutex<()>> {
        let mut locks = self.profile_locks.lock().await;
        // Keep only locks still owned by an active connection/close request.
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(key.to_string(), Arc::downgrade(&lock));
        lock
    }

    fn ws_url_for_profile(
        config: &BrowserConfig,
        user_handle: &crate::core::Handle,
        provider: &str,
    ) -> Result<url::Url, AppError> {
        config
            .websocket_url(&[
                (
                    "--user-data-dir",
                    &config.profile_path(user_handle, provider).to_string_lossy(),
                ),
                (
                    "timeout",
                    &BROWSERLESS_SESSION_TIMEOUT.as_millis().to_string(),
                ),
            ])
            .map_err(|_| AppError::Browser("Invalid Browserless URL".into()))
    }

    /// Builds the exact WS URL a real session would use (including the
    /// `--user-data-dir` / `timeout` launch args) so the settings-page "Test
    /// connection" button actually exercises what production sessions rely
    /// on, instead of only proving the bare WS endpoint is reachable.
    pub async fn test_connection(
        user_handle: &crate::core::Handle,
        config: &BrowserConfig,
        timeout: Duration,
    ) -> Result<(), frona_browser::Error> {
        let ws_url = Self::ws_url_for_profile(config, user_handle, "connection-test")
            .map_err(|error| frona_browser::Error::ToolFailed {
                tool: "browser_connect",
                message: error.to_string(),
            })?;
        let conn = BrowserConnection::connect(ws_url.as_str(), timeout, timeout).await?;
        conn.disconnect().await
    }

    async fn create_connection(
        &self,
        user_handle: &crate::core::Handle,
        provider: &str,
    ) -> Result<BrowserConnection, AppError> {
        let config = self.config.as_ref().ok_or_else(|| {
            AppError::Browser("Browser is not configured (FRONA_BROWSER_WS_URL not set)".into())
        })?;

        self.kill_browserless_sessions_for_profile(user_handle, provider)
            .await;

        let ws_url = Self::ws_url_for_profile(config, user_handle, provider)?;
        tracing::debug!("Connecting to browser");

        let timeout = Duration::from_millis(config.connection_timeout_ms);
        let lifetime = BROWSERLESS_SESSION_TIMEOUT.saturating_sub(SELF_EVICT_MARGIN);
        // Browserless can still be booting right after the container starts, which
        // surfaces as a transient connect error; a short retry absorbs that race.
        let mut retries = CONNECT_RETRY_DELAYS.into_iter();
        loop {
            match BrowserConnection::connect(ws_url.as_str(), timeout, lifetime).await {
                Ok(connection) => return Ok(connection),
                Err(error) => {
                    if error.is_retryable_connect_error()
                        && let Some(delay) = retries.next()
                    {
                        tracing::warn!(error = %error, delay = ?delay, "Browser connect failed, retrying");
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Err(AppError::Browser(format!(
                        "Failed to connect to browser: {error}"
                    )));
                }
            }
        }
    }

    pub async fn connection(
        &self,
        user_handle: &crate::core::Handle,
        provider: &str,
    ) -> Result<BrowserConnection, AppError> {
        let key = Self::profile_key(user_handle, provider);

        // Fast path: no lock needed just to reuse an already-alive connection.
        if let Some(conn) = self.sessions.read().await.get(&key).cloned()
            && conn.is_alive()
        {
            return Ok(conn);
        }

        // Serialize the check-then-create-then-insert sequence per profile so
        // concurrent callers for the same (user, provider) queue behind the
        // first connect attempt and reuse its result, instead of racing each
        // other to launch Chrome against the same `--user-data-dir`.
        let lock = self.profile_lock(&key).await;
        let _guard = lock.lock().await;

        if let Some(conn) = self.sessions.read().await.get(&key).cloned() {
            if conn.is_alive() {
                return Ok(conn);
            }
            tracing::warn!(
                user = %user_handle,
                provider = %provider,
                "Cached browser connection is dead; evicting and reconnecting"
            );
        }
        self.sessions.write().await.remove(&key);
        let conn = self.create_connection(user_handle, provider).await?;
        self.sessions.write().await.insert(key, conn.clone());
        Ok(conn)
    }

    fn admin_http_client() -> Client<hyper_util::client::legacy::connect::HttpConnector, Body> {
        Client::builder(TokioExecutor::new()).build_http::<Body>()
    }

    async fn list_browserless_sessions(&self) -> Vec<BrowserlessSession> {
        let Some(config) = self.config.as_ref() else {
            return vec![];
        };
        let client = Self::admin_http_client();

        let sessions_url = match config.http_url(&["sessions"], Some(config.api_token()), &[]) {
            Ok(url) => url,
            Err(_) => {
                tracing::warn!("Invalid Browserless sessions URL");
                return vec![];
            }
        };
        let mut delay = SESSION_LIST_RETRY_MIN_DELAY;
        let mut retries_left = SESSION_LIST_CONNECT_RETRIES;
        let resp = loop {
            let req = match hyper::Request::get(sessions_url.as_str()).body(Body::empty()) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("Failed to build sessions list request: {e}");
                    return vec![];
                }
            };

            match client.request(req).await {
                Ok(r) => break r,
                Err(e) if e.is_connect() && retries_left > 0 => {
                    tracing::debug!(
                        error = %e,
                        delay = ?delay,
                        "Browserless not listening yet, retrying session list"
                    );
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(SESSION_LIST_RETRY_MAX_DELAY);
                    retries_left -= 1;
                }
                Err(e) => {
                    tracing::warn!("Failed to list browserless sessions: {e}");
                    return vec![];
                }
            }
        };

        let body = match http_body_util::Limited::new(resp.into_body(), 4 << 20)
            .collect()
            .await
        {
            Ok(b) => b.to_bytes(),
            Err(e) => {
                tracing::warn!("Failed to read sessions response: {e}");
                return vec![];
            }
        };

        match serde_json::from_slice(&body) {
            Ok(s) => s,
            Err(_) => {
                tracing::warn!("Failed to parse Browserless sessions response");
                vec![]
            }
        }
    }

    /// Only the exact configured profile may be discovered or attached by its owner.
    pub(crate) async fn debugger_sessions(
        &self,
        user_handle: &crate::core::Handle,
        provider: &str,
    ) -> Result<Vec<BrowserlessSession>, AppError> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| AppError::Browser("Browser is not configured".into()))?;
        let expected = config.profile_path(user_handle, provider);
        let sessions = tokio::time::timeout(
            Duration::from_millis(config.connection_timeout_ms),
            self.list_browserless_sessions(),
        )
        .await
        .map_err(|_| AppError::Browser("Browserless session discovery timed out".into()))?;
        Ok(sessions
            .into_iter()
            .filter(|session| {
                session
                    .user_data_dir
                    .as_deref()
                    .is_some_and(|path| std::path::Path::new(path) == expected)
            })
            .collect())
    }

    async fn kill_browserless_session_ids(&self, browser_ids: &[String]) {
        if browser_ids.is_empty() {
            return;
        }
        let Some(config) = self.config.as_ref() else {
            return;
        };
        let client = Self::admin_http_client();

        for id in browser_ids {
            let kill_url = match config.http_url(&["kill", id], Some(config.api_token()), &[]) {
                Ok(url) => url,
                Err(_) => {
                    tracing::warn!("Invalid Browserless kill URL");
                    continue;
                }
            };
            let req = match hyper::Request::get(kill_url.as_str()).body(Body::empty()) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("Failed to build kill request: {e}");
                    continue;
                }
            };
            if let Err(e) = client.request(req).await {
                tracing::warn!(browser_id = %id, "Failed to kill session: {e}");
            }
        }
        tracing::info!(count = browser_ids.len(), "Killed browserless sessions");
    }

    async fn kill_browserless_sessions_for_profile(
        &self,
        user_handle: &crate::core::Handle,
        provider: &str,
    ) {
        let Some(config) = self.config.as_ref() else {
            return;
        };
        let profile_path = config.profile_path(user_handle, provider);
        let profile_str = profile_path.to_string_lossy();

        let sessions = self.list_browserless_sessions().await;
        let ids: Vec<String> = sessions
            .into_iter()
            .filter(|s| {
                s.session_type.as_deref() == Some("browser")
                    && s.user_data_dir.as_deref().is_some_and(|d| {
                        profile_str.ends_with(d) || d.ends_with(profile_str.as_ref())
                    })
            })
            .map(|s| s.browser_id)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        self.kill_browserless_session_ids(&ids).await;
    }

    pub async fn kill_all_sessions(&self) {
        let sessions = self.list_browserless_sessions().await;
        let ids: Vec<String> = sessions
            .into_iter()
            .filter(|s| s.session_type.as_deref() == Some("browser"))
            .map(|s| s.browser_id)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        self.kill_browserless_session_ids(&ids).await;
    }

    pub async fn close_session(
        &self,
        user_handle: &crate::core::Handle,
        provider: &str,
    ) -> Result<(), AppError> {
        let key = Self::profile_key(user_handle, provider);
        let lock = self.profile_lock(&key).await;
        let _guard = lock.lock().await;
        let connection = self.sessions.write().await.remove(&key);
        if let Some(conn) = connection {
            conn.disconnect()
                .await
                .map_err(|e| AppError::Browser(format!("Failed to close session: {e}")))?;
        }
        Ok(())
    }
}

/// No outer timeout: the handler-loop catches transport errors in ms and
/// chromiumoxide's `request_timeout` bounds individual CDP calls.
pub(crate) async fn run_with_reconnect<T, F, Fut>(
    mgr: &BrowserSessionManager,
    ctx: &crate::tool::InferenceContext,
    provider: &str,
    op: F,
) -> Result<T, AppError>
where
    F: Fn(BrowserConnection) -> Fut,
    Fut: std::future::Future<Output = Result<T, frona_browser::Error>>,
{
    let operation = async {
        let conn = mgr.connection(&ctx.user.handle, provider).await?;
        match op(conn.clone()).await {
            Ok(value) => Ok(value),
            Err(error) if error.is_disconnect() => {
                tracing::warn!("Browser session disconnected, reconnecting");
                // Invalidate the failed instance, not a replacement another caller created.
                conn.mark_dead();
                let conn = mgr.connection(&ctx.user.handle, provider).await?;
                op(conn)
                    .await
                    .map_err(|error| AppError::Browser(error.to_string()))
            }
            Err(error) => Err(AppError::Browser(error.to_string())),
        }
    };
    tokio::select! {
        biased;
        _ = ctx.cancel_token.cancelled() => Err(AppError::Tool("Browser operation cancelled".into())),
        result = operation => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(ws_url: &str) -> BrowserConfig {
        BrowserConfig {
            ws_url: ws_url.into(),
            api_token: None,
            profiles_path: "/profiles".into(),
            connection_timeout_ms: 30000,
        }
    }

    #[test]
    fn ws_url_inserts_root_path_before_query_string() {
        let url = BrowserSessionManager::ws_url_for_profile(
            &cfg("ws://browserless:3333"),
            &crate::handle!("alice"),
            "openai",
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "ws://browserless:3333/?--user-data-dir=%2Fprofiles%2Falice%2Fopenai&timeout=86400000"
        );
    }

    #[test]
    fn ws_url_normalises_trailing_slash_on_base() {
        let url = BrowserSessionManager::ws_url_for_profile(
            &cfg("ws://browserless:3333/"),
            &crate::handle!("alice"),
            "openai",
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "ws://browserless:3333/?--user-data-dir=%2Fprofiles%2Falice%2Fopenai&timeout=86400000"
        );
    }

    #[test]
    fn ws_url_appends_token_when_configured() {
        let mut config = cfg("ws://browserless:3333");
        config.api_token = Some("secret".into());
        let url =
            BrowserSessionManager::ws_url_for_profile(&config, &crate::handle!("alice"), "openai")
                .unwrap();
        assert_eq!(
            url.as_str(),
            "ws://browserless:3333/?token=secret&--user-data-dir=%2Fprofiles%2Falice%2Fopenai&timeout=86400000"
        );
    }

    #[tokio::test]
    async fn historical_profile_locks_are_pruned() {
        let manager = BrowserSessionManager::new(None);
        let active = manager.profile_lock("active").await;
        for i in 0..100 {
            drop(manager.profile_lock(&format!("profile{i}")).await);
            assert!(manager.profile_locks.lock().await.len() <= 2);
        }
        assert!(Arc::ptr_eq(&active, &manager.profile_lock("active").await));
        drop(active);
        drop(manager.profile_lock("last").await);
        assert_eq!(manager.profile_locks.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn a_second_live_viewer_replaces_the_first() {
        let mgr = BrowserSessionManager::new(None);
        let alice = crate::handle!("alice");
        let first = mgr.claim_live_view(&alice, "google");
        let _second = mgr.claim_live_view(&alice, "google");
        tokio::time::timeout(Duration::from_secs(1), first.replaced.notified())
            .await
            .expect("first viewer should be told it was replaced");
    }

    #[tokio::test]
    async fn live_viewers_on_other_profiles_are_left_alone() {
        let mgr = BrowserSessionManager::new(None);
        let alice = crate::handle!("alice");
        let first = mgr.claim_live_view(&alice, "google");
        let _other = mgr.claim_live_view(&alice, "github");
        let _bob = mgr.claim_live_view(&crate::handle!("bob"), "google");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), first.replaced.notified())
                .await
                .is_err()
        );
    }

    #[test]
    fn a_replaced_viewer_dropping_late_keeps_the_new_claim() {
        let mgr = BrowserSessionManager::new(None);
        let alice = crate::handle!("alice");
        let first = mgr.claim_live_view(&alice, "google");
        let second = mgr.claim_live_view(&alice, "google");
        drop(first);
        let viewers = mgr.live_viewers.lock().unwrap();
        assert_eq!(
            viewers.get("alice/google").map(|(id, _)| *id),
            Some(second.id)
        );
    }
}
