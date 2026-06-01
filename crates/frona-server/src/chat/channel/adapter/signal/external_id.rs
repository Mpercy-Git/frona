use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use uuid::Uuid;

use crate::core::error::AppError;

/// Signal target parsed out of a `Chat.channel_external_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalTarget {
    Dm { aci: Uuid },
    Group { master_key: [u8; 32] },
}

impl SignalTarget {
    pub fn parse(s: &str) -> Result<Self, AppError> {
        if let Some(rest) = s.strip_prefix("dm:") {
            let aci = Uuid::parse_str(rest).map_err(|e| {
                AppError::Validation(format!("invalid Signal DM external_id {s:?}: {e}"))
            })?;
            Ok(SignalTarget::Dm { aci })
        } else if let Some(rest) = s.strip_prefix("group:") {
            let bytes = URL_SAFE_NO_PAD.decode(rest.as_bytes()).map_err(|e| {
                AppError::Validation(format!("invalid Signal group external_id {s:?}: {e}"))
            })?;
            let master_key: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
                AppError::Validation(format!(
                    "Signal group master_key must be 32 bytes, got {} in {s:?}",
                    v.len()
                ))
            })?;
            Ok(SignalTarget::Group { master_key })
        } else {
            Err(AppError::Validation(format!(
                "unrecognised Signal external_id: {s:?}"
            )))
        }
    }
}

pub fn dm(aci: Uuid) -> String {
    format!("dm:{aci}")
}

pub fn group(master_key: &[u8]) -> String {
    format!("group:{}", URL_SAFE_NO_PAD.encode(master_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dm_round_trip() {
        let aci = Uuid::parse_str("3b9b1cfc-2dbe-4d2c-9c8a-0a8f6fffeeaa").unwrap();
        let id = dm(aci);
        match SignalTarget::parse(&id).unwrap() {
            SignalTarget::Dm { aci: a } => assert_eq!(a, aci),
            other => panic!("expected DM, got {other:?}"),
        }
    }

    #[test]
    fn parse_group_round_trip() {
        let mk = [7u8; 32];
        let id = group(&mk);
        match SignalTarget::parse(&id).unwrap() {
            SignalTarget::Group { master_key } => assert_eq!(master_key, mk),
            other => panic!("expected Group, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(SignalTarget::parse("nonsense").is_err());
        assert!(SignalTarget::parse("dm:").is_err());
        assert!(SignalTarget::parse("dm:not-a-uuid").is_err());
        assert!(SignalTarget::parse("group:").is_err());
        assert!(SignalTarget::parse("group:!!!").is_err());
    }

    #[test]
    fn parse_rejects_short_master_key() {
        // 4 bytes base64url-encoded -> too short for a master key
        let id = format!("group:{}", URL_SAFE_NO_PAD.encode([1u8, 2, 3, 4]));
        assert!(SignalTarget::parse(&id).is_err());
    }
}
