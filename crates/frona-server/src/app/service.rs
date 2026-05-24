use std::sync::Arc;

use crate::core::config::AppConfig;
use crate::core::error::AppError;

use super::manager::{AppManager, ProcessStatus};
use super::models::{App, AppManifest, AppResponse, AppStatus, HealthCheck};
use super::repository::AppRepository;

#[derive(Clone)]
pub struct AppService {
    repo: Arc<dyn AppRepository>,
    manager: Arc<AppManager>,
    config: AppConfig,
    policy_service: crate::policy::service::PolicyService,
    user_service: crate::auth::UserService,
}

impl AppService {
    pub fn new(
        repo: impl AppRepository + 'static,
        manager: Arc<AppManager>,
        config: AppConfig,
        policy_service: crate::policy::service::PolicyService,
        user_service: crate::auth::UserService,
    ) -> Self {
        Self {
            repo: Arc::new(repo),
            manager,
            config,
            policy_service,
            user_service,
        }
    }

    pub fn manager(&self) -> &Arc<AppManager> {
        &self.manager
    }

    pub async fn deploy(
        &self,
        agent_id: &str,
        user_id: &str,
        chat_id: &str,
        manifest: &AppManifest,
        credential_env_vars: Vec<(String, String)>,
    ) -> Result<AppResponse, AppError> {
        let existing = self.repo.find_by_user_handle(user_id, &manifest.handle).await?;

        if let Some(ref e) = existing
            && e.agent_id != agent_id
        {
            return Err(AppError::Forbidden(format!(
                "App handle '{}' is owned by a different agent",
                manifest.handle.as_str()
            )));
        }

        let app_id = if let Some(ref existing) = existing {
            existing.id.clone()
        } else {
            crate::core::repository::new_id()
        };

        let kind = manifest.effective_kind().to_string();
        let now = chrono::Utc::now();

        let manifest_json = serde_json::to_value(manifest)
            .map_err(|e| AppError::Validation(format!("Invalid manifest: {e}")))?;

        match kind.as_str() {
            "static" => {
                if let Some(ref ex) = existing
                    && matches!(ex.status, AppStatus::Running | AppStatus::Starting)
                {
                    let _ = self.manager.stop_app(&ex.id).await;
                }

                let static_dir = manifest
                    .static_dir
                    .as_ref()
                    .ok_or_else(|| {
                        AppError::Validation("static_dir required for static apps".into())
                    })?
                    .clone();

                let app = App {
                    id: app_id,
                    agent_id: agent_id.to_string(),
                    user_id: user_id.to_string(),
                    handle: manifest.handle.clone(),
                    name: manifest.name.clone(),
                    description: manifest.description.clone(),
                    kind,
                    command: None,
                    static_dir: Some(static_dir),
                    port: None,
                    status: AppStatus::Serving,
                    pid: None,
                    manifest: manifest_json,
                    chat_id: chat_id.to_string(),
                    crash_fix_attempts: 0,
                    last_accessed_at: None,
                    created_at: existing.as_ref().map(|e| e.created_at).unwrap_or(now),
                    updated_at: now,
                };

                let app = if existing.is_some() {
                    self.repo.update(&app).await?
                } else {
                    self.repo.create(&app).await?
                };
                Ok(app.into())
            }
            _ => {
                let command = manifest
                    .command
                    .as_ref()
                    .ok_or_else(|| {
                        AppError::Validation("command required for service apps".into())
                    })?
                    .clone();

                if let Some(ref ex) = existing
                    && matches!(ex.status, AppStatus::Running | AppStatus::Starting)
                {
                    self.manager.stop_app(&ex.id).await?;
                }

                let mut app = App {
                    id: app_id,
                    agent_id: agent_id.to_string(),
                    user_id: user_id.to_string(),
                    handle: manifest.handle.clone(),
                    name: manifest.name.clone(),
                    description: manifest.description.clone(),
                    kind: "service".to_string(),
                    command: Some(command.clone()),
                    static_dir: None,
                    port: None,
                    status: AppStatus::Starting,
                    pid: None,
                    manifest: manifest_json,
                    chat_id: chat_id.to_string(),
                    crash_fix_attempts: 0,
                    last_accessed_at: None,
                    created_at: existing.as_ref().map(|e| e.created_at).unwrap_or(now),
                    updated_at: now,
                };

                app = if existing.is_some() {
                    self.repo.update(&app).await?
                } else {
                    self.repo.create(&app).await?
                };

                // Reconcile before spawn — sandbox reads Cedar, not the manifest.
                let user_handle = self.user_service.handle_of(user_id).await?;
                self.policy_service
                    .reconcile_sandbox_policy(
                        user_id,
                        crate::policy::reconcile::EntityRef::App(format!(
                            "{user_handle}/{}",
                            app.handle
                        )),
                        manifest.sandbox_policy.as_ref().unwrap_or(&crate::policy::sandbox::SandboxPolicy::default()),
                    )
                    .await?;

                self.start_and_update(&mut app, &command, manifest, credential_env_vars)
                    .await
            }
        }
    }

