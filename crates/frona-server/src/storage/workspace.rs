use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::core::error::AppError;

pub struct Workspace {
    layers: Vec<PathBuf>,
    shared_dir: Option<PathBuf>,
}

impl Workspace {
    pub fn new(layers: Vec<PathBuf>, shared_dir: Option<PathBuf>) -> Self {
        Self { layers, shared_dir }
    }

    pub fn read(&self, path: &str) -> Option<String> {
        for layer in &self.layers {
            let full = layer.join(path);
            if let Ok(content) = std::fs::read_to_string(&full) {
                return Some(content);
            }
        }

        if let Some(ref shared) = self.shared_dir {
            let shared_path = shared.join(path);
            return std::fs::read_to_string(&shared_path).ok();
        }

        None
    }

    pub fn write(&self, path: &str, content: &str) -> Result<(), AppError> {
        self.write_bytes(path, content.as_bytes())
    }

    pub fn write_bytes(&self, path: &str, content: &[u8]) -> Result<(), AppError> {
        let full = self.layers[0].join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::Internal(format!("Failed to create directories: {e}"))
            })?;
        }
        std::fs::write(&full, content).map_err(|e| {
            AppError::Internal(format!("Failed to write {}: {e}", full.display()))
        })
    }

    pub fn exists(&self, path: &str) -> bool {
        for layer in &self.layers {
            if layer.join(path).exists() {
                return true;
            }
        }

        self.shared_dir
            .as_ref()
            .is_some_and(|shared| shared.join(path).exists())
    }

    pub fn read_dir(&self, path: &str) -> Vec<String> {
        let mut seen = HashSet::new();

        for layer in &self.layers {
            let dir = layer.join(path);
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    seen.insert(name);
                }
            }
        }

        if let Some(ref shared) = self.shared_dir {
            let shared_path = shared.join(path);
            if let Ok(entries) = std::fs::read_dir(&shared_path) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    seen.insert(name);
                }
            }
        }

        let mut result: Vec<String> = seen.into_iter().collect();
        result.sort();
        result
    }

    pub fn resolve_path(&self, path: &str) -> Option<PathBuf> {
        for layer in &self.layers {
            let full = layer.join(path);
            if full.exists() {
                return Some(full);
            }
        }
        None
    }

    pub fn base_path(&self) -> &Path {
        &self.layers[0]
    }
}
