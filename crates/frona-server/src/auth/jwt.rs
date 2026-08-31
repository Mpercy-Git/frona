use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Serialize, de::DeserializeOwned};

use crate::core::error::{AppError, AuthErrorCode};

#[derive(Clone, Default)]
pub struct JwtService;

impl JwtService {
    pub fn new() -> Self {
        Self
    }

    pub fn sign<T: Serialize>(
        &self,
        claims: &T,
        encoding_key: &EncodingKey,
        kid: &str,
    ) -> Result<String, AppError> {
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(kid.to_string());

        encode(&header, claims, encoding_key)
            .map_err(|e| AppError::Internal(format!("Token generation failed: {e}")))
    }

    pub fn verify<T: DeserializeOwned>(
        &self,
        token: &str,
        decoding_key: &DecodingKey,
    ) -> Result<T, AppError> {
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_required_spec_claims(&["exp", "sub"]);

        let data = decode::<T>(token, decoding_key, &validation).map_err(auth_error)?;
        Ok(data.claims)
    }

    pub fn decode_unverified_header(&self, token: &str) -> Result<jsonwebtoken::Header, AppError> {
        jsonwebtoken::decode_header(token).map_err(auth_error)
    }
}

/// Translates a `jsonwebtoken` failure into an [`AppError::Auth`] whose message
/// is safe to render. An expired token is an ordinary, expected event — it gets
/// its own code so clients refresh instead of reporting a failure — and every
/// other kind collapses to one opaque message: the library's `Debug` string
/// (`ExpiredSignature`, `InvalidSignature`, `Base64(..)`, …) is an internal
/// detail that helps an attacker probe tokens and means nothing to a user. The
/// detail stays in the logs.
fn auth_error(e: jsonwebtoken::errors::Error) -> AppError {
    if matches!(e.kind(), ErrorKind::ExpiredSignature) {
        tracing::debug!("Token rejected: expired");
        return AppError::Auth {
            message: "Session expired".into(),
            code: AuthErrorCode::TokenExpired,
        };
    }

    tracing::debug!(error = ?e.kind(), "Token rejected");
    AppError::Auth {
        message: "Invalid token".into(),
        code: AuthErrorCode::TokenInvalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Serialize, Deserialize)]
    struct TestClaims {
        sub: String,
        exp: u64,
    }

    /// Ed25519 keypair generated for this test only, wrapped the same way
    /// `KeyPairService` wraps the real ones.
    fn keys() -> (EncodingKey, DecodingKey) {
        use ed25519_dalek::SigningKey;

        let signing = SigningKey::generate(&mut rand::rng());

        let mut pkcs8 = Vec::with_capacity(48);
        pkcs8.extend_from_slice(&[
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ]);
        pkcs8.extend_from_slice(&signing.to_bytes());

        let encoding = EncodingKey::from_ed_der(&pkcs8);
        let decoding = DecodingKey::from_ed_der(&signing.verifying_key().to_bytes());
        (encoding, decoding)
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn expired_token_reports_expiry_without_leaking_library_detail() {
        let (encoding, decoding) = keys();
        let svc = JwtService::new();
        let token = svc
            .sign(
                &TestClaims {
                    sub: "user-1".into(),
                    // Well past the library's default 60s leeway.
                    exp: now() - 3600,
                },
                &encoding,
                "kid-1",
            )
            .expect("sign");

        let err = svc.verify::<TestClaims>(&token, &decoding).unwrap_err();
        let AppError::Auth { message, code } = err else {
            panic!("expected an auth error");
        };
        assert_eq!(code, AuthErrorCode::TokenExpired);
        assert_eq!(message, "Session expired");
        assert!(!message.contains("ExpiredSignature"));
    }

    #[test]
    fn tampered_token_is_opaquely_invalid() {
        let (encoding, _) = keys();
        let (_, other_decoding) = keys();
        let svc = JwtService::new();
        let token = svc
            .sign(
                &TestClaims {
                    sub: "user-1".into(),
                    exp: now() + 3600,
                },
                &encoding,
                "kid-1",
            )
            .expect("sign");

        // Verified against an unrelated key: a signature failure, not an expiry.
        let err = svc
            .verify::<TestClaims>(&token, &other_decoding)
            .unwrap_err();
        let AppError::Auth { message, code } = err else {
            panic!("expected an auth error");
        };
        assert_eq!(code, AuthErrorCode::TokenInvalid);
        assert_eq!(message, "Invalid token");
    }

    #[test]
    fn malformed_header_is_opaquely_invalid() {
        let svc = JwtService::new();
        let err = svc.decode_unverified_header("not-a-jwt").unwrap_err();
        let AppError::Auth { message, code } = err else {
            panic!("expected an auth error");
        };
        assert_eq!(code, AuthErrorCode::TokenInvalid);
        assert_eq!(message, "Invalid token");
    }
}
