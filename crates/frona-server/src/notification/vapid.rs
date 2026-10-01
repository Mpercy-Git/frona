//! VAPID application-server keys for Web Push.
//!
//! Web Push needs a P-256 key pair: the browser subscribes to the public half
//! and the server signs every push with the private half. Requiring an
//! operator to produce one by hand and paste both halves into the environment
//! left most self-hosted servers with push permanently dead — the settings
//! page could only report "this server has no usable VAPID key pair", which
//! describes a setup step nobody skipped on purpose, not a decision.
//!
//! So the server resolves its own pair: an explicitly configured key always
//! wins, and otherwise one is generated once and kept in
//! `{data_dir}/system/vapid.json`. Persisting it is the whole point — a pair
//! regenerated on every boot silently invalidates every subscription browsers
//! already made, and those devices go quiet with nothing to see but a 403
//! from the push service.

use std::path::{Path, PathBuf};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use web_push::VapidSignatureBuilder;

use crate::core::config::PushConfig;
use crate::core::error::AppError;

/// Location of the generated key pair, relative to the data directory.
const KEY_FILE: &str = "system/vapid.json";

/// The stored pair. The public key is derived from the private one and is
/// written only so the file is readable by a human wiring up another client;
/// on load the private key remains the single source of truth.
#[derive(Serialize, Deserialize)]
struct StoredKeys {
    public_key: String,
    private_key: String,
}

/// Fill in `push` with a usable VAPID key pair, generating and persisting one
/// if the deployment has not configured its own.
///
/// Never fails the caller: push notifications are a feature, not a
/// prerequisite for the server running. Anything that goes wrong is logged
/// with what it means for the user, and `PushSender` stays disabled.
pub fn ensure_keys(push: &mut PushConfig, data_dir: &str) {
    normalize(&mut push.vapid_public_key);
    normalize(&mut push.vapid_private_key);

    if let Some(private_key) = push.vapid_private_key.clone() {
        adopt(push, private_key, KeySource::Configured);
        return;
    }

    if push.vapid_public_key.is_some() {
        // Half a key pair is worse than none: browsers subscribe happily
        // against the public key and every send afterwards is unsigned and
        // impossible. Generating a fresh pair here would be a silent
        // substitution of the operator's key, so say what is missing instead.
        tracing::error!(
            "Push notifications disabled: a VAPID public key is configured but the private key is \
             missing, so devices subscribe successfully and then never receive anything. Set \
             FRONA_PUSH_VAPID_PRIVATE_KEY, or unset FRONA_PUSH_VAPID_PUBLIC_KEY to let the server \
             generate and keep its own key pair."
        );
        return;
    }

    let path = key_file_path(data_dir);
    match load_stored(&path) {
        Stored::Private(private_key) => {
            adopt(push, private_key, KeySource::Stored(&path));
            return;
        }
        // Generating over a file that exists but cannot be read would destroy
        // the only copy of a key every subscribed device is already using, for
        // what may be a permissions slip. Push stays off until someone looks.
        Stored::Unusable => return,
        Stored::Missing => {}
    }

    let (public_key, private_key) = match generate_key_pair() {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!(
                error = %e,
                "Push notifications disabled: could not generate a VAPID key pair. Set \
                 FRONA_PUSH_VAPID_PUBLIC_KEY and FRONA_PUSH_VAPID_PRIVATE_KEY (generate them with \
                 `npx web-push generate-vapid-keys`)."
            );
            return;
        }
    };

    match store_keys(&path, &public_key, &private_key) {
        Ok(()) => tracing::info!(
            path = %path.display(),
            "Generated a VAPID key pair for Web Push"
        ),
        Err(e) => tracing::error!(
            path = %path.display(),
            error = %e,
            "Generated a VAPID key pair but could not save it. Push works until the next restart, \
             after which every device has to be re-enabled by hand. Fix the data directory's \
             permissions, or set FRONA_PUSH_VAPID_PUBLIC_KEY and FRONA_PUSH_VAPID_PRIVATE_KEY."
        ),
    }

    push.vapid_public_key = Some(public_key);
    push.vapid_private_key = Some(private_key);
}

enum KeySource<'a> {
    Configured,
    Stored(&'a Path),
}

