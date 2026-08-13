use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::error::AppError;

const SKILLS_SH_BASE: &str = "https://skills.sh";
const GITHUB_RAW_BASE: &str = "https://raw.githubusercontent.com";

#[derive(Debug, Clone, Serialize)]
pub struct RemoteSkillSummary {
    pub name: String,
    pub repo: String,
    pub avatar_url: String,
    pub installs: u64,
}

#[derive(Debug, Clone)]
pub struct FetchedSkillFile {
    pub path: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct FetchedSkill {
    pub name: String,
    pub description: String,
    pub content: String,
    pub repo: String,
    pub sha: String,
    pub files: Vec<FetchedSkillFile>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredSkill {
    pub name: String,
    pub description: String,
    pub dir_path: String,
    pub sha: String,
}

#[derive(Debug, Deserialize)]
struct SkillsShResponse {
    skills: Vec<SkillsShEntry>,
}

#[derive(Debug, Deserialize)]
struct SkillsShEntry {
    name: String,
    source: String,
    installs: u64,
}

#[derive(Clone)]
pub struct SkillRegistryClient {
    client: reqwest::Client,
    cache_dir: PathBuf,
}

impl Default for SkillRegistryClient {
    fn default() -> Self {
        Self::new(crate::build_http_client(), "data/system/cache/skills")
    }
}

impl SkillRegistryClient {
    pub fn new(client: reqwest::Client, cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            client,
            cache_dir: cache_dir.into(),
        }
    }

    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<RemoteSkillSummary>, AppError> {
        let url = format!("{SKILLS_SH_BASE}/api/search");

        let resp = self.client.get(&url)
            .query(&[("q", query), ("limit", &limit.to_string())])
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("skills.sh search failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(AppError::Internal(format!("skills.sh returned status {}", resp.status())));
        }

        let data: SkillsShResponse = resp.json().await
            .map_err(|e| AppError::Internal(format!("Failed to parse skills.sh response: {e}")))?;

        Ok(data.skills.into_iter().map(|entry| {
            let avatar_url = derive_avatar_url(&entry.source);
            RemoteSkillSummary {
                name: entry.name,
                repo: entry.source,
                avatar_url,
                installs: entry.installs,
            }
        }).collect())
    }

    pub async fn discover_skills(&self, repo: &str) -> Result<Vec<DiscoveredSkill>, AppError> {
        let (repo_dir, head_sha) = self.ensure_repo(repo).await?;

        let mut skill_md_paths = Vec::new();
        walk_for_files(&repo_dir, &repo_dir, "SKILL.md", &mut skill_md_paths);

        let mut skills = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for relative_path in skill_md_paths {
            let dir_path = relative_path.parent()
                .filter(|p| *p != Path::new(""))
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();

            let name = if dir_path.is_empty() {
                repo.rsplit('/').next().unwrap_or(repo).to_string()
            } else {
                dir_path.rsplit('/').next().unwrap_or(&dir_path).to_string()
            };

            if !seen.insert(name.clone()) {
                continue;
            }

            let full_path = repo_dir.join(&relative_path);
            let description = std::fs::read_to_string(&full_path)
                .ok()
                .and_then(|content| agent_skills::Skill::parse(&sanitize_metadata_frontmatter(&content)).ok())
                .map(|parsed| parsed.description().as_str().to_string())
                .unwrap_or_default();

            skills.push(DiscoveredSkill {
                name,
                description,
                dir_path,
                sha: head_sha.clone(),
            });
        }

        Ok(skills)
    }

    pub async fn fetch_skill_from_cache(&self, repo: &str, discovered: &DiscoveredSkill) -> Result<FetchedSkill, AppError> {
        let (repo_dir, _) = self.ensure_repo(repo).await?;

        let skill_base = if discovered.dir_path.is_empty() {
            repo_dir.clone()
        } else {
            repo_dir.join(&discovered.dir_path)
        };

        let content = std::fs::read_to_string(skill_base.join("SKILL.md"))
            .map_err(|_| AppError::NotFound(format!("Skill '{}' not found in {repo}", discovered.name)))?;

        let parsed = agent_skills::Skill::parse(&sanitize_metadata_frontmatter(&content))
            .map_err(|e| AppError::Validation(format!("Invalid SKILL.md: {e}")))?;

        let files = read_skill_files(&skill_base);

        Ok(FetchedSkill {
            name: parsed.name().as_str().to_string(),
            description: parsed.description().as_str().to_string(),
            content,
            repo: repo.to_string(),
            sha: discovered.sha.clone(),
            files,
        })
    }

    pub async fn fetch_skill_content(&self, repo: &str, dir_path: &str) -> Result<String, AppError> {
        let url = if dir_path.is_empty() {
            format!("{GITHUB_RAW_BASE}/{repo}/main/SKILL.md")
        } else {
            format!("{GITHUB_RAW_BASE}/{repo}/main/{dir_path}/SKILL.md")
        };
        let resp = self.client.get(&url)
            .send()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to fetch SKILL.md: {e}")))?;

        if !resp.status().is_success() {
            return Err(AppError::NotFound(format!("SKILL.md not found at {dir_path} in {repo}")));
        }

        resp.text().await
            .map_err(|e| AppError::Internal(format!("Failed to read SKILL.md: {e}")))
    }

    async fn ensure_repo(&self, repo: &str) -> Result<(PathBuf, String), AppError> {
        let repo_dir = self.cache_dir.join(repo);

        if repo_dir.join(".git").exists() {
            let needs_pull = repo_dir.join(".git/FETCH_HEAD")
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| t.elapsed().unwrap_or_default() > std::time::Duration::from_secs(3600))
                .unwrap_or(true);

            if needs_pull {
                let output = tokio::process::Command::new("git")
                    .args(["pull", "--ff-only"])
                    .current_dir(&repo_dir)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped())
                    .output()
                    .await
                    .map_err(|e| AppError::Internal(format!("git pull failed: {e}")))?;

                if !output.status.success() {
                    tracing::warn!(repo = %repo, "git pull failed, using existing clone");
                }
            }

            let head_sha = git_head_sha(&repo_dir).await?;
            return Ok((repo_dir, head_sha));
        }

