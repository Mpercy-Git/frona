use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use surrealdb::types::SurrealValue;

use crate::Entity;
use crate::core::Handle;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "lowercase")]
#[surreal(crate = "surrealdb::types", lowercase)]
pub enum McpRuntime {
    Npm,
    Pypi,
    Binary,
}

impl std::fmt::Display for McpRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Npm => write!(f, "npm"),
            Self::Pypi => write!(f, "pypi"),
            Self::Binary => write!(f, "binary"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct McpPackage {
    pub runtime: McpRuntime,
    /// Package name, or an absolute path when `runtime == Binary`.
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "lowercase")]
#[surreal(crate = "surrealdb::types", lowercase)]
pub enum McpServerStatus {
    Installed,
    Starting,
    Running,
    Stopped,
    Failed,
}

impl std::fmt::Display for McpServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Installed => write!(f, "installed"),
            Self::Starting => write!(f, "starting"),
            Self::Running => write!(f, "running"),
            Self::Stopped => write!(f, "stopped"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct McpServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct CachedMcpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub enum TransportConfig {
    Stdio {
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Http {
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port_env_var: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint_path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
}

impl TransportConfig {
    pub fn args(&self) -> &[String] {
        match self {
            Self::Stdio { args, .. } | Self::Http { args, .. } => args,
        }
    }

    pub fn env(&self) -> &BTreeMap<String, String> {
        match self {
            Self::Stdio { env, .. } | Self::Http { env, .. } => env,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "mcp_server")]
pub struct McpServer {
    pub id: String,
    pub user_id: String,

    /// Per-user-unique. Appears in Cedar UIDs (`Mcp::"{user_handle}/{handle}"`),
    /// the `mcp__{handle}__{tool}` tool-id prefix, and the workspace dir name.
    pub handle: Handle,
    pub display_name: String,
    pub description: Option<String>,
    pub repository_url: Option<String>,
    pub registry_id: Option<String>,
    pub server_info: Option<McpServerInfo>,

    pub package: McpPackage,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub transports: Vec<TransportConfig>,
    /// One of: "stdio", "streamable-http", "sse".
    pub active_transport: String,
    pub env: BTreeMap<String, String>,
    pub status: McpServerStatus,
    pub tool_cache: Vec<CachedMcpTool>,
    pub workspace_dir: String,

    pub installed_at: DateTime<Utc>,
    pub last_started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialBinding {
    pub connection_id: String,
    pub vault_item_id: String,
    pub env_var: String,
    pub field: crate::credential::vault::models::VaultField,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServerInstall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name_override: Option<String>,
    /// When absent, the install service derives one via `sanitize_to_handle`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<Handle>,
    #[serde(default)]
    pub credentials: Vec<CredentialBinding>,
    #[serde(default)]
    pub extra_env: BTreeMap<String, String>,
    #[serde(default)]
    pub sandbox_policy: Option<crate::policy::sandbox::SandboxPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServerUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Vec<CredentialBinding>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_env: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_policy: Option<crate::policy::sandbox::SandboxPolicy>,
    pub active_transport: Option<String>,
}

/// Derive a `Handle` from an arbitrary string (display name or reverse-DNS
/// registry id). Validated against the full 5492-entry MCP registry: 0
/// invalid outputs. The `io.github.` strip drops collision count from 641
/// (tail-only) to 9.
pub fn sanitize_to_handle(input: &str) -> Handle {
    let stripped = strip_io_github_prefix(input);

    let mut out = String::with_capacity(stripped.len());
    let mut last: Option<char> = None;
    for c in stripped.chars() {
        let mapped = match c.to_ascii_lowercase() {
            ch if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-' => Some(ch),
            '.' | '/' | ' ' | '\t' => Some('-'),
            _ => Some('-'),
        };
        if let Some(ch) = mapped {
            if matches!(ch, '-' | '_') && matches!(last, Some('-') | Some('_')) {
                continue;
            }
            out.push(ch);
            last = Some(ch);
        }
    }
    let mut s = out.trim_matches(|c: char| c == '-' || c == '_').to_string();

    if s.is_empty() {
        let uuid = crate::core::repository::new_id();
        let short = uuid.chars().take(6).collect::<String>();
        s = format!("mcp-{short}");
    }

    if s.chars().count() == 1 {
        s = format!("m{s}");
    }

    if !s.starts_with(|c: char| c.is_ascii_lowercase()) {
        s = format!("m-{s}");
    }

    if s.len() > 32 {
        s.truncate(32);
        s = s.trim_end_matches(['-', '_']).to_string();
    }

    Handle::try_new(s)
        .expect("sanitize_to_handle produced a value that fails Handle validation — bug in the sanitizer")
}

fn strip_io_github_prefix(input: &str) -> &str {
    for prefix in ["io.github.", "io_github_"] {
        if input.len() >= prefix.len() && input[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return &input[prefix.len()..];
        }
    }
    input
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_to_handle_basic() {
        assert_eq!(sanitize_to_handle("Google Workspace").as_str(), "google-workspace");
        assert_eq!(sanitize_to_handle("GitHub").as_str(), "github");
        assert_eq!(sanitize_to_handle("my-tool-2").as_str(), "my-tool-2");
    }

    #[test]
    fn sanitize_to_handle_collapses_runs() {
        assert_eq!(sanitize_to_handle("hello   world").as_str(), "hello-world");
        assert_eq!(sanitize_to_handle("foo--bar__baz").as_str(), "foo-bar_baz");
    }

    #[test]
    fn sanitize_to_handle_trims_edges() {
        assert_eq!(sanitize_to_handle("  spaced  ").as_str(), "spaced");
        assert_eq!(sanitize_to_handle("___internal___").as_str(), "internal");
    }

    #[test]
    fn sanitize_to_handle_strips_io_github_prefix() {
        assert_eq!(
            sanitize_to_handle("io.github.bytedance/mcp-server-browser").as_str(),
            "bytedance-mcp-server-browser"
        );
        assert_eq!(
            sanitize_to_handle("io.github.ruvnet/claude-flow").as_str(),
            "ruvnet-claude-flow"
        );
        assert_eq!(
            sanitize_to_handle("IO.GITHUB.upstash/context7").as_str(),
            "upstash-context7"
        );
    }

    #[test]
    fn sanitize_to_handle_keeps_other_reverse_dns_prefixes() {
        assert_eq!(
            sanitize_to_handle("com.mcparmory/foo-bar").as_str(),
            "com-mcparmory-foo-bar"
        );
    }

    #[test]
    fn sanitize_to_handle_truncates_long_input() {
        let long = sanitize_to_handle("io.github.ChromeDevTools/chrome-devtools-mcp");
        assert!(long.as_str().len() <= 32);
        assert_eq!(long.as_str(), "chromedevtools-chrome-devtools-m");
    }

    #[test]
    fn sanitize_to_handle_truncation_strips_trailing_separator() {
        let h = sanitize_to_handle("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa--xyz");
        assert!(!h.as_str().ends_with('-'));
        assert!(!h.as_str().ends_with('_'));
    }

    #[test]
    fn sanitize_to_handle_empty_uses_uuid_fallback() {
        let h = sanitize_to_handle("");
        assert!(h.as_str().starts_with("mcp-"));
        assert!(h.as_str().len() >= 5);
    }

    #[test]
    fn sanitize_to_handle_all_special_uses_uuid_fallback() {
        let h = sanitize_to_handle("!@#$%");
        assert!(h.as_str().starts_with("mcp-"));
    }

    #[test]
    fn sanitize_to_handle_single_char_pads() {
        let h = sanitize_to_handle("x");
        assert_eq!(h.as_str(), "mx");
    }

    #[test]
    fn sanitize_to_handle_digit_leading_prefixed() {
        let h = sanitize_to_handle("2024-tool");
        assert_eq!(h.as_str(), "m-2024-tool");
    }

    #[test]
    fn sanitize_to_handle_unicode_dropped() {
        assert_eq!(sanitize_to_handle("héllo").as_str(), "h-llo");
        let h = sanitize_to_handle("日本語");
        assert!(h.as_str().starts_with("mcp-"));
    }

    #[test]
    fn runtime_display_matches_serde() {
        assert_eq!(McpRuntime::Npm.to_string(), "npm");
        assert_eq!(McpRuntime::Pypi.to_string(), "pypi");
        assert_eq!(McpRuntime::Binary.to_string(), "binary");
    }

    #[test]
    fn runtime_serde_round_trip() {
        let npm = serde_json::to_string(&McpRuntime::Npm).unwrap();
        assert_eq!(npm, "\"npm\"");
        let parsed: McpRuntime = serde_json::from_str("\"pypi\"").unwrap();
        assert_eq!(parsed, McpRuntime::Pypi);
    }

    #[test]
    fn status_display_matches_serde() {
        assert_eq!(McpServerStatus::Installed.to_string(), "installed");
        assert_eq!(McpServerStatus::Running.to_string(), "running");
        assert_eq!(McpServerStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn status_serde_round_trip() {
        let json = serde_json::to_string(&McpServerStatus::Running).unwrap();
        assert_eq!(json, "\"running\"");
        let parsed: McpServerStatus = serde_json::from_str("\"installed\"").unwrap();
        assert_eq!(parsed, McpServerStatus::Installed);
    }

    #[test]
    fn install_dto_accepts_registry_id_only() {
        let json = serde_json::json!({ "registry_id": "io.github.foo/bar" });
        let parsed: McpServerInstall = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.registry_id.as_deref(), Some("io.github.foo/bar"));
        assert!(parsed.manifest.is_none());
        assert!(parsed.display_name_override.is_none());
    }

    #[test]
    fn install_dto_accepts_manifest_only() {
        let json = serde_json::json!({
            "manifest": { "name": "foo", "packages": [] }
        });
        let parsed: McpServerInstall = serde_json::from_value(json).unwrap();
        assert!(parsed.manifest.is_some());
    }
}
