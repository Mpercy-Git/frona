use chrono::{DateTime, Utc};
use crate::Entity;
use crate::core::Handle;
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "lowercase")]
#[surreal(crate = "surrealdb::types", lowercase)]
pub enum AppStatus {
    Starting,
    Running,
    Stopped,
    Failed,
    Serving,
    Hibernated,
}

impl std::fmt::Display for AppStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Starting => write!(f, "starting"),
            Self::Running => write!(f, "running"),
            Self::Stopped => write!(f, "stopped"),
            Self::Failed => write!(f, "failed"),
            Self::Serving => write!(f, "serving"),
            Self::Hibernated => write!(f, "hibernated"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "app")]
pub struct App {
    pub id: String,
    pub agent_id: String,
    pub user_id: String,
    pub handle: Handle,
    pub name: String,
    pub description: Option<String>,
    pub kind: String,
    pub command: Option<String>,
    pub static_dir: Option<String>,
    pub port: Option<u16>,
    pub status: AppStatus,
    pub pid: Option<u32>,
    pub manifest: serde_json::Value,
    pub chat_id: String,
    pub crash_fix_attempts: u32,
    pub last_accessed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppManifest {
    /// Served at `/apps/{handle}/`; source at `apps/{handle}/` in the workspace.
    pub handle: Handle,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_check: Option<HealthCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expose: Option<bool>,
    /// Reconciled into Cedar on deploy/update; Cedar is the runtime source of truth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_policy: Option<crate::policy::sandbox::SandboxPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Vec<CredentialRequest>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hibernate: Option<bool>,
}

impl AppManifest {
    pub fn effective_kind(&self) -> &str {
        self.kind.as_deref().unwrap_or("service")
    }

    pub fn effective_expose(&self) -> bool {
        self.expose.unwrap_or(true)
    }

    pub fn effective_hibernate(&self) -> bool {
        self.hibernate.unwrap_or(true)
    }

