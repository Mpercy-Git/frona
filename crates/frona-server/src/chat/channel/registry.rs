use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use super::models::{ChannelFactory, ChannelManifest};

#[derive(Default)]
pub struct ChannelRegistry {
    factories: RwLock<HashMap<String, Arc<dyn ChannelFactory>>>,
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_factory(&self, factory: Arc<dyn ChannelFactory>) {
        let id = factory.manifest().id.clone();
        let mut factories = self.factories.write().expect("registry poisoned");
        factories.insert(id, factory);
    }

    pub fn get_factory(&self, id: &str) -> Option<Arc<dyn ChannelFactory>> {
        let factories = self.factories.read().expect("registry poisoned");
        factories.get(id).cloned()
    }

    pub fn list_manifests(&self) -> Vec<ChannelManifest> {
        let factories = self.factories.read().expect("registry poisoned");
        let mut out: Vec<ChannelManifest> =
            factories.values().map(|f| f.manifest()).collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn get_manifest(&self, id: &str) -> Option<ChannelManifest> {
        let factories = self.factories.read().expect("registry poisoned");
        factories.get(id).map(|f| f.manifest())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::channel::models::ChannelAdapter;
    use crate::core::error::AppError;

    struct StubFactory;
    impl ChannelFactory for StubFactory {
        fn manifest(&self) -> ChannelManifest {
            ChannelManifest {
                id: "stub".into(),
                display_name: "Stub".into(),
                description: "test".into(),
                config_fields: vec![],
                webhook_url_visible: false,
                setup_instructions: None,
                external_links: vec![],
            }
        }
        fn create(&self, _config: serde_json::Value) -> Result<Box<dyn ChannelAdapter>, AppError> {
            unimplemented!("not used in registry tests")
        }
    }

    #[test]
    fn register_and_list_factory() {
        let r = ChannelRegistry::new();
        r.register_factory(Arc::new(StubFactory));
        let manifests = r.list_manifests();
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].id, "stub");
        assert!(r.get_factory("stub").is_some());
    }

    #[test]
    fn re_register_replaces() {
        let r = ChannelRegistry::new();
        r.register_factory(Arc::new(StubFactory));
        r.register_factory(Arc::new(StubFactory));
        assert_eq!(r.list_manifests().len(), 1);
    }
}
