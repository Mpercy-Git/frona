//! MCP registry client backed by the `fronalabs/mcp-registry-database` release
//! artifact. Downloads the compressed dump, decompresses it once into the MCP
//! cache directory as `servers.json`, refreshes only when the remote
//! `content_sha256` changes, and serves queries from an in-memory cache of the
//! parsed `Vec<RegistryServerEntry>` (24-hour TTL, busted on refresh).

use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::RwLock;
use tokio::task;

use super::metadata::{self, PrebuiltMetadata, RegistryServerEntry};
use crate::core::error::AppError;

pub const PREBUILT_METADATA_URL: &str =
    "https://github.com/fronalabs/mcp-registry-database/releases/latest/download/metadata.json";
pub const PREBUILT_SERVERS_URL: &str =
    "https://github.com/fronalabs/mcp-registry-database/releases/latest/download/servers.json.xz";

const CACHE_TTL: Duration = Duration::from_secs(86400);

#[async_trait]
pub trait McpRegistryClient: Send + Sync {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<RegistryServerEntry>, AppError>;

    /// `name` is the reverse-DNS id like `io.github.foo/bar`.
    async fn fetch(&self, name: &str) -> Result<RegistryServerEntry, AppError>;

    /// The prebuilt dump is latest-only by design. Returns an error for any
    /// `version` that isn't the latest one currently in the dump.
    async fn fetch_version(
        &self,
        name: &str,
        version: &str,
    ) -> Result<RegistryServerEntry, AppError>;
}

struct CachedDump {
    entries: Arc<Vec<RegistryServerEntry>>,
    loaded_at: Instant,
}

pub struct PrebuiltMcpRegistryClient {
    metadata_url: String,
    servers_url: String,
    cache_dir: PathBuf,
    http: reqwest::Client,
    cache: RwLock<Option<CachedDump>>,
}

impl PrebuiltMcpRegistryClient {
    pub fn new(http: reqwest::Client, cache_dir: PathBuf) -> Self {
        Self {
            metadata_url: PREBUILT_METADATA_URL.to_string(),
            servers_url: PREBUILT_SERVERS_URL.to_string(),
            cache_dir,
            http,
            cache: RwLock::new(None),
        }
    }

    #[cfg(test)]
    fn with_urls(cache_dir: PathBuf, metadata_url: String, servers_url: String) -> Self {
        Self {
            metadata_url,
            servers_url,
            cache_dir,
            http: crate::build_http_client(),
            cache: RwLock::new(None),
        }
    }

    fn metadata_path(&self) -> PathBuf {
        self.cache_dir.join("metadata.json")
    }

    fn servers_path(&self) -> PathBuf {
        self.cache_dir.join("servers.json")
    }

