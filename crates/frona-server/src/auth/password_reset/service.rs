use std::sync::Arc;

use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};

use super::models::PasswordResetToken;
use super::repository::PasswordResetRepository;
use crate::auth::UserService;
use crate::core::error::AppError;
use crate::core::repository::new_id;
use crate::mail::MailService;

/// Minimum gap between reset emails for one account. Without it anyone who
/// knows an address can have the server mail its owner as fast as the
/// per-IP limit allows, from as many IPs as they like.
const RESET_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(60);

/// Reset emails the server will have in flight at once. Past this, requests
/// are still answered (so nothing reveals the limit) but no mail is queued.
const MAX_CONCURRENT_EMAILS: usize = 8;

#[derive(Clone)]
pub struct PasswordResetService {
    repo: Arc<dyn PasswordResetRepository>,
    expiry_minutes: u64,
    cooldown: std::time::Duration,
    mail_slots: Arc<tokio::sync::Semaphore>,
}

impl PasswordResetService {
    pub fn new(repo: Arc<dyn PasswordResetRepository>, expiry_minutes: u64) -> Self {
        Self {
            repo,
            expiry_minutes,
            cooldown: RESET_COOLDOWN,
            mail_slots: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_EMAILS)),
        }
    }

    /// Overrides the per-account email cooldown. For tests.
    pub fn with_cooldown(mut self, cooldown: std::time::Duration) -> Self {
        self.cooldown = cooldown;
        self
    }

    /// Overrides how many emails may be in flight at once. For tests.
    pub fn with_mail_slots(mut self, slots: usize) -> Self {
        self.mail_slots = Arc::new(tokio::sync::Semaphore::new(slots));
        self
    }

    /// Reserves room to send one reset email, held until the permit is
    /// dropped. `None` when too many are already in flight.
    pub fn try_reserve_mail_slot(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.mail_slots.clone().try_acquire_owned().ok()
    }

    /// Reset secrets are looked up by hash, so the stored value is useless to
    /// anyone who reads the table.
    fn hash(token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hex::encode(hasher.finalize())
    }

    fn generate_secret() -> String {
        // rand 0.10 renamed `RngCore` and moved `OsRng`; `random()` is backed by
        // ThreadRng (ChaCha12, OS-reseeded, `TryCryptoRng`) - the same CSPRNG the
        // vault uses for AES-GCM nonces, so the token keeps its strength.
        let bytes: [u8; 32] = rand::random();
        hex::encode(bytes)
    }

    /// Mints a single-use reset secret for `user_id`, superseding any
    /// outstanding ones, and returns the plaintext to be emailed.
    pub async fn issue(&self, user_id: &str) -> Result<String, AppError> {
        self.repo.delete_by_user_id(user_id).await?;

        let secret = Self::generate_secret();
        let now = Utc::now();
        let token = PasswordResetToken {
            id: new_id(),
            user_id: user_id.to_string(),
            token_hash: Self::hash(&secret),
            expires_at: now + Duration::minutes(self.expiry_minutes as i64),
            created_at: now,
        };
        self.repo.create(&token).await?;
        Ok(secret)
    }

    /// Like [`Self::issue`], but `None` while the account's last reset email
    /// is still inside the cooldown.
    pub async fn issue_if_allowed(&self, user_id: &str) -> Result<Option<String>, AppError> {
        if let Some(last) = self.repo.latest_created_at(user_id).await?
            && (Utc::now() - last)
                .to_std()
                .is_ok_and(|age| age < self.cooldown)
        {
            return Ok(None);
        }
        self.issue(user_id).await.map(Some)
    }

    /// Validates and burns a reset secret, returning the user it belongs to.
    /// Expired and unknown secrets are reported identically.
    ///
    /// The secret is claimed with a single delete that returns what it
    /// removed, so when several requests present the same secret at once
    /// exactly one is let through.
    pub async fn consume(&self, secret: &str) -> Result<String, AppError> {
        let invalid = || AppError::Validation("This reset link is invalid or has expired.".into());

        let user_id = self
            .repo
            .take_by_hash(&Self::hash(secret), Utc::now())
            .await?
            .ok_or_else(invalid)?;

        // Single use: burn every outstanding secret for the user, not just this
        // one, so a second link from an earlier request can't also be redeemed.
        self.repo.delete_by_user_id(&user_id).await?;
        Ok(user_id)
    }

    /// Drops outstanding secrets — called whenever the password changes by some
    /// other route, so a reset link requested beforehand can't be used after.
    pub async fn invalidate_for_user(&self, user_id: &str) {
        if let Err(e) = self.repo.delete_by_user_id(user_id).await {
            tracing::warn!(user_id = %user_id, error = %e, "Failed to clear password reset tokens");
        }
    }

    pub async fn purge_expired(&self) -> Result<(), AppError> {
        self.repo.delete_expired().await
    }

    /// The full "user asked for a reset" path: resolve the address, mint a
    /// secret, and mail the link. A miss is not an error — callers must not be
    /// able to tell registered addresses from unregistered ones.
    pub async fn send_reset_email(
        &self,
        user_service: &UserService,
        mail: &MailService,
        frontend_url: &str,
        email: &str,
        expiry_minutes: u64,
    ) -> Result<(), AppError> {
        let normalized = crate::auth::AuthService::normalize_email(email);
        let Some(user) = user_service.find_by_email(&normalized).await? else {
            tracing::info!("Password reset requested for an unregistered address");
            return Ok(());
        };
        if user.deactivated_at.is_some() {
            tracing::info!(user_id = %user.id, "Password reset requested for a deactivated account");
            return Ok(());
        }

        let Some(secret) = self.issue_if_allowed(&user.id).await? else {
            tracing::info!(user_id = %user.id, "Password reset requested inside the cooldown; no email sent");
            return Ok(());
        };
        let link = format!(
            "{}/reset-password?token={}",
            frontend_url.trim_end_matches('/'),
            secret
        );

        let body = format!(
            "Hi {},\n\n\
             Someone asked to reset the password for your Frona account. Open the link \
             below to choose a new one:\n\n\
             {}\n\n\
             The link works once and expires in {} minutes. If you didn't ask for this, \
             you can ignore this email — your password stays as it is.\n",
            user.name, link, expiry_minutes
        );

        mail.send(&user.email, "Reset your Frona password", body)
            .await?;
        tracing::info!(user_id = %user.id, "Password reset email sent");
        Ok(())
    }
}
