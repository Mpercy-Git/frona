pub mod driver;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::core::error::AppError;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use self::driver::{SandboxConfig, SandboxOutput, create_driver, execute_sandboxed};
use self::driver::resource_monitor::SystemResourceManager;

pub struct SandboxManager {
    driver: Arc<dyn driver::SandboxDriver>,
    shared_read_paths: Vec<String>,
    resource_manager: Arc<SystemResourceManager>,
    default_timeout_secs: u64,
}

impl SandboxManager {
    pub fn new(
        sandbox_disabled: bool,
        resource_manager: Arc<SystemResourceManager>,
    ) -> Self {
        Self {
            driver: Arc::from(create_driver(sandbox_disabled)),
            shared_read_paths: Vec::new(),
            resource_manager,
            default_timeout_secs: 0,
        }
    }

    pub fn driver_id(&self) -> &'static str {
        self.driver.driver_id()
    }

    pub fn with_default_timeout(mut self, secs: u64) -> Self {
        self.default_timeout_secs = secs;
        self
    }

    pub fn default_timeout_secs(&self) -> u64 {
        self.default_timeout_secs
    }

    pub fn resource_manager(&self) -> &Arc<SystemResourceManager> {
        &self.resource_manager
    }

    pub fn with_shared_read_paths(mut self, paths: Vec<String>) -> Self {
        self.shared_read_paths = paths;
        self
    }

    pub fn get_sandbox(
        &self,
        workspace: PathBuf,
        agent_id: &str,
        network_access: bool,
        allowed_network_destinations: Vec<String>,
    ) -> Sandbox {
        Sandbox {
            path: workspace,
            driver: Arc::clone(&self.driver),
            network_access,
            allowed_network_destinations,
            allowed_bind_ports: Vec::new(),
            extra_env_vars: Vec::new(),
            shared_read_paths: self.shared_read_paths.clone(),
            shared_read_files: Vec::new(),
            shared_write_paths: Vec::new(),
            denied_paths: Vec::new(),
            blocked_networks: Vec::new(),
            agent_id: agent_id.to_string(),
            resource_manager: Arc::clone(&self.resource_manager),
            init_venv: true,
            init_node: true,
        }
    }
}

pub struct Sandbox {
    path: PathBuf,
    driver: Arc<dyn driver::SandboxDriver>,
    network_access: bool,
    allowed_network_destinations: Vec<String>,
    allowed_bind_ports: Vec<u16>,
    extra_env_vars: Vec<(String, String)>,
    shared_read_paths: Vec<String>,
    shared_read_files: Vec<String>,
    shared_write_paths: Vec<String>,
    denied_paths: Vec<String>,
    blocked_networks: Vec<String>,
    agent_id: String,
    resource_manager: Arc<SystemResourceManager>,
    init_venv: bool,
    init_node: bool,
}

impl Sandbox {
    pub fn with_extra_env_vars(mut self, vars: Vec<(String, String)>) -> Self {
        self.extra_env_vars = vars;
        self
    }

    pub fn with_read_paths(mut self, paths: Vec<String>) -> Self {
        self.shared_read_paths.extend(paths);
        self
    }

    pub fn with_read_files(mut self, files: Vec<String>) -> Self {
        self.shared_read_files.extend(files);
        self
    }

    pub fn with_write_paths(mut self, paths: Vec<String>) -> Self {
        self.shared_write_paths.extend(paths);
        self
    }

    pub fn with_bind_ports(mut self, ports: Vec<u16>) -> Self {
        self.allowed_bind_ports = ports;
        self
    }

    pub fn with_denied_paths(mut self, paths: Vec<String>) -> Self {
        self.denied_paths.extend(paths);
        self
    }

    pub fn with_blocked_networks(mut self, networks: Vec<String>) -> Self {
        self.blocked_networks.extend(networks);
        self
    }

    pub fn without_venv(mut self) -> Self {
        self.init_venv = false;
        self
    }

    pub fn without_node(mut self) -> Self {
        self.init_node = false;
        self
    }
}

impl Sandbox {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn venv_path(&self) -> PathBuf {
        self.path.join(".venv")
    }