        std::fs::create_dir_all(&repo_dir)
            .map_err(|e| AppError::Internal(format!("Failed to create cache directory: {e}")))?;

        let url = format!("https://github.com/{repo}.git");
        let output = tokio::process::Command::new("git")
            .args(["clone", "--depth", "1", &url])
            .arg(&repo_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to run git clone: {e}")))?;

        if !output.status.success() {
            let _ = std::fs::remove_dir_all(&repo_dir);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not found") || stderr.contains("does not exist") || stderr.contains("Could not read from remote") {
                return Err(AppError::NotFound(format!("Repository '{repo}' not found")));
            }
            return Err(AppError::Internal(format!("git clone failed: {stderr}")));
        }

        let head_sha = git_head_sha(&repo_dir).await?;
        Ok((repo_dir, head_sha))
    }
}

async fn git_head_sha(repo_dir: &Path) -> Result<String, AppError> {
    let output = tokio::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to get HEAD sha: {e}")))?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Some SKILL.md files embed structured (non-string) values under
/// `metadata` — e.g. platform-specific config blocks such as
/// `metadata.openclaw: {...}`. The `agent-skills` crate's `Metadata` type
/// only accepts string values, so parsing such files fails outright with
/// "invalid type: map, expected a string". Stringify any non-string
/// `metadata` entries before parsing, mirroring the lenient handling our
/// own `parse_frontmatter` already applies when listing installed skills.
/// Falls back to the original content unchanged if anything looks off, so
/// genuinely malformed frontmatter still surfaces its real parse error.
fn sanitize_metadata_frontmatter(content: &str) -> String {
    let trimmed = content.trim_start();
    let Some(after_first) = trimmed.strip_prefix("---") else {
        return content.to_string();
    };
    let Some(end_idx) = after_first.find("\n---") else {
        return content.to_string();
    };

    let yaml_str = &after_first[..end_idx];
    let rest = &after_first[end_idx + 4..];

    let Ok(mut doc) = serde_yaml::from_str::<serde_yaml::Value>(yaml_str) else {
        return content.to_string();
    };
    let Some(mapping) = doc.as_mapping_mut() else {
        return content.to_string();
    };

    let metadata_key = serde_yaml::Value::String("metadata".to_string());
    if let Some(serde_yaml::Value::Mapping(metadata)) = mapping.get_mut(&metadata_key) {
        for (_, value) in metadata.iter_mut() {
            if !matches!(value, serde_yaml::Value::String(_))
                && let Ok(serialized) = serde_yaml::to_string(value)
            {
                *value = serde_yaml::Value::String(serialized.trim().to_string());
            }
        }
    }

    let Ok(sanitized_yaml) = serde_yaml::to_string(&doc) else {
        return content.to_string();
    };

    format!("---\n{sanitized_yaml}---{rest}")
}

fn walk_for_files(dir: &Path, base: &Path, target: &str, results: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default();
        if name == ".git" {
            continue;
        }
        if path.is_dir() {
            walk_for_files(&path, base, target, results);
        } else if name == target
            && let Ok(relative) = path.strip_prefix(base)
        {
            results.push(relative.to_path_buf());
        }
    }
}

