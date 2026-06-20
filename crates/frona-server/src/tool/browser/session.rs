use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use http_body_util::BodyExt;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use tokio::sync::RwLock;

use crate::core::config::BrowserConfig;
use crate::core::error::AppError;
use frona_browser::BrowserConnection;

/// Browserless treats `<=0` as `0ms` and 408s the upgrade instantly, so we
/// pass a long positive instead to defeat its 30s per-job default.
const BROWSERLESS_SESSION_TIMEOUT: Duration = Duration::from_secs(24 * 3600);
/// Self-evict before Browserless's hard limit so the next request rebuilds
/// cleanly instead of hitting a forced close mid-op.
const SELF_EVICT_MARGIN: Duration = Duration::from_secs(60);

#[derive(serde::Deserialize)]
struct BrowserlessSession {
    #[serde(rename = "browserId")]
    browser_id: String,
    #[serde(rename = "type")]
    session_type: Option<String>,
    #[serde(rename = "userDataDir", default)]
    user_data_dir: Option<String>,
}

#[derive(Clone)]
pub struct BrowserSessionManager {
    config: Option<BrowserConfig>,
    sessions: Arc<RwLock<HashMap<String, BrowserConnection>>>,
}

impl BrowserSessionManager {
    pub fn new(config: Option<BrowserConfig>) -> Self {
        Self {
            config,
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn config(&self) -> Option<&BrowserConfig> {
        self.config.as_ref()
    }

    fn profile_key(user_handle: &crate::core::Handle, provider: &str) -> String {
        format!("{user_handle}/{provider}")
    }

    /// Inserts `/` before the query string — browserless v2 returns HTTP 400 without it.
    fn ws_url_for_profile(config: &BrowserConfig, user_handle: &crate::core::Handle, provider: &str) -> String {
        let user_data_dir = config.profile_path(user_handle, provider);
        let base = config.ws_url.trim_end_matches('/');
        format!(
            "{}/?--user-data-dir={}&timeout={}",
            base,
            user_data_dir.display(),
            BROWSERLESS_SESSION_TIMEOUT.as_millis()
        )
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

        let ws_url = Self::ws_url_for_profile(config, user_handle, provider);
        tracing::debug!(ws_url = %ws_url, browserless_ws_url = %config.ws_url, "Connecting to browser");

        let timeout = Duration::from_millis(config.connection_timeout_ms);
        let lifetime = BROWSERLESS_SESSION_TIMEOUT.saturating_sub(SELF_EVICT_MARGIN);
        BrowserConnection::connect(&ws_url, timeout, lifetime)
            .await
            .map_err(|e| AppError::Browser(format!("Failed to connect to browser: {e}")))
    }

    pub async fn connection(
        &self,
        user_handle: &crate::core::Handle,
        provider: &str,
    ) -> Result<BrowserConnection, AppError> {
        let key = Self::profile_key(user_handle, provider);
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

    pub async fn reconnect(
        &self,
        user_handle: &crate::core::Handle,
        provider: &str,
    ) -> Result<BrowserConnection, AppError> {
        let key = Self::profile_key(user_handle, provider);
        self.sessions.write().await.remove(&key);
        self.connection(user_handle, provider).await
    }

    fn admin_http_client() -> Client<hyper_util::client::legacy::connect::HttpConnector, Body> {
        Client::builder(TokioExecutor::new()).build_http::<Body>()
    }

    async fn list_browserless_sessions(&self) -> Vec<BrowserlessSession> {
        let Some(config) = self.config.as_ref() else {
            return vec![];
        };
        let http_base = config.http_base_url();
        let client = Self::admin_http_client();

        let sessions_url = format!("{http_base}/sessions?token={}", config.api_token());
        let req = match hyper::Request::get(&sessions_url).body(Body::empty()) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Failed to build sessions list request: {e}");
                return vec![];
            }
        };

        let resp = match client.request(req).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Failed to list browserless sessions: {e}");
                return vec![];
            }
        };

        let body = match resp.into_body().collect().await {
            Ok(b) => b.to_bytes(),
            Err(e) => {
                tracing::warn!("Failed to read sessions response: {e}");
                return vec![];
            }
        };

        match serde_json::from_slice(&body) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to parse sessions response: {e}");
                vec![]
            }
        }
    }

    async fn kill_browserless_session_ids(&self, browser_ids: &[String]) {
        if browser_ids.is_empty() {
            return;
        }
        let Some(config) = self.config.as_ref() else {
            return;
        };
        let http_base = config.http_base_url();
        let client = Self::admin_http_client();

        for id in browser_ids {
            let kill_url = format!("{http_base}/kill/{id}?token={}", config.api_token());
            let req = match hyper::Request::get(&kill_url).body(Body::empty()) {
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

    async fn kill_browserless_sessions_for_profile(&self, user_handle: &crate::core::Handle, provider: &str) {
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

    pub async fn close_session(&self, user_handle: &crate::core::Handle, provider: &str) -> Result<(), AppError> {
        let key = Self::profile_key(user_handle, provider);
        let mut sessions = self.sessions.write().await;
        if let Some(conn) = sessions.remove(&key) {
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
    user_handle: &crate::core::Handle,
    provider: &str,
    op: F,
) -> Result<T, AppError>
where
    F: Fn(BrowserConnection) -> Fut,
    Fut: std::future::Future<Output = Result<T, frona_browser::Error>>,
{
    let conn = mgr.connection(user_handle, provider).await?;
    match op(conn).await {
        Ok(v) => Ok(v),
        Err(e) if e.is_disconnect() => {
            tracing::warn!("Browser session disconnected, reconnecting");
            let conn = mgr.reconnect(user_handle, provider).await?;
            op(conn).await.map_err(|e| AppError::Browser(e.to_string()))
        }
        Err(e) => Err(AppError::Browser(e.to_string())),
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
        let url = BrowserSessionManager::ws_url_for_profile(&cfg("ws://browserless:3333"), &crate::handle!("alice"), "openai");
        assert_eq!(
            url,
            "ws://browserless:3333/?--user-data-dir=/profiles/alice/openai&timeout=86400000"
        );
    }

    #[test]
    fn ws_url_normalises_trailing_slash_on_base() {
        let url = BrowserSessionManager::ws_url_for_profile(&cfg("ws://browserless:3333/"), &crate::handle!("alice"), "openai");
        assert_eq!(
            url,
            "ws://browserless:3333/?--user-data-dir=/profiles/alice/openai&timeout=86400000"
        );
    }
}
