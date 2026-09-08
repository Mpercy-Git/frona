pub mod basic;
pub mod pkm;
pub mod service;

/// Memory tools withheld from an agent with
/// [`crate::agent::models::Agent::private_memory`] set: they write where the
/// user's *other* agents can read, and there is no private equivalent to
/// redirect them to. Withholding happens in `ToolManager::build_agent_registry`.
///
/// Only `store_user_memory` qualifies. Basic memory's other write,
/// `store_agent_memory`, is already agent-scoped, and PKM's `memory_remember`
/// keeps working for a private agent - it writes an agent-scoped short memory
/// instead of a user-scoped one, so the agent still remembers, just to itself.
pub const PRIVATE_MEMORY_WITHHELD_TOOLS: &[&str] = &["store_user_memory"];
