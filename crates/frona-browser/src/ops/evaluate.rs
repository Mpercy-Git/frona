use crate::connection::BrowserConnection;
use crate::error::Error;
use crate::Result;

impl BrowserConnection {
    pub async fn evaluate(
        &self,
        code: &str,
        _await_promise: bool,
    ) -> Result<serde_json::Value> {
        let page = self.active_page().await?;
        let result = page.evaluate(code).await.map_err(Error::Cdp)?;
        Ok(result.value().cloned().unwrap_or(serde_json::Value::Null))
    }
}
