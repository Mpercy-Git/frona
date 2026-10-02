//! Client for the legacy MCP HTTP+SSE transport (protocol 2024-11-05), which
//! rmcp 1.x no longer ships. The client opens a long-lived `GET <url>` event
//! stream; the first event (`endpoint`) names the URL to `POST` requests to,
//! and every server message arrives as a `message` event on the stream.
//!
//! Exposed as a `(Sink, Stream)` pair so it plugs into rmcp's
//! `IntoTransport` via `SinkStreamTransport`.

use std::collections::HashMap;
use std::time::Duration;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use http::{HeaderMap, HeaderName, HeaderValue};
use rmcp::RoleClient;
use rmcp::service::{RxJsonRpcMessage, TxJsonRpcMessage};
use tokio::sync::oneshot;

use crate::core::error::AppError;

const ENDPOINT_TIMEOUT: Duration = Duration::from_secs(15);

pub type SseSink = UnboundedSender<TxJsonRpcMessage<RoleClient>>;
pub type SseStream = UnboundedReceiver<RxJsonRpcMessage<RoleClient>>;

pub async fn connect(
    url: &str,
    headers: HashMap<HeaderName, HeaderValue>,
) -> Result<(SseSink, SseStream), AppError> {
    let base = reqwest::Url::parse(url)
        .map_err(|e| AppError::Validation(format!("invalid MCP SSE url '{url}': {e}")))?;

    let mut header_map = HeaderMap::new();
    for (k, v) in headers {
        header_map.insert(k, v);
    }
    let client = reqwest::Client::builder()
        .default_headers(header_map)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::Tool(format!("building MCP SSE client: {e}")))?;

    let resp = client
        .get(base.clone())
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .send()
        .await
        .map_err(|e| AppError::Tool(format!("connecting to MCP SSE endpoint {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Tool(format!(
            "MCP SSE endpoint {url} returned {}",
            resp.status()
        )));
    }

    let (in_tx, in_rx) = unbounded::<RxJsonRpcMessage<RoleClient>>();
    let (endpoint_tx, endpoint_rx) = oneshot::channel::<String>();

    let reader = tokio::spawn(read_events(resp, in_tx, endpoint_tx));

    let endpoint = match tokio::time::timeout(ENDPOINT_TIMEOUT, endpoint_rx).await {
        Ok(Ok(e)) => e,
        _ => {
            reader.abort();
            return Err(AppError::Tool(format!(
                "MCP SSE endpoint {url} did not send an `endpoint` event"
            )));
        }
    };
    let post_url = base.join(&endpoint).map_err(|e| {
        AppError::Tool(format!(
            "invalid MCP SSE message endpoint '{endpoint}': {e}"
        ))
    })?;
    if post_url.origin() != base.origin() {
        reader.abort();
        return Err(AppError::Tool(format!(
            "MCP SSE server advertised a cross-origin message endpoint: {post_url}"
        )));
    }

    let (out_tx, mut out_rx) = unbounded::<TxJsonRpcMessage<RoleClient>>();
    tokio::spawn(async move {
        use futures::StreamExt;
        while let Some(msg) = out_rx.next().await {
            match client.post(post_url.clone()).json(&msg).send().await {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => tracing::warn!(status = %r.status(), "MCP SSE POST rejected"),
                Err(e) => tracing::warn!(error = %e, "MCP SSE POST failed"),
            }
        }
        reader.abort();
    });

    Ok((out_tx, in_rx))
}

async fn read_events(
    mut resp: reqwest::Response,
    in_tx: UnboundedSender<RxJsonRpcMessage<RoleClient>>,
    endpoint_tx: oneshot::Sender<String>,
) {
    let mut endpoint_tx = Some(endpoint_tx);
    let mut buf: Vec<u8> = Vec::new();
    let mut event = String::new();
    let mut data = String::new();

    loop {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(error = %e, "MCP SSE stream error");
                return;
            }
        };
        buf.extend_from_slice(&chunk);

        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&raw);
            let line = line.trim_end_matches(['\n', '\r']);

            if line.is_empty() {
                if !data.is_empty() {
                    let name = if event.is_empty() {
                        "message"
                    } else {
                        event.as_str()
                    };
                    match name {
                        "endpoint" => {
                            if let Some(tx) = endpoint_tx.take() {
                                let _ = tx.send(data.clone());
                            }
                        }
                        "message" => match serde_json::from_str(&data) {
                            Ok(msg) => {
                                if in_tx.unbounded_send(msg).is_err() {
                                    return;
                                }
                            }
                            Err(e) => tracing::warn!(error = %e, "undecodable MCP SSE message"),
                        },
                        _ => {}
                    }
                }
                event.clear();
                data.clear();
            } else if let Some(v) = line.strip_prefix("event:") {
                event = v.trim_start().to_string();
            } else if let Some(v) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(v.strip_prefix(' ').unwrap_or(v));
            }
        }
    }
}
