//! What a page *is* on disk: the bytes we compose, the bytes we read back, and the `rev`
//! that identifies them.
//!
//! One page → one markdown file. Frontmatter carries the structured half (metadata,
//! attributes, typed `[[wikilinks]]`) so a plain `read` is a precise field-pull; the body
//! is the human-editable half. Both directions live here - [`compose_page`] renders a page
//! to bytes, [`MarkdownPage::parse`] and [`extract_body`] recover the editable prose from
//! bytes a model or a human wrote.
//!
//! # The file-bytes ↔ `rev` invariant
//!
//! A page's `rev` is the sha256 of the exact bytes on disk ([`sha256_hex`]). The sync
//! manifest is a `path → rev` map and the CAS token on every human edit is a `rev`, so a
//! stored `rev` that does not match the file is not a stale cache - it is a client told
//! "your copy is current" about bytes it has never seen, or a conflict raised against an
//! edit that was in fact clean.
//!
//! That is why composition and hashing are one module: the rev is the hash of exactly what
//! [`compose_page`] produced.
//!
//! [`PkmStorage`] is the other half of the pair and stays deliberately separate: it knows
//! where files go, and nothing about what is in them.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use chrono::Utc;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use sha2::{Digest, Sha256};

use crate::core::error::AppError;
use crate::db::repo::pkm::PkmRepo;
use crate::memory::pkm::ontology::PrefixMap;

use super::model::{EntityCategory, KnowledgeEntity, KnowledgeEntityLink};
use super::storage::PkmStorage;
use super::vault::VaultScope;

pub(crate) fn canonicalize_wikilinks(
    markdown: &str,
    redirects: &BTreeMap<String, String>,
    scope: &VaultScope,
) -> String {
    if redirects.is_empty() || !markdown.contains("[[") {
        return markdown.to_string();
    }
    static WIKILINK: OnceLock<regex::Regex> = OnceLock::new();
    let wikilink = WIKILINK.get_or_init(|| {
        regex::Regex::new(r"\[\[([^\[\]|#]+)((?:#[^\[\]|]*)?(?:\|[^\[\]]*)?)\]\]")
            .expect("wikilink expression is valid")
    });
    let vault_prefix = format!("{}/", scope.directory());
    wikilink
        .replace_all(markdown, |captures: &regex::Captures<'_>| {
            let target = &captures[1];
            let (path, prefixed) = match target.strip_prefix(&vault_prefix) {
                Some(path) => (path, true),
                None => (target, false),
            };
            let canonical = canonical_path(path, redirects);
            if canonical == path {
                return captures[0].to_string();
            }
            let target = if prefixed {
                scope.vault_path(&canonical)
            } else {
                canonical
            };
            format!("[[{target}{}]]", &captures[2])
        })
        .into_owned()
}

pub(crate) fn canonical_path(path: &str, redirects: &BTreeMap<String, String>) -> String {
    let mut canonical = path;
    let mut seen = std::collections::HashSet::new();
    while seen.insert(canonical) {
        let Some(next) = redirects.get(canonical) else {
            break;
        };
        canonical = next;
    }
    canonical.to_string()
}

/// A parsed markdown page body: lenient, language-agnostic syntax cleanup already
/// applied, plus the structural facts `compose_page` needs. Produced by
/// [`MarkdownPage::parse`] in a single parser pass.
pub struct MarkdownPage {
    /// The cleaned body - whole-document code fence unwrapped, leading frontmatter
    /// dropped, trimmed.
    pub body: String,
    /// Whether `body` opens with an ATX heading (i.e. it carries its own title).
    pub has_title: bool,
}

impl MarkdownPage {
    /// Parse and clean a raw model (or deterministic) body. Single parser pass for the
    /// cleanup; `has_title` is a cheap syntax check on the result.
    pub fn parse(raw: &str) -> Self {
        let body = clean(raw);
        let has_title = opens_with_heading(&body);
        Self { body, has_title }
    }
}

