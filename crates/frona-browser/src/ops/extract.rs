use crate::connection::BrowserConnection;
use crate::error::Error;
use crate::types::{ExtractFormat, Link};
use crate::Result;

impl BrowserConnection {
    pub async fn extract(&self, selector: Option<&str>, format: ExtractFormat) -> Result<String> {
        let page = self.active_page().await?;

        let content = if let Some(sel) = selector {
            let el = page.find_element(sel).await.map_err(Error::Cdp)?;
            match format {
                ExtractFormat::Html => el.outer_html().await.map_err(Error::Cdp)?.unwrap_or_default(),
                ExtractFormat::Text => el.inner_text().await.map_err(Error::Cdp)?.unwrap_or_default(),
            }
        } else {
            let js = match format {
                ExtractFormat::Html => "document.body.innerHTML",
                ExtractFormat::Text => "document.body.innerText",
            };
            page.evaluate(js)
                .await
                .map_err(Error::Cdp)?
                .value()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default()
        };

        Ok(content)
    }

    pub async fn read_links(&self) -> Result<Vec<Link>> {
        const JS: &str = r#"
            JSON.stringify(
                Array.from(document.querySelectorAll('a[href]'))
                    .map(el => ({
                        text: el.innerText || '',
                        href: el.getAttribute('href') || ''
                    }))
                    .filter(link => link.href !== '')
            )
        "#;
        let page = self.active_page().await?;
        let v = page.evaluate(JS).await.map_err(Error::Cdp)?;
        let s = v
            .value()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        let links: Vec<Link> = serde_json::from_str(&s).unwrap_or_default();
        Ok(links)
    }
}