fn read_skill_files(skill_dir: &Path) -> Vec<FetchedSkillFile> {
    let mut files = Vec::new();
    collect_skill_files(skill_dir, skill_dir, &mut files);
    files
}

fn collect_skill_files(dir: &Path, base: &Path, files: &mut Vec<FetchedSkillFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default();
        if name == ".git" || name == "SKILL.md" {
            continue;
        }
        if path.is_dir() {
            collect_skill_files(&path, base, files);
        } else if let Ok(content) = std::fs::read(&path)
            && let Ok(relative) = path.strip_prefix(base)
        {
            files.push(FetchedSkillFile {
                path: relative.to_string_lossy().to_string(),
                content,
            });
        }
    }
}

pub fn derive_avatar_url(repo: &str) -> String {
    let owner = repo.split('/').next().unwrap_or(repo);
    format!("https://github.com/{owner}.png")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_metadata_frontmatter_stringifies_nested_map() {
        let content = r#"---
name: pricewin-flight-search
description: Search for flights.
metadata:
  openclaw:
    compatible: true
    version: "1.0"
  author: example-org
---
# Instructions
"#;

        // The raw content fails to parse because `metadata.openclaw` is a map.
        assert!(agent_skills::Skill::parse(content).is_err());

        let sanitized = sanitize_metadata_frontmatter(content);
        let parsed = agent_skills::Skill::parse(&sanitized).expect("should parse after sanitizing");
        assert_eq!(parsed.name().as_str(), "pricewin-flight-search");
        let metadata = parsed.frontmatter().metadata().expect("metadata present");
        assert_eq!(metadata.get("author"), Some("example-org"));
        assert!(metadata.get("openclaw").unwrap().contains("compatible"));
    }

    #[test]
    fn test_sanitize_metadata_frontmatter_leaves_valid_content_unchanged_semantically() {
        let content = r#"---
name: my-skill
description: Does something useful.
metadata:
  author: test
---
Body text.
"#;
        let sanitized = sanitize_metadata_frontmatter(content);
        let parsed = agent_skills::Skill::parse(&sanitized).expect("should still parse");
        assert_eq!(parsed.name().as_str(), "my-skill");
        assert_eq!(parsed.body().trim(), "Body text.");
    }

    #[test]
    fn test_sanitize_metadata_frontmatter_falls_back_on_non_frontmatter_content() {
        let content = "no frontmatter here";
        assert_eq!(sanitize_metadata_frontmatter(content), content);
    }

    #[test]
    fn test_derive_avatar_url() {
        assert_eq!(
            derive_avatar_url("vercel-labs/agent-skills"),
            "https://github.com/vercel-labs.png"
        );
    }

    #[test]
    fn test_derive_avatar_url_no_slash() {
        assert_eq!(derive_avatar_url("owner"), "https://github.com/owner.png");
    }

    #[test]
    fn test_walk_for_files() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        std::fs::create_dir_all(base.join("skill-a")).unwrap();
        std::fs::write(base.join("skill-a/SKILL.md"), "test").unwrap();
        std::fs::create_dir_all(base.join("nested/skill-b")).unwrap();
        std::fs::write(base.join("nested/skill-b/SKILL.md"), "test").unwrap();
        std::fs::write(base.join("README.md"), "readme").unwrap();

        let mut results = Vec::new();
        walk_for_files(base, base, "SKILL.md", &mut results);

        assert_eq!(results.len(), 2);
        let paths: Vec<String> = results.iter().map(|p| p.to_string_lossy().to_string()).collect();
        assert!(paths.contains(&"skill-a/SKILL.md".to_string()));
        assert!(paths.contains(&"nested/skill-b/SKILL.md".to_string()));
    }

    #[test]
    fn test_walk_for_files_root_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        std::fs::write(base.join("SKILL.md"), "test").unwrap();

        let mut results = Vec::new();
        walk_for_files(base, base, "SKILL.md", &mut results);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].to_string_lossy(), "SKILL.md");
    }

    #[test]
    fn test_read_skill_files() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        std::fs::write(base.join("SKILL.md"), "skill content").unwrap();
        std::fs::write(base.join("helper.py"), "print('hello')").unwrap();
        std::fs::create_dir_all(base.join("templates")).unwrap();
        std::fs::write(base.join("templates/prompt.md"), "prompt").unwrap();

        let files = read_skill_files(base);

        assert_eq!(files.len(), 2);
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"helper.py"));
        assert!(paths.contains(&"templates/prompt.md"));
    }
}
