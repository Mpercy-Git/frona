use std::time::{Duration, Instant};

use crate::connection::BrowserConnection;
use crate::error::Error;
use crate::Result;

impl BrowserConnection {
    pub async fn wait_for_selector(&self, selector: &str, timeout: Duration) -> Result<()> {
        let page = self.active_page().await?;
        let start = Instant::now();
        let probe_js = format!(
            "!!document.querySelector({})",
            serde_json::to_string(selector)?
        );

        loop {
            let v = page.evaluate(probe_js.clone()).await.map_err(Error::Cdp)?;
            if v.value().and_then(|x| x.as_bool()).unwrap_or(false) {
                return Ok(());
            }
            if start.elapsed() >= timeout {
                return Err(Error::Timeout(timeout));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}