/// Lenient, single-pass, language-agnostic cleanup of an authored body. Inspects the
/// first markdown block: if the whole document is one code fence, return its inner
/// text (the model wrapped everything in ```); if it's a leading YAML frontmatter
/// block, drop it and keep the rest. Otherwise leave it untouched. Keys on markdown
/// *syntax* only - never on heading text - so it is locale-independent.
fn clean(raw: &str) -> String {
    let trimmed = raw.trim();
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    let mut iter = Parser::new_ext(trimmed, opts).into_offset_iter();
    match iter.next() {
        Some((Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))), _)) => {
            let mut inner = String::new();
            for (ev, _) in iter.by_ref() {
                match ev {
                    Event::Text(t) => inner.push_str(&t),
                    Event::End(TagEnd::CodeBlock) => {
                        return if iter.next().is_none() {
                            inner.trim().to_string()
                        } else {
                            trimmed.to_string()
                        };
                    }
                    _ => {}
                }
            }
            trimmed.to_string()
        }
        Some((Event::Start(Tag::MetadataBlock(_)), _)) => {
            for (ev, range) in iter {
                if matches!(ev, Event::End(TagEnd::MetadataBlock(_))) {
                    return trimmed[range.end..].trim_start().to_string();
                }
            }
            trimmed.to_string()
        }
        _ => trimmed.to_string(),
    }
}

/// True if `body` opens with an ATX heading of any level (`#` … `######`) - i.e. it
/// already carries its own title. Pure markdown syntax (a run of 1–6 `#` followed by
/// whitespace), so it's language-independent and, unlike a bare `# ` check, accepts a
/// `##`-level lead heading without double-titling.
fn opens_with_heading(body: &str) -> bool {
    let Some(first) = body.lines().find(|l| !l.trim().is_empty()) else {
        return false;
    };
    let first = first.trim_start();
    let hashes = first.bytes().take_while(|&b| b == b'#').count();
    (1..=6).contains(&hashes) && first[hashes..].starts_with(|c: char| c.is_whitespace())
}

/// Extract just the human-editable body of an existing page. Frontmatter is removed, as
/// is the legacy machine-generated `## History` ledger when reading an older projection.
/// Fed to the writer on a dirty page via sync `adopt` so user prose is preserved.
pub fn extract_body(page: &str) -> String {
    let after_fm = page
        .strip_prefix("---\n")
        .and_then(|rest| rest.find("\n---\n").map(|i| &rest[i + 5..]))
        .unwrap_or(page);
    let mut body = after_fm.trim_start();
    if body.starts_with("# ")
        && let Some(nl) = body.find('\n')
    {
        body = body[nl + 1..].trim_start();
    }
    let mut end = body.len();
    if let Some(i) = body.find("\n## History") {
        end = end.min(i);
    }
    body[..end].trim().to_string()
}

/// Pull the `uid:` value out of a page's leading YAML frontmatter (the id
/// [`compose_page`] stamps). None if the file has no frontmatter or no `uid` key - that
/// marks a file we didn't mint, which recovery leaves untouched.
///
/// Read back by [`PkmStorage::list_page_files`], which walks the vault and needs each
/// file's durable identity rather than its content.
pub(super) fn parse_uid(content: &str) -> Option<String> {
    let rest = content.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    rest[..end].lines().find_map(|line| {
        line.strip_prefix("uid:")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    })
}

