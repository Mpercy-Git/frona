use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::core::error::AppError;
use crate::core::supervisor::Supervisor;
use crate::notification::models::NotificationData;

use super::manager::McpManager;
use super::models::McpServerStatus;
use super::service::McpServerService;

pub struct McpSupervisor {
    service: Arc<McpServerService>,
    manager: Arc<McpManager>,
}

impl McpSupervisor {
    pub fn new(service: Arc<McpServerService>, manager: Arc<McpManager>) -> Self {
        Self { service, manager }
    }
}

#[async_trait]
impl Supervisor for McpSupervisor {
    fn label(&self) -> &'static str {
        "mcp"
    }

    async fn find_running(&self) -> Result<Vec<String>, AppError> {
        let servers = self.service.find_running().await?;
        Ok(servers.into_iter().map(|s| s.id).collect())
    }

    async fn start(&self, id: &str) -> Result<(), AppError> {
        let server = self.service.find_by_id(id).await?;
        self.service.start(&server.user_id, id).await?;
        Ok(())
    }

    async fn stop(&self, id: &str) -> Result<(), AppError> {
        self.manager.stop(id).await
    }

    async fn find_dead(&self) -> Result<Vec<String>, AppError> {
        Ok(self.manager.health_check().await)
    }

    async fn restart_count(&self, id: &str) -> u32 {
        self.manager.restart_count(id).await
    }

    async fn mark_failed(&self, id: &str, _reason: &str) -> Result<(), AppError> {
        self.service.mark_status(id, McpServerStatus::Failed).await
    }

    async fn record_access(&self, _id: &str) {}

    async fn find_idle(&self, _idle_threshold: Duration) -> Result<Vec<String>, AppError> {
        Ok(vec![])
    }

    async fn mark_hibernated(&self, id: &str) -> Result<(), AppError> {
        self.manager.stop(id).await?;
        self.service.mark_status(id, McpServerStatus::Stopped).await
    }

    async fn owner_of(&self, id: &str) -> Result<String, AppError> {
        let server = self.service.find_by_id(id).await?;
        Ok(server.user_id)
    }

    async fn display_name(&self, id: &str) -> String {
        self.service
            .find_by_id(id)
            .await
            .map(|s| s.display_name)
            .unwrap_or_else(|_| id.to_string())
    }

    async fn notification_data(&self, id: &str, action: &str) -> NotificationData {
        // No dedicated `Mcp` notification variant — reuse `App`.
        let handle = self
            .service
            .find_by_id(id)
            .await
            .ok()
            .map(|s| s.handle.to_string())
            .unwrap_or_else(|| id.to_string());
        NotificationData::App {
            app_handle: handle,
            action: action.to_string(),
        }
    }
}
