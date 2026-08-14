//! Renaming a page and everything that points at it.
//!
//! Two callers need this: the consolidation reconcile stage (a model-proposed move) and
//! the sync engine (a user dragging a file in Obsidian).
//!
//! A rename touches three things, and missing any one leaves the vault inconsistent:
//!   1. the DB edges (`rename_entity` repoints the path-keyed link tables),
//!   2. the page's own `.md` file (moved, content preserved),
//!   3. the `[[wikilinks]]` baked into every inbound-linking page's **file** - without
//!      which those files keep a dangling `[[old-path]]` until independently re-authored.

use crate::core::error::AppError;
use crate::db::repo::pkm::PkmRepo;

use super::storage::PkmStorage;
use super::vault::VaultScope;

/// Rename one page and every reference to it. The target must be free - callers check
/// `entity_by_path(to).is_none()` first.
///
/// Inbound linkers are captured **before** the DB rename, by the old path: after it, the
/// edges point at `to` and the old association is gone. (The sync directory-rename path
/// needs the opposite order - see [`rewrite_links_after_move`].)
pub(crate) async fn page_everywhere(
    repo: &PkmRepo,
    storage: &PkmStorage,
    vault: &VaultScope,
    user_id: &str,
    from: &str,
    to: &str,
) -> Result<(), AppError> {
    let linkers = repo.entities_linking_to(user_id, from).await.unwrap_or_default();
    repo.rename_entity(user_id, from, to).await?;
    if let Err(e) = storage.move_page_file(vault, from, to) {
        tracing::warn!(error = %e, %from, %to, "pkm rename: page file move failed");
    }
    rewrite_links(storage, vault, &linkers, from, to);
    Ok(())
}

/// Rewrite the mirror's `[[links]]` for a move whose DB rename has **already** happened -
/// the deferred half, for the sync directory rename.
///
/// A directory rename moves many pages in one transaction and only then fixes links, so
/// that a linker which itself moved is rewritten at its final path. That means the
/// linkers can only be found by the *new* path, which is what this queries.
pub(crate) async fn rewrite_links_after_move(
    repo: &PkmRepo,
    storage: &PkmStorage,
    vault: &VaultScope,
    user_id: &str,
    from: &str,
    to: &str,
) {
    let linkers = repo.entities_linking_to(user_id, to).await.unwrap_or_default();
    rewrite_links(storage, vault, &linkers, from, to);
}

fn rewrite_links(
    storage: &PkmStorage,
    vault: &VaultScope,
    linkers: &[String],
    from: &str,
    to: &str,
) {
    for linker in linkers {
        if let Err(e) = storage.rewrite_wikilinks(vault, linker, from, to) {
            tracing::warn!(error = %e, page = %linker, "pkm rename: wikilink rewrite failed");
        }
    }
}
