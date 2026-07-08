use frona_api_types::mcp::{
    BridgeCallRequest, BridgeCallResponse, BridgeServerDetail, BridgeServerInfo,
};

pub struct BridgeClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl BridgeClient {
    pub fn new(base_url: String, token: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
            token,
        }
    }

    pub async fn list_servers(&self) -> Result<Vec<BridgeServerInfo>, Error> {
        let url = format!("{}/api/mcp/bridge/servers", self.base_url);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await?;
        check_status(resp).await?.json().await.map_err(Error::Http)
    }

    pub async fn server_tools(&self, slug: &str) -> Result<BridgeServerDetail, Error> {
        let url = format!("{}/api/mcp/bridge/servers/{slug}", self.base_url);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await?;
        check_status(resp).await?.json().await.map_err(Error::Http)
    }

    pub async fn call_tool(
        &self,
        slug: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<BridgeCallResponse, Error> {
        let url = format!("{}/api/mcp/bridge/{slug}/call/{tool_name}", self.base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .json(&BridgeCallRequest { arguments })
            .send()
            .await?;
        check_status(resp).await?.json().await.map_err(Error::Http)
    }
}

async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response, Error> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    Err(Error::Api {
        status: status.as_u16(),
        body,
    })
}

#[derive(Debug)]
pub enum Error {
    Http(reqwest::Error),
    Api { status: u16, body: String },
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "HTTP error: {e}"),
            Self::Api { status, body } => write!(f, "API error ({status}): {body}"),
        }
    }
}
