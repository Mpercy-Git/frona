use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct TokenQuery {
    pub(super) token: String,
}
