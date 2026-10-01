use crate::inference::protocol::parameters::WireParameters;
use axum::body::Bytes;
use rig_core::http_client::{
    self, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
};

tokio::task_local! { static PARAMETERS: WireParameters; }

pub async fn scope<F: Future>(parameters: WireParameters, future: F) -> F::Output {
    PARAMETERS.scope(parameters, future).await
}

/// SDK transports call this after serialization and before request signing.
/// `WireClient`'s own `HttpClientExt` impl below goes through `rewrite()`
/// instead (it owns the full `Request`, not just a JSON body); this is the
/// lower-level entry point for a transport that builds its own body and
/// signs directly, e.g. an AWS SigV4-signed adapter - Bedrock isn't a
/// provider in this fork yet, so nothing calls it yet.
#[allow(dead_code)]
pub(crate) fn apply_current(body: &mut serde_json::Value) {
    let _ = PARAMETERS.try_with(|parameters| parameters.apply(body));
}

/// Applies the native JSON merge after Rig has serialized the provider request.
/// Parameters belong to the current async call, never to the cached client.
#[derive(Debug, Clone, Default)]
pub struct WireClient {
    inner: reqwest::Client,
}

impl WireClient {
    pub(crate) fn without_redirects() -> Result<Self, reqwest::Error> {
        Ok(Self {
            inner: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }
}

fn rewrite(request: Request<Bytes>) -> http_client::Result<Request<Bytes>> {
    let Ok(parameters) = PARAMETERS.try_with(Clone::clone) else {
        return Ok(request);
    };
    if request.method() != http_client::Method::POST {
        return Ok(request);
    }
    let (mut parts, bytes) = request.into_parts();
    let mut body: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| http_client::Error::Instance(Box::new(error)))?;
    if !body.is_object() {
        return Err(http_client::Error::Instance(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "provider request must be a JSON object",
        ))));
    }
    parameters.apply(&mut body);
    let bytes =
        serde_json::to_vec(&body).map_err(|error| http_client::Error::Instance(Box::new(error)))?;
    parts.headers.remove("content-length");
    Ok(Request::from_parts(parts, Bytes::from(bytes)))
}

/// reqwest's top-level message ("error sending request for url") hides the
/// actual cause (DNS, connect, TLS, reset, timeout) in the `source()` chain,
/// which is lost once Rig stringifies the error. Log it at the transport layer.
fn log_transport_error(error: &http_client::Error) {
    if let http_client::Error::Instance(inner) = error {
        let mut chain = Vec::new();
        let mut source = inner.source();
        while let Some(cause) = source {
            chain.push(cause.to_string());
            source = cause.source();
        }
        if !chain.is_empty() {
            tracing::warn!(error = %inner, causes = %chain.join(" -> "), "Provider transport error");
        }
    }
}

impl HttpClientExt for WireClient {
    fn send<T, U>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        let request = rewrite(request.map(Into::into));
        let inner = self.inner.clone();
        async move {
            let result = inner.send(request?).await;
            if let Err(error) = &result {
                log_transport_error(error);
            }
            result
        }
    }

    fn send_multipart<U>(
        &self,
        request: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        self.inner.send_multipart(request)
    }

    fn send_streaming<T>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + Send
    where
        T: Into<Bytes> + Send,
    {
        let request = rewrite(request.map(Into::into));
        async move {
            let result = self.inner.send_streaming(request?).await;
            if let Err(error) = &result {
                log_transport_error(error);
            }
            result
        }
    }
}