    pub fn effective_restart_policy(&self) -> &str {
        self.restart_policy.as_deref().unwrap_or("on_failure")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkDestination {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CredentialRequest {
    pub query: String,
    pub reason: String,
    pub env_var_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    #[serde(default = "default_health_path")]
    pub path: String,
    #[serde(default)]
    pub interval_secs: Option<u64>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub initial_delay_secs: Option<u64>,
    #[serde(default)]
    pub failure_threshold: Option<u32>,
}

fn default_health_path() -> String {
    "/".to_string()
}

impl HealthCheck {
    pub fn effective_interval(&self) -> u64 {
        self.interval_secs.unwrap_or(10)
    }

    pub fn effective_timeout(&self) -> u64 {
        self.timeout_secs.unwrap_or(2)
    }

    pub fn effective_initial_delay(&self) -> u64 {
        self.initial_delay_secs.unwrap_or(5)
    }

    pub fn effective_failure_threshold(&self) -> u32 {
        self.failure_threshold.unwrap_or(3)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    #[serde(default)]
    pub memory_mb: Option<u64>,
    #[serde(default)]
    pub cpu_shares: Option<u64>,
    #[serde(default)]
    pub max_pids: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppResponse {
    pub id: String,
    pub agent_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub static_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub status: AppStatus,
    pub manifest: serde_json::Value,
    pub handle: String,
    pub url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<App> for AppResponse {
    fn from(app: App) -> Self {
        let expose = serde_json::from_value::<AppManifest>(app.manifest.clone())
            .map(|m| m.effective_expose())
            .unwrap_or(true);

        let url = if expose
            && matches!(
                app.status,
                AppStatus::Running | AppStatus::Serving | AppStatus::Hibernated
            )
        {
            Some(format!("/apps/{}/", app.handle))
        } else {
            None
        };

        Self {
            id: app.id,
            agent_id: app.agent_id,
            handle: app.handle.to_string(),
            name: app.name,
            description: app.description,
            kind: app.kind,
            command: app.command,
            static_dir: app.static_dir,
            port: app.port,
            status: app.status,
            manifest: app.manifest,
            url,
            created_at: app.created_at,
            updated_at: app.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_manifest() -> AppManifest {
        serde_json::from_value(serde_json::json!({
            "handle": "test",
            "name": "Test"
        }))
        .unwrap()
    }

    fn make_app(status: AppStatus, manifest: serde_json::Value) -> App {
        let now = Utc::now();
        App {
            id: "app-1".to_string(),
            agent_id: "agent-1".to_string(),
            user_id: "user-1".to_string(),
            handle: crate::handle!("test-app"),
            name: "Test App".to_string(),
            description: None,
            kind: "service".to_string(),
            command: Some("python app.py".to_string()),
            static_dir: None,
            port: Some(4000),
            status,
            pid: Some(1234),
            manifest,
            chat_id: "test-chat".to_string(),
            crash_fix_attempts: 0,
            last_accessed_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn app_status_display() {
        assert_eq!(AppStatus::Starting.to_string(), "starting");
        assert_eq!(AppStatus::Running.to_string(), "running");
        assert_eq!(AppStatus::Stopped.to_string(), "stopped");
        assert_eq!(AppStatus::Failed.to_string(), "failed");
        assert_eq!(AppStatus::Serving.to_string(), "serving");
        assert_eq!(AppStatus::Hibernated.to_string(), "hibernated");
    }

    #[test]
    fn manifest_effective_kind_defaults_to_service() {
        let m = minimal_manifest();
        assert_eq!(m.effective_kind(), "service");
    }

    #[test]
    fn manifest_effective_kind_uses_provided_value() {
        let m: AppManifest = serde_json::from_value(serde_json::json!({
            "handle": "test", "name": "Test", "kind": "static"
        }))
        .unwrap();
        assert_eq!(m.effective_kind(), "static");
    }

    #[test]
    fn manifest_effective_expose_defaults_to_true() {
        assert!(minimal_manifest().effective_expose());
    }

    #[test]
    fn manifest_effective_expose_respects_false() {
        let m: AppManifest = serde_json::from_value(serde_json::json!({
            "handle": "test", "name": "Test", "expose": false
        }))
        .unwrap();
        assert!(!m.effective_expose());
    }

    #[test]
    fn manifest_effective_hibernate_defaults_to_true() {
        assert!(minimal_manifest().effective_hibernate());
    }

    #[test]
    fn manifest_effective_hibernate_respects_false() {
        let m: AppManifest = serde_json::from_value(serde_json::json!({
            "handle": "test", "name": "Test", "hibernate": false
        }))
        .unwrap();
        assert!(!m.effective_hibernate());
    }

    #[test]
    fn manifest_effective_restart_policy_defaults_to_on_failure() {
        assert_eq!(minimal_manifest().effective_restart_policy(), "on_failure");
    }

    #[test]
    fn manifest_effective_restart_policy_uses_provided() {
        let m: AppManifest = serde_json::from_value(serde_json::json!({
            "handle": "test", "name": "Test", "restart_policy": "always"
        }))
        .unwrap();
        assert_eq!(m.effective_restart_policy(), "always");
    }

    #[test]
    fn health_check_effective_defaults() {
        let hc: HealthCheck = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(hc.effective_interval(), 10);
        assert_eq!(hc.effective_timeout(), 2);
        assert_eq!(hc.effective_initial_delay(), 5);
        assert_eq!(hc.effective_failure_threshold(), 3);
        assert_eq!(hc.path, "/");
    }

    #[test]
    fn health_check_effective_uses_provided() {
        let hc: HealthCheck = serde_json::from_value(serde_json::json!({
            "path": "/healthz",
            "interval_secs": 30,
            "timeout_secs": 5,
            "initial_delay_secs": 10,
            "failure_threshold": 5
        }))
        .unwrap();
        assert_eq!(hc.effective_interval(), 30);
        assert_eq!(hc.effective_timeout(), 5);
        assert_eq!(hc.effective_initial_delay(), 10);
        assert_eq!(hc.effective_failure_threshold(), 5);
        assert_eq!(hc.path, "/healthz");
    }

    #[test]
    fn app_response_url_present_when_running_and_exposed() {
        let manifest = serde_json::json!({"handle": "test", "name": "Test"});
        let app = make_app(AppStatus::Running, manifest);
        let resp = AppResponse::from(app);
        assert_eq!(resp.url, Some("/apps/test-app/".to_string()));
    }

    #[test]
    fn app_response_url_present_when_serving() {
        let manifest = serde_json::json!({"handle": "test", "name": "Test"});
        let app = make_app(AppStatus::Serving, manifest);
        let resp = AppResponse::from(app);
        assert!(resp.url.is_some());
    }

    #[test]
    fn app_response_url_present_when_hibernated() {
        let manifest = serde_json::json!({"handle": "test", "name": "Test"});
        let app = make_app(AppStatus::Hibernated, manifest);
        let resp = AppResponse::from(app);
        assert!(resp.url.is_some());
    }

    #[test]
    fn app_response_url_none_when_stopped() {
        let manifest = serde_json::json!({"handle": "test", "name": "Test"});
        let app = make_app(AppStatus::Stopped, manifest);
        let resp = AppResponse::from(app);
        assert!(resp.url.is_none());
    }

    #[test]
    fn app_response_url_none_when_failed() {
        let manifest = serde_json::json!({"handle": "test", "name": "Test"});
        let app = make_app(AppStatus::Failed, manifest);
        let resp = AppResponse::from(app);
        assert!(resp.url.is_none());
    }

    #[test]
    fn app_response_url_none_when_expose_false() {
        let manifest = serde_json::json!({"handle": "test", "name": "Test", "expose": false});
        let app = make_app(AppStatus::Running, manifest);
        let resp = AppResponse::from(app);
        assert!(resp.url.is_none());
    }

    #[test]
    fn app_response_url_defaults_exposed_on_invalid_manifest() {
        let app = make_app(AppStatus::Running, serde_json::json!("not an object"));
        let resp = AppResponse::from(app);
        assert!(resp.url.is_some());
    }
}