    pub fn setup(&self) -> Result<(), AppError> {
        self.ensure_dir()?;
        if self.init_venv {
            self.setup_venv();
        }
        if self.init_node {
            self.setup_node_env();
        }
        Ok(())
    }

    pub fn ensure_dir(&self) -> Result<(), AppError> {
        if !self.path.exists() {
            std::fs::create_dir_all(&self.path).map_err(|e| {
                AppError::Tool(format!(
                    "Failed to create sandbox dir {}: {e}",
                    self.path.display()
                ))
            })?;
        }
        Ok(())
    }

    fn setup_venv(&self) {
        let venv = self.venv_path();
        if venv.exists() {
            return;
        }

        let result = std::process::Command::new("python3")
            .args(["-m", "venv", "--system-site-packages", ".venv"])
            .current_dir(&self.path)
            .output();

        match result {
            Ok(output) if output.status.success() => {
                tracing::info!(path = %venv.display(), "Created Python venv");
            }
            Ok(output) => {
                tracing::warn!(
                    stderr = String::from_utf8_lossy(&output.stderr).as_ref(),
                    "Failed to create Python venv"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "python3 not available, skipping venv creation");
            }
        }
    }

    fn setup_node_env(&self) {
        let node_prefix = self.path.join(".node");
        if node_prefix.exists() {
            return;
        }

        if let Err(e) = std::fs::create_dir_all(&node_prefix) {
            tracing::warn!(error = %e, "Failed to create .node prefix directory");
            return;
        }

        if !self.path.join("package.json").exists() {
            let result = std::process::Command::new("npm")
                .args(["init", "-y"])
                .current_dir(&self.path)
                .output();

            match result {
                Ok(output) if output.status.success() => {
                    tracing::info!(path = %self.path.display(), "Initialized npm workspace");
                }
                Ok(output) => {
                    tracing::warn!(
                        stderr = String::from_utf8_lossy(&output.stderr).as_ref(),
                        "Failed to run npm init"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "npm not available, skipping Node.js env setup");
                }
            }
        }
    }

    fn base_config(&self) -> Result<SandboxConfig, AppError> {
        self.setup()?;

        let canonical_path = std::fs::canonicalize(&self.path).unwrap_or_else(|_| self.path.clone());

        let mut additional_path_dirs = Vec::new();
        let mut env_vars = Vec::new();

        let venv_bin = canonical_path.join(".venv").join("bin");
        if venv_bin.exists() {
            additional_path_dirs.push(venv_bin.to_string_lossy().into_owned());
            env_vars.push((
                "VIRTUAL_ENV".to_string(),
                canonical_path.join(".venv").to_string_lossy().into_owned(),
            ));
        }

        let (node_path_dirs, node_env) = node_env_vars(&canonical_path);
        additional_path_dirs.extend(node_path_dirs);
        env_vars.extend(node_env);

        env_vars.push(("HOME".to_string(), canonical_path.to_string_lossy().into_owned()));
        env_vars.push(("XDG_CONFIG_HOME".to_string(), canonical_path.join(".config").to_string_lossy().into_owned()));
        env_vars.push(("XDG_CACHE_HOME".to_string(), canonical_path.join(".cache").to_string_lossy().into_owned()));

        env_vars.extend(self.extra_env_vars.clone());

        let config = SandboxConfig {
            workspace_dir: canonical_path.to_string_lossy().into_owned(),
            network_access: self.network_access,
            allowed_network_destinations: self.allowed_network_destinations.clone(),
            allowed_bind_ports: self.allowed_bind_ports.clone(),
            additional_read_paths: self.shared_read_paths.clone(),
            additional_read_files: self.shared_read_files.clone(),
            additional_write_paths: self.shared_write_paths.clone(),
            denied_paths: self.denied_paths.clone(),
            blocked_networks: self.blocked_networks.clone(),
            additional_path_dirs,
            env_vars,
            ..Default::default()
        };

        Ok(config)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        &self,
        program: &str,
        args: &[&str],
        working_dir: Option<&str>,
        extra_path_dirs: Vec<String>,
        stdin: Option<std::process::Stdio>,
        stdout: std::process::Stdio,
        stderr: std::process::Stdio,
    ) -> Result<tokio::process::Child, AppError> {
        let mut config = self.base_config()?;

        config.additional_path_dirs.extend(extra_path_dirs);

        if let Some(wd) = working_dir {
            let canonical_wd = std::fs::canonicalize(wd)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| wd.to_string());
            config.working_dir = Some(canonical_wd);
        }