    pub async fn stop(&self, agent_id: &str, app_id: &str, chat_id: &str) -> Result<AppResponse, AppError> {
        let mut app = self.get_owned_app(agent_id, app_id).await?;

        self.manager.stop_app(app_id).await?;

        app.status = AppStatus::Stopped;
        app.pid = None;
        app.port = None;
        app.chat_id = chat_id.to_string();
        app.crash_fix_attempts = 0;
        app.updated_at = chrono::Utc::now();
        let app = self.repo.update(&app).await?;
        Ok(app.into())
    }

    pub async fn start(
        &self,
        agent_id: &str,
        app_id: &str,
        chat_id: &str,
        credential_env_vars: Vec<(String, String)>,
    ) -> Result<AppResponse, AppError> {
        let mut app = self.get_owned_app(agent_id, app_id).await?;

        if matches!(app.status, AppStatus::Running | AppStatus::Starting) {
            return Ok(app.into());
        }

        app.chat_id = chat_id.to_string();
        app.crash_fix_attempts = 0;

        let command = app
            .command
            .as_ref()
            .ok_or_else(|| AppError::Validation("No command for this app".into()))?
            .clone();

        let manifest: AppManifest = serde_json::from_value(app.manifest.clone())
            .map_err(|e| AppError::Internal(format!("Invalid stored manifest: {e}")))?;

        self.start_and_update(&mut app, &command, &manifest, credential_env_vars)
            .await
    }

    pub async fn restart(
        &self,
        agent_id: &str,
        app_id: &str,
        chat_id: &str,
    ) -> Result<AppResponse, AppError> {
        let mut app = self.get_owned_app(agent_id, app_id).await?;

        app.chat_id = chat_id.to_string();
        app.crash_fix_attempts = 0;

        let command = app
            .command
            .as_ref()
            .ok_or_else(|| AppError::Validation("No command for this app".into()))?
            .clone();

        let manifest: AppManifest = serde_json::from_value(app.manifest.clone())
            .map_err(|e| AppError::Internal(format!("Invalid stored manifest: {e}")))?;

        if matches!(app.status, AppStatus::Running | AppStatus::Starting) {
            self.manager.stop_app(app_id).await?;
        }

        self.start_and_update(&mut app, &command, &manifest, Vec::new())
            .await
    }

    pub async fn destroy(&self, agent_id: &str, app_id: &str) -> Result<(), AppError> {
        let app = self.get_owned_app(agent_id, app_id).await?;

        if matches!(
            app.status,
            AppStatus::Running | AppStatus::Starting | AppStatus::Hibernated
        ) {
            self.manager.stop_app(app_id).await?;
        }

        let user_handle = self.user_service.handle_of(&app.user_id).await?;
        self.policy_service
            .reconcile_sandbox_policy(
                &app.user_id,
                crate::policy::reconcile::EntityRef::App(format!("{user_handle}/{}", app.handle)),
                &crate::policy::sandbox::SandboxPolicy::permissive(),
            )
            .await?;
        self.repo.delete(app_id).await
    }

    pub async fn list(&self, agent_id: &str) -> Result<Vec<AppResponse>, AppError> {
        let apps = self.repo.find_by_agent_id(agent_id).await?;
        Ok(apps.into_iter().map(Into::into).collect())
    }

    pub async fn list_by_user(&self, user_id: &str) -> Result<Vec<AppResponse>, AppError> {
        let apps = self.repo.find_by_user_id(user_id).await?;
        Ok(apps.into_iter().map(Into::into).collect())
    }

    pub async fn get(&self, app_id: &str) -> Result<Option<App>, AppError> {
        self.repo.find_by_id(app_id).await
    }

    pub async fn find_by_user_handle(
        &self,
        user_id: &str,
        handle: &crate::core::Handle,
    ) -> Result<Option<App>, AppError> {
        self.repo.find_by_user_handle(user_id, handle).await
    }

    pub async fn get_by_user(
        &self,
        user_id: &str,
        app_id: &str,
    ) -> Result<AppResponse, AppError> {
        let app = self
            .repo
            .find_by_id(app_id)
            .await?
            .ok_or_else(|| AppError::NotFound("App not found".into()))?;
        if app.user_id != user_id {
            return Err(AppError::Forbidden("Not your app".into()));
        }
        Ok(app.into())
    }