/// Take `private_key` as the pair's private half and make the config's public
/// half agree with it.
///
/// A public key that does not match is not a cosmetic mismatch: the browser
/// subscribes with what this server serves, and the push service checks the
/// signature against exactly that. Deriving it from the private key is the
/// only combination that can ever deliver.
fn adopt(push: &mut PushConfig, private_key: String, source: KeySource<'_>) {
    let Some(derived) = public_from_private(&private_key) else {
        match source {
            KeySource::Configured => tracing::error!(
                "Push notifications disabled: the configured VAPID private key could not be \
                 parsed. It must be the base64url-encoded P-256 private key produced by \
                 `npx web-push generate-vapid-keys`."
            ),
            KeySource::Stored(path) => tracing::error!(
                path = %path.display(),
                "Push notifications disabled: the stored VAPID private key could not be parsed. \
                 Delete the file to have a fresh pair generated — every device then has to be \
                 re-enabled."
            ),
        }
        return;
    };

    if push
        .vapid_public_key
        .as_ref()
        .is_some_and(|configured| configured != &derived)
    {
        tracing::warn!(
            "The configured VAPID public key does not match the configured private key. Serving \
             the key derived from the private key instead — devices subscribed with the old \
             public key must be re-enabled."
        );
    }

    push.vapid_public_key = Some(derived);
    push.vapid_private_key = Some(private_key);
}

/// Generate a fresh VAPID pair as `(public_key, private_key)`, both
/// base64url-encoded without padding — the encoding `web-push` reads and the
/// one a browser expects for `applicationServerKey`.
pub fn generate_key_pair() -> Result<(String, String), AppError> {
    // A random 32-byte scalar is a valid P-256 private key with overwhelming
    // probability; the vanishing chance of a reject (zero, or past the curve
    // order) is worth a retry, not a report.
    for _ in 0..8 {
        let bytes: [u8; 32] = rand::random();
        let private_key = URL_SAFE_NO_PAD.encode(bytes);
        if let Some(public_key) = public_from_private(&private_key) {
            return Ok((public_key, private_key));
        }
    }
    Err(AppError::Internal(
        "Could not generate a valid P-256 key pair".into(),
    ))
}

/// Derive the base64url public key a browser subscribes with, or `None` if the
/// private key is not one `web-push` can sign with.
pub fn public_from_private(private_key: &str) -> Option<String> {
    let builder = VapidSignatureBuilder::from_base64_no_sub(private_key.trim()).ok()?;
    Some(URL_SAFE_NO_PAD.encode(builder.get_public_key()))
}

fn key_file_path(data_dir: &str) -> PathBuf {
    Path::new(data_dir).join(KEY_FILE)
}

/// What the key file on disk has to say.
enum Stored {
    /// No file yet — this server has never generated a pair.
    Missing,
    /// A private key to use.
    Private(String),
    /// A file is there but cannot be used, and must not be written over.
    Unusable,
}

fn load_stored(path: &Path) -> Stored {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Stored::Missing,
        Err(e) => {
            tracing::error!(
                path = %path.display(),
                error = %e,
                "Push notifications disabled: the stored VAPID key file could not be read. Fix its \
                 permissions, or delete it to have a fresh pair generated — every device then has \
                 to be re-enabled."
            );
            return Stored::Unusable;
        }
    };

    match serde_json::from_str::<StoredKeys>(&contents) {
        Ok(stored) => Stored::Private(stored.private_key),
        Err(e) => {
            tracing::error!(
                path = %path.display(),
                error = %e,
                "Push notifications disabled: the stored VAPID key file could not be parsed. \
                 Delete it to have a fresh pair generated — every device then has to be \
                 re-enabled."
            );
            Stored::Unusable
        }
    }
}