        let mut cmd = self.driver.sandboxed_command(program, args, &config)?;

        cmd.env_clear();

        const PASSTHROUGH_VARS: &[&str] =
            &["TERM", "LANG", "LC_ALL", "LC_CTYPE", "TZ", "USER", "LOGNAME", "TMPDIR", "SHELL"];

        for key in PASSTHROUGH_VARS {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }

        {
            let existing = std::env::var("PATH").unwrap_or_default();
            if config.additional_path_dirs.is_empty() {
                cmd.env("PATH", existing);
            } else {
                let extra = config.additional_path_dirs.join(":");
                cmd.env("PATH", format!("{extra}:{existing}"));
            }
        }

        for (key, value) in &config.env_vars {
            cmd.env(key, value);
        }

        if let Some(stdin) = stdin {
            cmd.stdin(stdin);
        }
        cmd.stdout(stdout);
        cmd.stderr(stderr);

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }

        let child = tokio::process::Command::from(cmd)
            .spawn()
            .map_err(|e| AppError::Tool(format!("Failed to spawn process: {e}")))?;

        if let Some(pid) = child.id() {
            self.resource_manager.register(pid, &self.agent_id);
        }

        Ok(child)
    }

    pub async fn execute(
        &self,
        program: &str,
        args: &[&str],
        timeout_secs: u64,
        on_stdout: Option<mpsc::Sender<String>>,
        stdin_rx: Option<mpsc::Receiver<String>>,
        cancel_token: Option<CancellationToken>,
    ) -> Result<SandboxOutput, AppError> {
        let mut config = self.base_config()?;
        config.timeout_secs = timeout_secs;

        execute_sandboxed(
            &*self.driver,
            program,
            args,
            &config,
            on_stdout,
            stdin_rx,
            cancel_token,
            Some(&self.resource_manager),
            Some(&self.agent_id),
        )
        .await
    }
}