    /// Downloads + decompresses the dump if the remote `content_sha256` differs
    /// from the local one. No-op when they match. Must be called before any
    /// query method. Busts the in-memory cache when a new dump is written.
    pub async fn ensure_fresh(&self) -> Result<(), AppError> {
        fs::create_dir_all(&self.cache_dir).map_err(|e| {
            AppError::Tool(format!("creating MCP registry cache dir failed: {e}"))
        })?;

        let servers_path = self.servers_path();
        if let Ok(meta) = fs::metadata(&servers_path)
            && let Ok(modified) = meta.modified()
            && modified.elapsed().unwrap_or_default() < CACHE_TTL
        {
            return Ok(());
        }

        let remote_meta_bytes = match self.http.get(&self.metadata_url).send().await {
            Ok(resp) => match resp.error_for_status() {
                Ok(resp) => resp.bytes().await.ok(),
                Err(e) => {
                    tracing::warn!("MCP registry metadata HTTP error: {e}");
                    None
                }
            },
            Err(e) => {
                tracing::warn!("MCP registry metadata fetch failed: {e}");
                None
            }
        };

        let Some(remote_meta_bytes) = remote_meta_bytes else {
            if servers_path.is_file() {
                return Ok(());
            }
            return Err(AppError::Tool(
                "MCP registry unreachable and no local cache available".into(),
            ));
        };

        let remote_meta: PrebuiltMetadata = serde_json::from_slice(&remote_meta_bytes)
            .map_err(|e| AppError::Tool(format!("parsing MCP registry metadata failed: {e}")))?;

        let local_meta = self.read_local_metadata();
        if let Some(local) = &local_meta
            && local.content_sha256 == remote_meta.content_sha256
            && servers_path.is_file()
        {
            fs::File::open(&servers_path).and_then(|f| f.set_modified(std::time::SystemTime::now())).ok();
            return Ok(());
        }

        let compressed = self
            .http
            .get(&self.servers_url)
            .send()
            .await
            .map_err(|e| AppError::Tool(format!("fetching MCP registry dump failed: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::Tool(format!("MCP registry dump HTTP error: {e}")))?
            .bytes()
            .await
            .map_err(|e| AppError::Tool(format!("reading MCP registry dump body failed: {e}")))?
            .to_vec();

        let json_bytes = task::spawn_blocking(move || -> Result<Vec<u8>, AppError> {
            let mut decoder = xz2::read::XzDecoder::new(compressed.as_slice());
            let mut out = Vec::new();
            decoder.read_to_end(&mut out).map_err(|e| {
                AppError::Tool(format!("decompressing MCP registry dump failed: {e}"))
            })?;
            Ok(out)
        })
        .await
        .map_err(|e| AppError::Tool(format!("MCP registry decompress task failed: {e}")))??;

        atomic_write(&servers_path, &json_bytes)?;
        atomic_write(&self.metadata_path(), &remote_meta_bytes)?;
        self.invalidate_cache().await;
        Ok(())
    }

    fn read_local_metadata(&self) -> Option<PrebuiltMetadata> {
        let bytes = fs::read(self.metadata_path()).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Returns the parsed dump, loading + caching it on first use. Subsequent
    /// calls within the TTL hand back a cheap `Arc` clone.
    async fn entries(&self) -> Result<Arc<Vec<RegistryServerEntry>>, AppError> {
        {
            let cache = self.cache.read().await;
            if let Some(c) = cache.as_ref()
                && c.loaded_at.elapsed() < CACHE_TTL
            {
                return Ok(c.entries.clone());
            }
        }

        let path = self.servers_path();
        let entries = task::spawn_blocking(move || metadata::load_dump(&path))
            .await
            .map_err(|e| AppError::Tool(format!("MCP registry load task failed: {e}")))??;
        let entries = Arc::new(entries);

        let mut cache = self.cache.write().await;
        *cache = Some(CachedDump {
            entries: entries.clone(),
            loaded_at: Instant::now(),
        });
        Ok(entries)
    }

    async fn invalidate_cache(&self) {
        *self.cache.write().await = None;
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| AppError::Tool(format!("creating {} failed: {e}", parent.display())))?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|s| s.to_str()).unwrap_or("")
    ));
    {
        let file = File::create(&tmp)
            .map_err(|e| AppError::Tool(format!("creating {} failed: {e}", tmp.display())))?;
        let mut w = BufWriter::new(file);
        w.write_all(bytes)
            .map_err(|e| AppError::Tool(format!("writing {} failed: {e}", tmp.display())))?;
        w.flush()
            .map_err(|e| AppError::Tool(format!("flushing {} failed: {e}", tmp.display())))?;
    }
    fs::rename(&tmp, path)
        .map_err(|e| AppError::Tool(format!("renaming into {} failed: {e}", path.display())))?;
    Ok(())
}

#[async_trait]
impl McpRegistryClient for PrebuiltMcpRegistryClient {
    async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RegistryServerEntry>, AppError> {
        self.ensure_fresh().await?;
        let entries = self.entries().await?;
        Ok(metadata::search_entries(&entries, query, limit))
    }

    async fn fetch(&self, name: &str) -> Result<RegistryServerEntry, AppError> {
        self.ensure_fresh().await?;
        let entries = self.entries().await?;
        metadata::fetch_entry(&entries, name)
            .cloned()
            .ok_or_else(|| AppError::Tool(format!("MCP registry has no server named {name}")))
    }

    async fn fetch_version(
        &self,
        name: &str,
        version: &str,
    ) -> Result<RegistryServerEntry, AppError> {
        let entry = self.fetch(name).await?;
        if entry.version == version {
            Ok(entry)
        } else {
            Err(AppError::Tool(format!(
                "MCP registry dump only has {name}@{latest}, not {version}",
                latest = entry.version
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn atomic_write_overwrites_existing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("foo.json");
        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
    }

    #[tokio::test]
    async fn entries_caches_parsed_dump_across_calls() {
        let dir = tempdir().unwrap();
        let client = PrebuiltMcpRegistryClient::with_urls(
            dir.path().to_path_buf(),
            "http://invalid.invalid/metadata.json".into(),
            "http://invalid.invalid/servers.json.xz".into(),
        );

        let payload = serde_json::json!([
            {
                "name": "a/one",
                "description": "first",
                "version": "1.0.0",
                "packages": [{
                    "registry_type": "npm",
                    "identifier": "a/one",
                    "transport": { "type": "stdio" }
                }]
            }
        ]);
        fs::write(
            client.servers_path(),
            serde_json::to_vec(&payload).unwrap(),
        )
        .unwrap();

        let first = client.entries().await.unwrap();
        let second = client.entries().await.unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "second call must return the cached Arc, not reparse the file",
        );
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].name, "a/one");

        client.invalidate_cache().await;
        let third = client.entries().await.unwrap();
        assert!(
            !Arc::ptr_eq(&first, &third),
            "after invalidation the next call must reparse",
        );
    }
}
