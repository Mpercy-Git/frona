use std::collections::HashMap;
use std::sync::Arc;

use crate::agent::harness::Harness;
use crate::auth::User;

use super::Command;

#[derive(Default)]
pub struct CommandRegistry {
    static_handlers: HashMap<String, Arc<dyn Command>>,
    switch_agent_fallback: Option<Arc<dyn Command>>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Re-registering an existing name replaces the prior handler.
    pub fn register(&mut self, cmd: Arc<dyn Command>) -> &mut Self {
        self.static_handlers.insert(cmd.name().to_string(), cmd);
        self
    }

    pub fn with_switch_agent_fallback(&mut self, handler: Arc<dyn Command>) -> &mut Self {
        self.switch_agent_fallback = Some(handler);
        self
    }

    /// Static handlers only — does NOT apply the agent-handle fallback.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Command>> {
        self.static_handlers.get(name).cloned()
    }

    pub async fn resolve(
        &self,
        name: &str,
        harness: &Harness,
        user: &User,
    ) -> Option<Arc<dyn Command>> {
        if let Some(cmd) = self.static_handlers.get(name) {
            return Some(cmd.clone());
        }
        let fallback = self.switch_agent_fallback.as_ref()?;
        if harness
            .agent_service
            .find_by_handle(&user.id, name)
            .await
            .ok()
            .flatten()
            .is_some()
        {
            return Some(fallback.clone());
        }
        None
    }

    /// Static handlers only, in name order. Switch-agent fallback excluded —
    /// the discovery endpoint enumerates agents separately.
    pub fn list_static(&self) -> Vec<Arc<dyn Command>> {
        let mut handlers: Vec<_> = self.static_handlers.values().cloned().collect();
        handlers.sort_by(|a, b| a.name().cmp(b.name()));
        handlers
    }
}
