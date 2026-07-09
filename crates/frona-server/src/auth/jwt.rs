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

        let data = decode::<T>(token, decoding_key, &validation)
            .map_err(|e| AppError::Auth { message: format!("Invalid token: {e}"), code: AuthErrorCode::TokenInvalid })?;
        Ok(data.claims)
    }

    pub fn decode_unverified_header(
        &self,
        token: &str,
    ) -> Result<jsonwebtoken::Header, AppError> {
        jsonwebtoken::decode_header(token)
            .map_err(|e| AppError::Auth { message: format!("Invalid token header: {e}"), code: AuthErrorCode::TokenInvalid })
    }
}