/// Compose the full page: YAML frontmatter (metadata + `attributes` + typed
/// `[[wikilinks]]` grouped by relation) plus the authored article. Superseded memories
/// remain durable in the database and are integrated selectively by Page Author.
pub fn compose_page(
    page: &KnowledgeEntity,
    article: &MarkdownPage,
    attributes: &serde_json::Value,
    links: &[KnowledgeEntityLink],
    prefixes: &PrefixMap,
    scope: &VaultScope,
) -> String {
    let category = match page.category {
        EntityCategory::Concept => "concept",
        EntityCategory::Playbook => "playbook",
    };
    let mut s = String::new();
    s.push_str("---\n");
    // The page's immutable record id - the file's durable identity, independent of
    // its (mutable) path. Lets recovery match a file to its DB page after a rename,
    // so a crash-orphaned file is relocated (not rebuilt). See `reconcile_files`.
    s.push_str(&format!("uid: {}\n", page.id));
    // CURIEs, in array form. Display-only: frontmatter is re-derived on every
    // write and `MarkdownPage::parse` reads only `body`/`has_title`, so it never
    // round-trips and a prefix shift is cosmetic.
    s.push_str(&format!(
        "type: [{}]\n",
        prefixes.display_joined(&page.kinds)
    ));
    s.push_str(&format!("title: {}\n", page.name));
    s.push_str(&format!(
        "description: {}\n",
        page.description.replace('\n', " ")
    ));
    s.push_str(&format!("path: {}\n", page.path));
    s.push_str(&format!("category: {category}\n"));
    s.push_str(&format!("use_count: {}\n", page.use_count));
    s.push_str(&format!("updated: {}\n", Utc::now().to_rfc3339()));
    if !page.related_playbooks.is_empty() {
        s.push_str("related_playbooks:\n");
        for path in &page.related_playbooks {
            s.push_str(&format!("  - \"[[{}]]\"\n", scope.vault_path(path)));
        }
    }

    // Attributes as machine-parseable frontmatter keys (JSON scalars are
    // valid YAML, so quoting/typing is preserved).
    if let Some(map) = attributes.as_object().filter(|m| !m.is_empty()) {
        s.push_str("attributes:\n");
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        for k in keys {
            let v = serde_json::to_string(&map[k]).unwrap_or_else(|_| "null".into());
            s.push_str(&format!("  {}: {v}\n", yaml_key(k)));
        }
    }

    let mut by_rel: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for l in links {
        by_rel
            .entry(l.relation.as_str())
            .or_default()
            .push(l.to_entity_path.as_str());
    }
    // Links are emitted as absolute vault-root wikilinks - `[[<directory>/<to>]]`
    // - so they resolve in Obsidian and by the agent's root-prepend rule. The
    // stored `to_entity_path` is clean; the directory is the render prefix.
    for (rel, tos) in &by_rel {
        s.push_str(&format!("{}:\n", yaml_key(rel)));
        for to in tos {
            s.push_str(&format!("  - \"[[{}]]\"\n", scope.vault_path(to)));
        }
    }
    s.push_str("---\n\n");

    // The model authors its own title; if it opened with prose instead of a
    // heading, add one (`article.has_title` already accounts for any ATX level,
    // so a `##` lead heading isn't double-titled).
    if !article.has_title {
        s.push_str(&format!("# {}\n\n", page.name));
    }
    s.push_str(article.body.trim());
    s.push_str("\n\n");

    if !page.related_playbooks.is_empty() {
        s.push_str("## Related Playbooks\n\n");
        for path in &page.related_playbooks {
            s.push_str(&format!("- [[{}]]\n", scope.vault_path(path)));
        }
        s.push('\n');
    }

    s
}

/// Render a frontmatter mapping key safely. A CURIE key (`schema:name`,
/// `frona:worksFor`) is valid YAML-LD but its colon trips Obsidian's Properties UI
/// and naive parsers, so quote any key containing a `:`; plain keys stay bare (so
/// existing free-text attribute keys render unchanged).
fn yaml_key(key: &str) -> std::borrow::Cow<'_, str> {
    if key.contains(':') {
        std::borrow::Cow::Owned(format!("\"{key}\""))
    } else {
        std::borrow::Cow::Borrowed(key)
    }
}

/// The sync `rev` of a rendered page - `sha256` (hex) of its exact file bytes.
/// The CAS token on edit and the change-detector in the manifest; opaque to the
/// client (the server defines it, the client only compares).
pub fn sha256_hex(bytes: &str) -> String {
    let mut h = Sha256::new();
    h.update(bytes.as_bytes());
    format!("{:x}", h.finalize())
}

/// Commit the exact canonical bytes and revision, then update the page's `.md` mirror.
/// A failed or interrupted mirror write remains recoverable from the database.
pub(crate) async fn write_page_and_rev(
    repo: &PkmRepo,
    storage: &PkmStorage,
    vault: &VaultScope,
    user_id: &str,
    path: &str,
    file: &str,
) -> Result<String, AppError> {
    let rev = sha256_hex(file);
    repo.set_page_projection(user_id, path, file, &rev).await?;
    storage.write_page(vault, path, file)?;
    Ok(rev)
}