    pub async fn update_status(
        &self,
        app_id: &str,
        status: AppStatus,
        port: Option<u16>,
        pid: Option<u32>,
    ) -> Result<(), AppError> {
        if let Some(mut app) = self.repo.find_by_id(app_id).await? {
            app.status = status;
            app.port = port;
            app.pid = pid;
            app.updated_at = chrono::Utc::now();
            self.repo.update(&app).await?;
        }
        Ok(())
    }

    pub async fn update_crash_fix_attempts(
        &self,
        app_id: &str,
        attempts: u32,
    ) -> Result<(), AppError> {
        if let Some(mut app) = self.repo.find_by_id(app_id).await? {
            app.crash_fix_attempts = attempts;
            self.repo.update(&app).await?;
        }
        Ok(())
    }

    pub async fn update_last_accessed(
        &self,
        app_id: &str,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), AppError> {
        if let Some(mut app) = self.repo.find_by_id(app_id).await? {
            app.last_accessed_at = Some(at);
            self.repo.update(&app).await?;
        }
        Ok(())
    }

    pub async fn find_running(&self) -> Result<Vec<App>, AppError> {
        self.repo.find_running().await
    }

    pub fn max_restart_attempts(&self) -> u32 {
        self.config.max_restart_attempts
    }

    pub fn hibernate_after_secs(&self) -> u64 {
        self.config.hibernate_after_secs
    }

    async fn get_owned_app(&self, agent_id: &str, app_id: &str) -> Result<App, AppError> {
        let app = self
            .repo
            .find_by_id(app_id)
            .await?
            .ok_or_else(|| AppError::NotFound("App not found".into()))?;
        if app.agent_id != agent_id {
            return Err(AppError::Forbidden("App not owned by this agent".into()));
        }
        Ok(app)
    }

    pub async fn deploy_and_await(
        &self,
        agent_id: &str,
        user_id: &str,
        chat_id: &str,
        manifest: &AppManifest,
        credential_env_vars: Vec<(String, String)>,
    ) -> Result<AppResponse, AppError> {
        let app = self
            .deploy(agent_id, user_id, chat_id, manifest, credential_env_vars)
            .await?;

        if manifest.effective_kind() == "static" {
            return Ok(app);
        }

        let port = match app.port {
            Some(p) => p,
            None => return Ok(app),
        };

        let hc = manifest.health_check.clone().unwrap_or(HealthCheck {
            path: "/".to_string(),
            interval_secs: None,
            timeout_secs: None,
            initial_delay_secs: Some(2),
            failure_threshold: None,
        });

        let initial_delay = std::time::Duration::from_secs(hc.effective_initial_delay());
        let interval = std::time::Duration::from_secs(1);
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(self.config.health_check_timeout_secs);

        tokio::time::sleep(initial_delay).await;

        loop {
            if self.manager.health_check(port, &hc).await {
                return Ok(app);
            }

            if let ProcessStatus::Dead(_) = self.manager.check_process(&app.id).await {
                match self
                    .manager
                    .try_restart_crashed(&app.id, self.config.max_restart_attempts)
                    .await
                {
                    Ok(Some((new_port, pid))) => {
                        self.update_status(&app.id, AppStatus::Running, Some(new_port), Some(pid))
                            .await?;
                        tokio::time::sleep(initial_delay).await;
                        continue;
                    }
                    _ => {
                        self.update_status(&app.id, AppStatus::Failed, None, None)
                            .await?;
                        self.manager.remove_process(&app.id).await;
                        return Err(AppError::Tool(format!(
                            "App failed to start after all restart attempts. Check apps/{}/logs/app.log for details.",
                            app.handle
                        )));
                    }
                }
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::Tool(format!(
                    "App started but health check on {} did not pass within {}s. The app may still be starting — check apps/{}/logs/app.log",
                    hc.path, self.config.health_check_timeout_secs, app.handle
                )));
            }

            tokio::time::sleep(interval).await;
        }
    }

    async fn start_and_update(
        &self,
        app: &mut App,
        command: &str,
        manifest: &AppManifest,
        credential_env_vars: Vec<(String, String)>,
    ) -> Result<AppResponse, AppError> {
        match self
            .manager
            .start_app(&app.id, &app.agent_id, &app.user_id, command, manifest, credential_env_vars)
            .await
        {
            Ok((port, pid)) => {
                app.port = Some(port);
                app.pid = Some(pid);
                app.status = AppStatus::Running;
                app.updated_at = chrono::Utc::now();
                let app = self.repo.update(app).await?;
                Ok(app.into())
            }
            Err(e) => {
                app.status = AppStatus::Failed;
                app.updated_at = chrono::Utc::now();
                let _ = self.repo.update(app).await;
                Err(e)
            }
        }
    }
}
