use std::sync::Arc;

use super::{Command, CommandRegistry};

pub mod clear;
pub mod compact;
pub mod switch_agent;
pub mod title;

pub use clear::ClearCommand;
pub use compact::CompactCommand;
pub use switch_agent::SwitchAgentCommand;
pub use title::TitleCommand;

pub fn register_all(registry: &mut CommandRegistry) {
    let handlers: Vec<Arc<dyn Command>> = vec![
        Arc::new(ClearCommand),
        Arc::new(CompactCommand),
        Arc::new(TitleCommand),
    ];
    for h in handlers {
        registry.register(h);
    }
    registry.with_switch_agent_fallback(Arc::new(SwitchAgentCommand));
}