fn store_keys(path: &Path, public_key: &str, private_key: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let body = serde_json::to_string_pretty(&StoredKeys {
        public_key: public_key.to_string(),
        private_key: private_key.to_string(),
    })
    .map_err(|e| e.to_string())?;

    // Write-then-rename: a half-written file here reads back as a corrupt key
    // on the next boot, which costs every device its subscription.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    restrict_permissions(&tmp);
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

fn normalize(field: &mut Option<String>) {
    let trimmed = field.as_ref().map(|v| v.trim().to_string());
    *field = trimmed.filter(|v| !v.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> PushConfig {
        PushConfig::default()
    }

    #[test]
    fn generated_pair_round_trips() {
        let (public_key, private_key) = generate_key_pair().unwrap();
        assert_eq!(
            public_from_private(&private_key).as_deref(),
            Some(public_key.as_str())
        );
        // 65-byte uncompressed P-256 point, base64url without padding.
        assert_eq!(URL_SAFE_NO_PAD.decode(&public_key).unwrap().len(), 65);
        assert_eq!(URL_SAFE_NO_PAD.decode(&private_key).unwrap().len(), 32);
    }

    #[test]
    fn generates_and_then_reuses_the_stored_pair() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();

        let mut first = config();
        ensure_keys(&mut first, &data_dir);
        assert!(first.vapid_private_key.is_some());
        assert!(first.vapid_public_key.is_some());

        // Restarting must not hand browsers a different key than the one their
        // existing subscriptions were made with.
        let mut second = config();
        ensure_keys(&mut second, &data_dir);
        assert_eq!(second.vapid_private_key, first.vapid_private_key);
        assert_eq!(second.vapid_public_key, first.vapid_public_key);
    }

    #[cfg(unix)]
    #[test]
    fn stored_pair_is_not_world_readable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        ensure_keys(&mut config(), &dir.path().to_string_lossy());

        let mode = std::fs::metadata(key_file_path(&dir.path().to_string_lossy()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "key file must not be group/world readable");
    }

    #[test]
    fn configured_keys_win_over_generation() {
        let dir = tempfile::tempdir().unwrap();
        let (public_key, private_key) = generate_key_pair().unwrap();

        let mut push = config();
        push.vapid_public_key = Some(public_key.clone());
        push.vapid_private_key = Some(private_key.clone());
        ensure_keys(&mut push, &dir.path().to_string_lossy());

        assert_eq!(push.vapid_private_key, Some(private_key));
        assert_eq!(push.vapid_public_key, Some(public_key));
        assert!(
            !key_file_path(&dir.path().to_string_lossy()).exists(),
            "a configured pair must not be persisted or replaced"
        );
    }

    #[test]
    fn mismatched_public_key_is_replaced_by_the_derived_one() {
        let dir = tempfile::tempdir().unwrap();
        let (_, private_key) = generate_key_pair().unwrap();
        let (other_public, _) = generate_key_pair().unwrap();

        let mut push = config();
        push.vapid_public_key = Some(other_public.clone());
        push.vapid_private_key = Some(private_key.clone());
        ensure_keys(&mut push, &dir.path().to_string_lossy());

        assert_eq!(push.vapid_public_key, public_from_private(&private_key));
        assert_ne!(push.vapid_public_key, Some(other_public));
    }

    #[test]
    fn blank_configured_keys_are_treated_as_unset() {
        let dir = tempfile::tempdir().unwrap();
        let mut push = config();
        push.vapid_public_key = Some("  ".into());
        push.vapid_private_key = Some(String::new());
        ensure_keys(&mut push, &dir.path().to_string_lossy());

        let private_key = push.vapid_private_key.expect("a pair should be generated");
        assert_eq!(push.vapid_public_key, public_from_private(&private_key));
    }

    #[test]
    fn a_public_key_without_its_private_half_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (public_key, _) = generate_key_pair().unwrap();

        let mut push = config();
        push.vapid_public_key = Some(public_key.clone());
        ensure_keys(&mut push, &dir.path().to_string_lossy());

        // Substituting a generated pair here would silently retire the
        // operator's key; push stays off instead.
        assert_eq!(push.vapid_public_key, Some(public_key));
        assert_eq!(push.vapid_private_key, None);
    }

    #[test]
    fn an_unusable_stored_key_is_not_silently_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();
        let path = key_file_path(&data_dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = r#"{"public_key":"nope","private_key":"not-a-key"}"#;
        std::fs::write(&path, file).unwrap();

        let mut push = config();
        ensure_keys(&mut push, &data_dir);

        assert_eq!(push.vapid_private_key, None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), file);
    }

    #[test]
    fn a_corrupt_key_file_is_not_silently_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().to_string();
        let path = key_file_path(&data_dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();

        let mut push = config();
        ensure_keys(&mut push, &data_dir);

        assert_eq!(push.vapid_private_key, None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }
}