pub fn node_env_vars(workspace: &Path) -> (Vec<String>, Vec<(String, String)>) {
    let node_prefix = workspace.join(".node");
    if !node_prefix.exists() {
        return (Vec::new(), Vec::new());
    }
    let mut path_dirs = Vec::new();
    let bin = node_prefix.join("bin");
    if bin.exists() {
        path_dirs.push(bin.to_string_lossy().into_owned());
    }
    let env = vec![
        ("NPM_CONFIG_PREFIX".into(), node_prefix.to_string_lossy().into_owned()),
        ("NPM_CONFIG_CACHE".into(), workspace.join(".npm-cache").to_string_lossy().into_owned()),
        ("NODE_PATH".into(), workspace.join("node_modules").to_string_lossy().into_owned()),
    ];
    (path_dirs, env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn python3_available() -> bool {
        std::process::Command::new("python3")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn temp_sandbox(name: &str) -> Sandbox {
        let dir = std::env::temp_dir().join("frona_test_workspace").join(name);
        Sandbox {
            path: dir,
            driver: Arc::from(create_driver(false)),
            network_access: false,
            allowed_network_destinations: Vec::new(),
            allowed_bind_ports: Vec::new(),
            extra_env_vars: Vec::new(),
            shared_read_paths: Vec::new(),
            shared_read_files: Vec::new(),
            shared_write_paths: Vec::new(),
            denied_paths: Vec::new(),
            blocked_networks: Vec::new(),
            agent_id: "test".to_string(),
            resource_manager: Arc::new(SystemResourceManager::new(80.0, 80.0, 90.0, 90.0)),
            init_venv: true,
            init_node: true,
        }
    }

    #[test]
    fn test_setup_creates_venv() {
        if !python3_available() {
            eprintln!("python3 not found, skipping");
            return;
        }
        let ws = temp_sandbox(&format!("setup_venv_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::remove_dir_all(&ws.path);

        ws.setup().unwrap();

        assert!(ws.path.exists());
        assert!(ws.venv_path().exists());
        assert!(ws.venv_path().join("bin").join("python3").exists());

        let _ = std::fs::remove_dir_all(&ws.path);
    }

    #[test]
    fn test_setup_venv_with_relative_path() {
        if !python3_available() {
            eprintln!("python3 not found, skipping");
            return;
        }
        let name = format!("relative_venv_{}", uuid::Uuid::new_v4());
        let rel_base = PathBuf::from("target").join("test_workspaces");
        let rel_path = rel_base.join(&name);
        let _ = std::fs::remove_dir_all(&rel_path);

        let ws = Sandbox {
            path: rel_path.clone(),
            driver: Arc::from(create_driver(false)),
            network_access: false,
            allowed_network_destinations: Vec::new(),
            allowed_bind_ports: Vec::new(),
            extra_env_vars: Vec::new(),
            shared_read_paths: Vec::new(),
            shared_read_files: Vec::new(),
            shared_write_paths: Vec::new(),
            denied_paths: Vec::new(),
            blocked_networks: Vec::new(),
            agent_id: "test".to_string(),
            resource_manager: Arc::new(SystemResourceManager::new(80.0, 80.0, 90.0, 90.0)),
            init_venv: true,
            init_node: true,
        };
        ws.setup().unwrap();

        assert!(rel_path.join(".venv").exists(), "venv should exist at the workspace root");
        assert!(
            rel_path.join(".venv").join("bin").join("python3").exists(),
            "venv should contain bin/python3"
        );
        assert!(
            !rel_path.join("target").exists(),
            "no nested dirs should be created inside workspace"
        );

        let _ = std::fs::remove_dir_all(&rel_path);
    }

    #[tokio::test]
    async fn test_execute_sets_home_and_xdg_env_vars() {
        let ws = temp_sandbox(&format!("home_env_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::remove_dir_all(&ws.path);

        let result = ws
            .execute(
                "bash",
                &["-c", "echo $HOME; echo $XDG_CONFIG_HOME; echo $XDG_CACHE_HOME"],
                10,
                None,
                None,
                None,
            )
            .await
            .unwrap();

        let canonical = std::fs::canonicalize(&ws.path).unwrap();
        let lines: Vec<&str> = result.stdout.trim().lines().collect();
        assert_eq!(lines.len(), 3, "expected 3 lines, got: {:?}", lines);
        assert_eq!(lines[0], canonical.to_string_lossy());
        assert_eq!(lines[1], canonical.join(".config").to_string_lossy().as_ref());
        assert_eq!(lines[2], canonical.join(".cache").to_string_lossy().as_ref());

        let _ = std::fs::remove_dir_all(&ws.path);
    }

    #[tokio::test]
    async fn test_execute_does_not_leak_parent_env() {
        unsafe { std::env::set_var("FRONA_TEST_SECRET", "leaked") };
        let ws = temp_sandbox(&format!("env_leak_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::remove_dir_all(&ws.path);

        let result = ws
            .execute("bash", &["-c", "echo $FRONA_TEST_SECRET"], 10, None, None, None)
            .await
            .unwrap();

        assert_eq!(result.stdout.trim(), "", "parent env vars must not leak into sandbox");

        unsafe { std::env::remove_var("FRONA_TEST_SECRET") };
        let _ = std::fs::remove_dir_all(&ws.path);
    }

    #[test]
    fn test_setup_idempotent() {
        if !python3_available() {
            eprintln!("python3 not found, skipping");
            return;
        }
        let ws = temp_sandbox(&format!("setup_idempotent_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::remove_dir_all(&ws.path);

        ws.setup().unwrap();
        ws.setup().unwrap();

        assert!(ws.venv_path().exists());

        let _ = std::fs::remove_dir_all(&ws.path);
    }

    #[tokio::test]
    async fn test_execute_async_streaming() {
        let ws = temp_sandbox(&format!("streaming_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::remove_dir_all(&ws.path);
        let (tx, mut rx) = mpsc::channel(16);

        let result = ws
            .execute(
                "bash",
                &["-c", "echo hello; echo world"],
                10,
                Some(tx),
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("hello"));
        assert!(result.stdout.contains("world"));

        let mut lines = Vec::new();
        while let Ok(line) = rx.try_recv() {
            lines.push(line);
        }
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "hello");
        assert_eq!(lines[1], "world");

        let _ = std::fs::remove_dir_all(&ws.path);
    }
}