/// Re-stamp a page's `rev` from whatever is on disk - the other half, for a page whose
/// bytes moved rather than changed (a rename). Returns the new rev, or `None` when there
/// is no file to hash.
///
/// Nothing is stamped in that case, deliberately. Hashing an absent file yields the
/// sha256 of the empty string, which is a *claim* - "this page is empty" - and a client
/// that stores it agrees with the server that a page with no file is in sync. Leaving the
/// previous rev in place keeps the row visibly out of step until the boot
/// `reconcile_files` pass re-renders the file and stamps it for real.
pub(crate) async fn restamp_rev(
    repo: &PkmRepo,
    storage: &PkmStorage,
    vault: &VaultScope,
    user_id: &str,
    path: &str,
) -> Result<Option<String>, AppError> {
    let Some(content) = storage.read_page(vault, path) else {
        return Ok(None);
    };
    let rev = sha256_hex(&content);
    repo.set_page_projection(user_id, path, &content, &rev)
        .await?;
    Ok(Some(rev))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Handle;
    use std::path::PathBuf;

    /// A scope is pure to build - composition needs one only for the wikilink prefix,
    /// so no filesystem is involved in any test here.
    fn vault() -> VaultScope {
        VaultScope::new(
            Handle::try_new("testuser").unwrap(),
            "Memory",
            PathBuf::from("/tmp/pkm-test"),
        )
        .unwrap()
    }

    #[test]
    fn merged_wikilinks_follow_chained_redirects_without_losing_labels_or_fragments() {
        let redirects = BTreeMap::from([
            (
                "hardware/device-x".into(),
                "devices/example-tools-device-x".into(),
            ),
            (
                "devices/example-tools-device-x".into(),
                "tools/example-tools-device-x".into(),
            ),
        ]);
        let rewritten = canonicalize_wikilinks(
            "Use [[hardware/device-x|the screwdriver]] and \
             [[Memory/hardware/device-x#Firmware|its firmware]]. \
             Keep [[hardware/device-x-compatible-case]] distinct.",
            &redirects,
            &vault(),
        );
        assert!(rewritten.contains("[[tools/example-tools-device-x|the screwdriver]]"));
        assert!(
            rewritten.contains("[[Memory/tools/example-tools-device-x#Firmware|its firmware]]")
        );
        assert!(rewritten.contains("[[hardware/device-x-compatible-case]]"));
        assert!(!rewritten.contains("[[hardware/device-x|"));
        assert!(!rewritten.contains("[[Memory/hardware/device-x#"));
    }

    #[test]
    fn curie_frontmatter_keys_are_quoted_and_liftable() {
        use crate::memory::pkm::model::{
            AttributeSource, EntityCategory, EntityOrigin, KnowledgeEntity, KnowledgeEntityLink,
            LinkOrigin,
        };
        let now = chrono::Utc::now();
        let page = KnowledgeEntity {
            id: "id1".into(),
            user_id: "u".into(),
            path: "services/postgres".into(),
            origin: EntityOrigin::Internal,
            category: EntityCategory::Concept,
            kinds: vec!["urn:frona:Service".into(), "frona:Service".into()],
            name: "Postgres".into(),
            description: "the db".into(),
            identity_evidence: Vec::new(),
            attribute_sources: vec![AttributeSource {
                property: "frona:port".into(),
                value: serde_json::json!(5432),
                source_memory_ids: vec!["memory-secret".into()],
            }],
            source_memory_ids: vec!["memory-secret".into()],
            body: String::new(),
            sync_content: None,
            mirrored_rev: None,
            extracted_rev: None,
            related_playbooks: vec!["playbooks/backup-postgres".into()],
            search_text: String::new(),
            search_names: Vec::new(),
            search_name_tokens: Vec::new(),
            search_assertions: Vec::new(),
            attributes: serde_json::json!({ "frona:port": 5432, "host": "db.internal" }),
            use_count: 0,
            aliases: Default::default(),
            rev: None,
            updated_at: now,
            rendered_at: now,
        };
        let link = KnowledgeEntityLink {
            id: "l1".into(),
            user_id: "u".into(),
            from_entity_path: "services/postgres".into(),
            to_entity_path: "services/redis".into(),
            relation: "frona:dependsOn".into(),
            source_memory_ids: vec!["memory-secret".into()],
            origin: LinkOrigin::Asserted,
            created_at: now,
        };
        let article = MarkdownPage {
            body: "# Postgres\nbody".into(),
            has_title: true,
        };
        let rendered = compose_page(
            &page,
            &article,
            &page.attributes,
            std::slice::from_ref(&link),
            &PrefixMap::standard(),
            &vault(),
        );
        assert!(!rendered.contains("memory-secret"));
        assert!(!rendered.contains("attribute_sources"));
        assert!(!rendered.contains("source_memory_ids"));
        assert!(
            rendered.contains("related_playbooks:\n  - \"[[Memory/playbooks/backup-postgres]]\"")
        );
        assert!(
            rendered.contains("## Related Playbooks\n\n- [[Memory/playbooks/backup-postgres]]")
        );

        assert!(
            rendered.contains("\"frona:dependsOn\":"),
            "relation CURIE key quoted:\n{rendered}"
        );
        assert!(
            rendered.contains("\"frona:port\": 5432"),
            "attribute CURIE key quoted:\n{rendered}"
        );
        assert!(
            rendered.contains("  host: "),
            "plain attribute key stays bare:\n{rendered}"
        );
        assert!(
            rendered.contains("type: [frona:Service]"),
            "type is a CURIE array, even at one element:\n{rendered}"
        );

        let fm = rendered.split("---\n").nth(1).expect("frontmatter block");
        let v: serde_yaml::Value = serde_yaml::from_str(fm).expect("frontmatter parses");
        assert!(
            v.get("frona:dependsOn").is_some(),
            "relation key parsed as the CURIE"
        );
        assert!(
            v.get("attributes")
                .and_then(|a| a.get("frona:port"))
                .is_some(),
            "attribute CURIE key parsed"
        );
        let types: Vec<&str> = v
            .get("type")
            .and_then(|t| t.as_sequence())
            .expect("type is a sequence")
            .iter()
            .filter_map(|t| t.as_str())
            .collect();
        assert_eq!(types, ["frona:Service"], "and it parses back as one");
    }

    /// The `uid:` stamp is the file's durable identity, so it has to survive a read back
    /// out of what `compose_page` wrote - and be absent for a file we did not mint.
    #[test]
    fn uid_stamp_round_trips_and_is_absent_from_a_foreign_file() {
        assert_eq!(
            parse_uid("---\nuid: page:abc\ntitle: X\n---\n\nbody"),
            Some("page:abc".into())
        );
        assert_eq!(parse_uid("---\ntitle: X\n---\n\nbody"), None, "no uid key");
        assert_eq!(parse_uid("# Just prose\n\nno frontmatter"), None);
        assert_eq!(
            parse_uid("---\nuid:   \n---\n"),
            None,
            "blank uid is not an id"
        );
    }

    #[test]
    fn opens_with_heading_accepts_any_atx_level() {
        assert!(opens_with_heading("# Title\n\nBody"));
        assert!(opens_with_heading("## Title\n\nBody"), "H2 lead heading");
        assert!(
            opens_with_heading("\n\n   ### Title"),
            "leading blank lines and indent are skipped"
        );
        assert!(!opens_with_heading("Just prose, no heading."));
        assert!(!opens_with_heading("#no-space-is-not-a-heading"));
        assert!(!opens_with_heading("####### seven hashes is not a heading"));
        assert!(!opens_with_heading("   "));
    }

    #[test]
    fn parse_unwraps_whole_document_fence() {
        assert_eq!(
            MarkdownPage::parse("```markdown\nHello **world**.\n```").body,
            "Hello **world**."
        );
        assert_eq!(
            MarkdownPage::parse("```\nbare fence\n```").body,
            "bare fence"
        );
        let mixed = "Intro.\n\n```rust\nfn main() {}\n```\n\nOutro.";
        assert_eq!(MarkdownPage::parse(mixed).body, mixed);
    }

    #[test]
    fn parse_strips_leading_frontmatter() {
        let raw = "---\ntitle: X\n---\n\nProse here.";
        assert_eq!(MarkdownPage::parse(raw).body, "Prose here.");
    }

    #[test]
    fn parse_leaves_thematic_break_and_headings_alone() {
        let hr = "Intro.\n\n---\n\nAfter the break.";
        assert_eq!(MarkdownPage::parse(hr).body, hr);
        let titled = "# Title\n\nBody.";
        assert_eq!(MarkdownPage::parse(titled).body, titled);
    }

    #[test]
    fn parse_reports_has_title() {
        assert!(MarkdownPage::parse("# Local Postgres\n\nprose").has_title);
        assert!(
            MarkdownPage::parse("## Local Postgres\n\nprose").has_title,
            "an H2 lead heading is still a title"
        );
        assert!(!MarkdownPage::parse("Just prose, no title.").has_title);
        assert!(MarkdownPage::parse("```\n# Titled\n\nbody\n```").has_title);
    }

    /// The rev is over *exact* bytes - no trimming, no normalisation. A trailing newline
    /// is a different file, which is the whole point: a client comparing revs must see a
    /// difference wherever the bytes differ, or it is told a stale copy is current.
    #[test]
    fn rev_is_over_exact_bytes() {
        let bytes = "---\nuid: page:1\n---\n\n# X\n\nbody\n";
        assert_eq!(sha256_hex(bytes), sha256_hex(bytes), "deterministic");
        assert_ne!(
            sha256_hex(bytes),
            sha256_hex(bytes.trim_end()),
            "a trailing newline counts"
        );
        assert_ne!(
            sha256_hex(bytes),
            sha256_hex(&bytes.replace("# X", "# Y")),
            "content counts"
        );
        assert_eq!(sha256_hex(bytes).len(), 64, "hex sha256");
    }
}
