use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use crate::core::error::AppError;
use crate::memory::pkm::ontology::catalogue::core::OntologyCatalogue;
use crate::memory::pkm::ontology::catalogue::loading::ontology_files;
use crate::memory::pkm::ontology::release;

/// Which root a source came from. The distinction is not cosmetic: it is what makes
/// "this vocabulary left because a newer release replaced it" distinguishable from
/// "the user deleted it" during merge-refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Root {
    /// The downloaded frona-ontologies release. Replaced wholesale on upgrade.
    Release,
    /// Ontologies the user added. Trusted fully - it is their knowledge base - but
    /// checked for the one thing that would silently break the walk.
    User,
}

/// The two directories a catalogue is assembled from, and the only place that pairing
/// is spelled out. Carried rather than re-derived so the download task and the loader
/// cannot disagree about where the artifacts go.
#[derive(Clone, Debug)]
pub struct Roots {
    /// The release that ships in the image. **Read-only** - in a container this is a
    /// layer, so anything written here is gone on restart.
    pub release: PathBuf,
    /// The user's own ontologies, on the data volume. The only writable half, so this
    /// is also where anything fetched at runtime lands. Need not exist.
    pub user: PathBuf,
}

impl Roots {
    /// Scan both and absorb into one catalogue.
    ///
    /// `Release` goes first because attribution is assigned on first sight: a term the
    /// release declares has to stay attributed to the release.
    pub fn load(&self) -> Result<Arc<OntologyCatalogue>, AppError> {
        OntologyCatalogue::load(&[
            (Root::Release, &self.release_in_use()),
            (Root::User, &self.user),
        ])
    }

    /// Which directory actually supplies the release: the image's copy if it is
    /// usable, otherwise a repaired one.
    ///
    /// They are alternatives, never both. Each declares the same `owl:Ontology` IRIs,
    /// so scanning both would trip the duplicate-identity check and leave the server
    /// with no catalogue - a repair that broke exactly what it was fixing.
    pub fn release_in_use(&self) -> PathBuf {
        match usable(&self.release) {
            true => self.release.clone(),
            false => release::repair_dir(&self.user),
        }
    }

    /// Does the release need fetching? Only when neither directory is usable.
    pub fn needs_repair(&self) -> bool {
        !usable(&self.release) && !usable(&release::repair_dir(&self.user))
    }
}

/// Can this directory serve as the release half of the catalogue?
///
/// The three cases are genuinely different, and collapsing them is how a corrupt
/// install gets treated as an empty one (or vice versa):
///
///   * **A manifest that verifies** - a published release, intact. Use it.
///   * **No manifest, but ontologies present** - someone assembled this by hand, or a
///     test fixture. Not a release at all, so there is nothing to verify against and
///     nothing to repair; trust it, the same way the user root is trusted.
///   * **A manifest that does not match, or nothing at all** - a damaged or absent
///     release. A truncated artifact still parses and simply holds fewer terms, so
///     "some `.ttl.gz` are present" is not evidence of anything.
pub(super) fn usable(dir: &Path) -> bool {
    match release::verify(dir) {
        Ok(()) => true,
        Err(release::Invalid::NoManifest) => {
            ontology_files(dir).is_ok_and(|f| !f.is_empty())
        }
        Err(_) => false,
    }
}
