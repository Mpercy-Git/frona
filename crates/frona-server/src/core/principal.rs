use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb::types")]
pub struct Principal {
    pub kind: PrincipalKind,
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, SurrealValue)]
#[serde(rename_all = "snake_case")]
#[surreal(crate = "surrealdb::types", rename_all = "snake_case")]
pub enum PrincipalKind {
    User,
    Agent,
    McpServer,
    App,
    Channel,
}

impl Principal {
    pub fn user(id: impl Into<String>) -> Self {
        Self {
            kind: PrincipalKind::User,
            id: id.into(),
        }
    }

    pub fn agent(id: impl Into<String>) -> Self {
        Self {
            kind: PrincipalKind::Agent,
            id: id.into(),
        }
    }

    pub fn mcp_server(id: impl Into<String>) -> Self {
        Self {
            kind: PrincipalKind::McpServer,
            id: id.into(),
        }
    }

    pub fn app(id: impl Into<String>) -> Self {
        Self {
            kind: PrincipalKind::App,
            id: id.into(),
        }
    }

    pub fn channel(id: impl Into<String>) -> Self {
        Self {
            kind: PrincipalKind::Channel,
            id: id.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_set_kind_and_id() {
        assert_eq!(
            Principal::user("u1"),
            Principal {
                kind: PrincipalKind::User,
                id: "u1".into()
            }
        );
        assert_eq!(
            Principal::agent("a1"),
            Principal {
                kind: PrincipalKind::Agent,
                id: "a1".into()
            }
        );
        assert_eq!(
            Principal::mcp_server("m1"),
            Principal {
                kind: PrincipalKind::McpServer,
                id: "m1".into()
            }
        );
        assert_eq!(
            Principal::app("p1"),
            Principal {
                kind: PrincipalKind::App,
                id: "p1".into()
            }
        );
        assert_eq!(
            Principal::channel("c1"),
            Principal {
                kind: PrincipalKind::Channel,
                id: "c1".into()
            }
        );
    }

    #[test]
    fn serde_snake_case_round_trip() {
        let cases = [
            (Principal::user("u"), "user"),
            (Principal::agent("a"), "agent"),
            (Principal::mcp_server("m"), "mcp_server"),
            (Principal::app("p"), "app"),
            (Principal::channel("c"), "channel"),
        ];
        for (principal, expected_kind) in cases {
            let json = serde_json::to_value(&principal).unwrap();
            assert_eq!(json["kind"], expected_kind);
            let round: Principal = serde_json::from_value(json).unwrap();
            assert_eq!(round, principal);
        }
    }
}
