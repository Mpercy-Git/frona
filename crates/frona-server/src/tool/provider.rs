use serde::Serialize;

/// A source of tools that an agent can use. Built-in providers (browser, cli, search, etc.)
/// are defined statically; MCP providers are derived from installed `McpServer` rows.
#[derive(Debug, Clone, Serialize)]
pub struct ToolProvider {
    pub id: String,
    pub display_name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub kind: ToolProviderKind,
    pub status: ToolProviderStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolProviderKind {
    Builtin,
    Mcp {
        server_id: String,
        repository_url: Option<String>,
        version: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ToolProviderStatus {
    Available,
    Unavailable { reason: String },
}

/// Static spec for a built-in provider. Kept as a `&'static` table so the catalog
/// has zero runtime cost; converted to owned `ToolProvider`s on demand by `builtin_providers()`.
pub struct BuiltinSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub configurable: bool,
}

pub const BUILTIN_PROVIDERS: &[BuiltinSpec] = &[
    BuiltinSpec {
        id: "human_in_the_loop",
        display_name: "human_in_the_loop",
        description: "Ask the user clarifying questions or request user takeover when the agent cannot proceed on its own.",
        configurable: true,
    },
    BuiltinSpec {
        id: "messaging",
        display_name: "messaging",
        description: "Send markdown messages and attachments to the user in the active chat.",
        configurable: true,
    },
    BuiltinSpec {
        id: "file",
        display_name: "file",
        description: "Produce files in the agent's workspace and attach them to messages or task results.",
        configurable: true,
    },
    BuiltinSpec {
        id: "identity",
        display_name: "identity",
        description: "Read and update the agent's own identity attributes — name, persona, preferences.",
        configurable: true,
    },
    BuiltinSpec {
        id: "memory",
        display_name: "memory",
        description: "Store long-term memories about the user and the agent that persist across chats.",
        configurable: true,
    },
    BuiltinSpec {
        id: "agent",
        display_name: "agent",
        description: "Create new agents. Only available to the system agent.",
        configurable: true,
    },
    BuiltinSpec {
        id: "task",
        display_name: "task",
        description: "Create, list, schedule, defer, complete, fail and delete background tasks, optionally delegated to other agents.",
        configurable: true,
    },
    BuiltinSpec {
        id: "browser",
        display_name: "browser",
        description: "Headless browser automation: navigate pages, click, type, extract content, and take screenshots.",
        configurable: true,
    },
    BuiltinSpec {
        id: "web_fetch",
        display_name: "web_fetch",
        description: "Fetch and convert the contents of a web page to clean markdown for further reasoning.",
        configurable: true,
    },
    BuiltinSpec {
        id: "search",
        display_name: "search",
        description: "Search the web via the configured search provider and return ranked results with snippets.",
        configurable: true,
    },
    BuiltinSpec {
        id: "heartbeat",
        display_name: "heartbeat",
        description: "Schedule periodic wake-ups so the agent can run recurring checks or background loops.",
        configurable: true,
    },
    BuiltinSpec {
        id: "credentials",
        display_name: "credentials",
        description: "Request access to credentials in the user's vault with an explicit approval prompt.",
        configurable: true,
    },
    BuiltinSpec {
        id: "app",
        display_name: "app",
        description: "Deploy, start, stop, restart and destroy agent-published web apps.",
        configurable: true,
    },
    BuiltinSpec {
        id: "voice_call",
        display_name: "voice_call",
        description: "Make outbound voice calls, send DTMF digits, and hang up active calls.",
        configurable: true,
    },
];

pub fn builtin_providers() -> Vec<ToolProvider> {
    BUILTIN_PROVIDERS
        .iter()
        .map(|s| ToolProvider {
            id: s.id.to_string(),
            display_name: s.display_name.to_string(),
            description: Some(s.description.to_string()),
            icon: None,
            kind: ToolProviderKind::Builtin,
            status: ToolProviderStatus::Available,
        })
        .collect()
}

pub fn is_configurable_builtin(provider_id: &str) -> bool {
    BUILTIN_PROVIDERS
        .iter()
        .any(|s| s.id == provider_id && s.configurable)
}

pub fn is_builtin_provider(provider_id: &str) -> bool {
    BUILTIN_PROVIDERS.iter().any(|s| s.id == provider_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_duplicate_provider_ids() {
        let mut ids: Vec<&str> = BUILTIN_PROVIDERS.iter().map(|s| s.id).collect();
        ids.sort();
        let len_before = ids.len();
        ids.dedup();
        assert_eq!(len_before, ids.len(), "duplicate provider ids in BUILTIN_PROVIDERS");
    }

    #[test]
    fn provider_ids_are_snake_case() {
        for spec in BUILTIN_PROVIDERS {
            assert!(
                spec.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "provider id `{}` must be snake_case",
                spec.id
            );
        }
    }

    #[test]
    fn builtin_providers_returns_all() {
        let providers = builtin_providers();
        assert_eq!(providers.len(), BUILTIN_PROVIDERS.len());
        for p in &providers {
            assert!(matches!(p.kind, ToolProviderKind::Builtin));
            assert!(matches!(p.status, ToolProviderStatus::Available));
        }
    }
}
