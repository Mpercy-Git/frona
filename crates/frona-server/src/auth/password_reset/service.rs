use std::sync::Arc;

use chrono::{Duration, Utc};
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

use super::models::PasswordResetToken;
use super::repository::PasswordResetRepository;
use crate::auth::UserService;
use crate::core::error::AppError;
use crate::core::repository::new_id;
use crate::mail::MailService;

#[derive(Clone)]
pub struct PasswordResetService {
    repo: Arc<dyn PasswordResetRepository>,
    expiry_minutes: u64,
}

impl PasswordResetService {
    pub fn new(repo: Arc<dyn PasswordResetRepository>, expiry_minutes: u64) -> Self {
        Self {
            repo,
            expiry_minutes,
        }
    }

    /// Reset secrets are looked up by hash, so the stored value is useless to
    /// anyone who reads the table.
    fn hash(token: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hex::encode(hasher.finalize())
    }

    fn generate_secret() -> String {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
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

    /// Validates and burns a reset secret, returning the user it belongs to.
    /// Expired and unknown secrets are reported identically.
    pub async fn consume(&self, secret: &str) -> Result<String, AppError> {
        let invalid = || {
            AppError::Validation("This reset link is invalid or has expired.".into())
        };

        let token = self
            .repo
            .find_by_hash(&Self::hash(secret))
            .await?
            .ok_or_else(invalid)?;

        if token.expires_at <= Utc::now() {
            let _ = self.repo.delete(&token.id).await;
            return Err(invalid());
        }

        // Single use: burn every outstanding secret for the user, not just this
        // one, so a second link from an earlier request can't also be redeemed.
        self.repo.delete_by_user_id(&token.user_id).await?;
        Ok(token.user_id)
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

        let secret = self.issue(&user.id).await?;
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
