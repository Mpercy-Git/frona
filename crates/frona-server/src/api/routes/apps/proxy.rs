use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use tower::ServiceExt as _;
use tower_http::services::ServeDir;

use crate::api::cookie::{extract_app_session_from_cookie_header, make_app_session_cookie};
use crate::app::models::AppStatus;
use crate::core::state::AppState;

/// Path prefix every app is served under, on whichever origin serves it.
pub(crate) const APPS_PREFIX: &str = "/apps/";

/// Where the apps origin hands a signed-in user's code to be exchanged for an
/// `app_session` cookie.
pub(crate) const APP_CALLBACK_PATH: &str = "/api/auth/apps/callback";

fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// The `redirect` query parameter, if it is a same-origin path. Anything that
/// could be read as another origin (`//host`, backslashes) or that is not
/// plain visible ASCII is refused.
fn requested_redirect(uri: &Uri) -> Option<String> {
    let value = url::form_urlencoded::parse(uri.query()?.as_bytes())
        .find(|(key, _)| key == "redirect")?
        .1
        .into_owned();
    let plain = value.bytes().all(|b| (0x21..=0x7e).contains(&b));
    (value.starts_with('/') && !value.starts_with("//") && !value.contains('\\') && plain)
        .then_some(value)
}

fn is_secure(url: Option<&str>) -> bool {
    url.is_some_and(|u| u.starts_with("https"))
}

/// Main-origin gate. Turns the user's refresh cookie into an app credential:
/// directly as an `app_session` cookie when apps share the main origin, or as
/// a one-minute code handed to the apps origin when `server.apps_url` is set.
pub(crate) async fn auth_gate(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let redirect_url = requested_redirect(&uri).unwrap_or_else(|| "/".to_string());
    let apps_url = state.config.server.public_apps_url();

    let cookie_header = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();

    let login = || {
        let login_url = build_login_redirect(&state, &redirect_url);
        Redirect::temporary(&login_url).into_response()
    };

    let Some(refresh_token) =
        crate::api::cookie::extract_refresh_token_from_cookie_header(cookie_header)
    else {
        return login();
    };

    let Ok(claims) = state
        .token_service
        .validate_refresh(&state.keypair_service, refresh_token)
        .await
    else {
        return login();
    };

    let user = match state.user_service.find_by_id(&claims.sub).await {
        Ok(Some(u)) if u.deactivated_at.is_none() => u,
        _ => return login(),
    };

    if let Some(apps_url) = apps_url {
        if !redirect_url.starts_with(APPS_PREFIX) {
            return StatusCode::BAD_REQUEST.into_response();
        }
        let Ok(code) = state
            .token_service
            .create_app_gate_code(&state.keypair_service, &user)
            .await
        else {
            return login();
        };
        let location = format!(
            "{apps_url}{APP_CALLBACK_PATH}?code={}&redirect={}",
            encode(&code),
            encode(&redirect_url)
        );
        return no_store_redirect(&location, None);
    }

    let Ok(app_session_jwt) = state
        .token_service
        .create_app_session(&state.keypair_service, &user)
        .await
    else {
        return login();
    };

    let cookie = make_app_session_cookie(
        &app_session_jwt,
        state.config.auth.access_token_expiry_secs,
        is_secure(state.config.server.base_url.as_deref()),
    );
    no_store_redirect(&redirect_url, Some(cookie))
}

/// Apps-origin half of the handoff: exchange the gate's code for an
/// `app_session` cookie scoped to this origin, then continue to the app.
pub(crate) async fn auth_gate_callback(State(state): State<AppState>, uri: Uri) -> Response {
    let Some(apps_url) = state.config.server.public_apps_url() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let query = uri.query().unwrap_or_default();
    let code = url::form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == "code")
        .map(|(_, v)| v.into_owned());
    let redirect = requested_redirect(&uri).filter(|r| r.starts_with(APPS_PREFIX));
    let (Some(code), Some(redirect)) = (code, redirect) else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    let Ok(claims) = state
        .token_service
        .validate_app_gate_code(&state.keypair_service, &code)
        .await
    else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let user = match state.user_service.find_by_id(&claims.sub).await {
        Ok(Some(u)) if u.deactivated_at.is_none() => u,
        _ => return StatusCode::FORBIDDEN.into_response(),
    };
    let Ok(session) = state
        .token_service
        .create_app_session(&state.keypair_service, &user)
        .await
    else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    let cookie = make_app_session_cookie(
        &session,
        state.config.auth.access_token_expiry_secs,
        is_secure(Some(&apps_url)),
    );
    no_store_redirect(&redirect, Some(cookie))
}

/// 307 that browsers and proxies must not cache and that never leaks the
/// current URL (it can carry a gate code) as a `Referer`.
fn no_store_redirect(location: &str, cookie: Option<HeaderValue>) -> Response {
    let mut builder = Response::builder()
        .status(StatusCode::TEMPORARY_REDIRECT)
        .header("location", location)
        .header("cache-control", "no-store")
        .header("referrer-policy", "no-referrer");
    if let Some(cookie) = cookie {
        builder = builder.header("set-cookie", cookie);
    }
    builder
        .body(Body::empty())
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn build_login_redirect(state: &AppState, app_redirect: &str) -> String {
    let frontend_url = state.config.server.public_frontend_url();
    let base_url = state.config.server.public_base_url();
    let encoded_redirect = encode(app_redirect);
    if frontend_url.is_empty() {
        return format!("/login?redirect={encoded_redirect}");
    }
    let gate_url = format!("{base_url}/api/auth/apps?redirect={encoded_redirect}");
    format!("{frontend_url}/login?redirect={}", encode(&gate_url))
}

pub(crate) async fn proxy_app_root(
    State(state): State<AppState>,
    Path(handle): Path<String>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    proxy_app_inner(state, handle, String::new(), headers, request).await
}

pub(crate) async fn proxy_app_path(
    State(state): State<AppState>,
    Path((handle, sub_path)): Path<(String, String)>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    proxy_app_inner(state, handle, sub_path, headers, request).await
}

async fn proxy_app_inner(
    state: AppState,
    handle: String,
    sub_path: String,
    headers: HeaderMap,
    request: Request,
) -> Response {
    tracing::debug!(handle = %handle, sub_path = %sub_path, "Proxy: incoming request");

    let user_id = match authenticate_proxy_request(&state, &headers).await {
        Some(uid) => uid,
        None => {
            let original_uri = request.uri().to_string();
            // On the apps origin the gate lives on the main origin, so the link
            // must be absolute; with shared origins a relative one will do.
            let gate_base = if state.config.server.public_apps_url().is_some() {
                state.config.server.public_base_url()
            } else {
                String::new()
            };
            let gate_url = format!(
                "{gate_base}/api/auth/apps?redirect={}",
                encode(&original_uri)
            );
            return Redirect::temporary(&gate_url).into_response();
        }
    };

    let parsed_handle = match crate::core::Handle::try_new(&handle) {
        Ok(h) => h,
        Err(_) => {
            tracing::warn!(handle = %handle, "Proxy: malformed handle");
            return StatusCode::NOT_FOUND.into_response();
        }
    };

    let app = match state
        .app_service
        .find_by_user_handle(&user_id, &parsed_handle)
        .await
    {
        Ok(Some(app)) => {
            tracing::debug!(handle = %handle, app_id = %app.id, status = ?app.status, kind = %app.kind, "Proxy: app found");
            app
        }
        Ok(None) => {
            tracing::warn!(handle = %handle, "Proxy: app not found for user");
            return StatusCode::NOT_FOUND.into_response();
        }
        Err(e) => {
            tracing::warn!(handle = %handle, error = %e, "Proxy: app lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    state.app_service.manager().record_access(&app.id).await;

    let app_handle = app.handle.to_string();
    match app.kind.as_str() {
        "static" => serve_static(&state, &app, &sub_path, request).await,
        _ => {
            if app.status == AppStatus::Hibernated {
                return handle_hibernated_app(&state, &app, &sub_path, request).await;
            }

            let port = match app.port {
                Some(p) => p,
                None => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
            };

            if !matches!(app.status, AppStatus::Running) {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }

            forward_to_port(port, &sub_path, &app_handle, request).await
        }
    }
}

async fn authenticate_proxy_request(state: &AppState, headers: &HeaderMap) -> Option<String> {
    if let Some(auth_header) = headers.get("authorization").and_then(|v| v.to_str().ok())
        && let Some(token) = auth_header.strip_prefix("Bearer ")
        && let Ok(claims) = state
            .token_service
            .validate(&state.keypair_service, token)
            .await
    {
        return Some(claims.sub);
    }

    let cookie_header = headers.get("cookie").and_then(|v| v.to_str().ok())?;
    let token = extract_app_session_from_cookie_header(cookie_header)?;
    state
        .token_service
        .validate_app_session(&state.keypair_service, token)
        .await
        .ok()
        .map(|c| c.sub)
}

async fn serve_static(
    state: &AppState,
    app: &crate::app::models::App,
    sub_path: &str,
    request: Request,
) -> Response {
    let static_dir = app.static_dir.as_deref().unwrap_or("dist");
    let Ok(agent) = state.agent_service.get(&app.user_id, &app.agent_id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(user) = state
        .user_service
        .find_by_id(&app.user_id)
        .await
        .ok()
        .flatten()
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let workspace_path = state
        .storage_service
        .agent_workspace_path(&user.handle, &agent.handle);
    let serve_path = workspace_path.join(static_dir);

    if !serve_path.exists() {
        tracing::warn!(app_id = %app.id, path = %serve_path.display(), "Proxy: static dir not found");
        return StatusCode::NOT_FOUND.into_response();
    }

    let path = if sub_path.is_empty() { "/" } else { sub_path };
    let (mut parts, body) = request.into_parts();
    parts.uri = path.parse().unwrap_or(Uri::from_static("/"));
    let req = Request::from_parts(parts, body);

    let service = ServeDir::new(&serve_path).append_index_html_on_directories(true);

    match service.oneshot(req).await {
        Ok(resp) => {
            let status = resp.status();
            if status == StatusCode::NOT_FOUND
                && path == "/"
                && let Some(fallback) = find_html_fallback(&serve_path)
            {
                return fallback;
            }
            if status == StatusCode::NOT_FOUND {
                tracing::warn!(app_id = %app.id, serve_path = %serve_path.display(), sub_path = %path, "Proxy: static file not found");
            }
            resp.into_response()
        }
        Err(e) => {
            tracing::error!(app_id = %app.id, error = %e, "Proxy: static serve error");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn find_html_fallback(serve_path: &std::path::Path) -> Option<Response> {
    let entries = std::fs::read_dir(serve_path).ok()?;
    let html_files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("html"))
        })
        .collect();

    if html_files.len() == 1 {
        let content = std::fs::read(html_files[0].path()).ok()?;
        return Some(
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/html; charset=utf-8")
                .body(Body::from(content))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        );
    }

    None
}

async fn handle_hibernated_app(
    state: &AppState,
    app: &crate::app::models::App,
    sub_path: &str,
    original_request: Request,
) -> Response {
    let manifest: crate::app::models::AppManifest =
        match serde_json::from_value(app.manifest.clone()) {
            Ok(m) => m,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };

    let command = match &app.command {
        Some(c) => c.clone(),
        None => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };

    let result = state
        .app_service
        .manager()
        .start_app(
            &app.id,
            &app.agent_id,
            &app.user_id,
            &command,
            &manifest,
            Vec::new(),
        )
        .await;

    match result {
        Ok((port, pid)) => {
            let _ = state
                .app_service
                .update_status(&app.id, AppStatus::Running, Some(port), Some(pid))
                .await;

            let health = manifest
                .health_check
                .as_ref()
                .map(|h| {
                    (
                        h.path.clone(),
                        h.effective_initial_delay(),
                        h.effective_timeout(),
                    )
                })
                .unwrap_or_else(|| ("/".to_string(), 5, 2));

            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(health.1);

            let hc = crate::app::models::HealthCheck {
                path: health.0,
                interval_secs: Some(1),
                timeout_secs: Some(health.2),
                initial_delay_secs: Some(0),
                failure_threshold: None,
            };

            loop {
                if tokio::time::Instant::now() >= deadline {
                    return StatusCode::SERVICE_UNAVAILABLE.into_response();
                }
                if state.app_service.manager().health_check(port, &hc).await {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }

            forward_to_port(port, sub_path, app.handle.as_str(), original_request).await
        }
        Err(_) => {
            let _ = state
                .app_service
                .update_status(&app.id, AppStatus::Failed, None, None)
                .await;
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

fn rewrite_location(value: &str, app_prefix: &str) -> Option<String> {
    if let Some(path) = value.strip_prefix("http://127.0.0.1") {
        let path = path.find('/').map(|i| &path[i..]).unwrap_or("/");
        return Some(format!(
            "{app_prefix}{}",
            path.strip_prefix('/').unwrap_or(path)
        ));
    }

    if value.starts_with('/') {
        return Some(format!(
            "{app_prefix}{}",
            value.strip_prefix('/').unwrap_or(value)
        ));
    }

    None
}

async fn forward_to_port(
    port: u16,
    path: &str,
    app_handle: &str,
    original_request: Request,
) -> Response {
    let query = original_request.uri().query();
    let uri = match (path.is_empty(), query) {
        (true, None) => format!("http://127.0.0.1:{port}/"),
        (true, Some(q)) => format!("http://127.0.0.1:{port}/?{q}"),
        (false, None) => format!("http://127.0.0.1:{port}/{path}"),
        (false, Some(q)) => format!("http://127.0.0.1:{port}/{path}?{q}"),
    };

    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    let (parts, body) = original_request.into_parts();

    let reqwest_method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())
        .unwrap_or(reqwest::Method::GET);

    let mut upstream_req = client.request(reqwest_method, &uri);

    for (key, value) in &parts.headers {
        match key.as_str() {
            "host" | "connection" | "transfer-encoding" | "authorization" | "cookie" => continue,
            _ => {
                if let Ok(name) = reqwest::header::HeaderName::from_bytes(key.as_ref())
                    && let Ok(val) = reqwest::header::HeaderValue::from_bytes(value.as_bytes())
                {
                    upstream_req = upstream_req.header(name, val);
                }
            }
        }
    }

    let body_bytes = match axum::body::to_bytes(body, 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if !body_bytes.is_empty() {
        upstream_req = upstream_req.body(body_bytes);
    }

    let upstream_resp = match upstream_req.send().await {
        Ok(r) => r,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    let status =
        StatusCode::from_u16(upstream_resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

    let app_prefix = format!("/apps/{app_handle}/");
    let mut builder = Response::builder().status(status);
    for (key, value) in upstream_resp.headers() {
        if let Ok(name) = axum::http::header::HeaderName::from_bytes(key.as_ref())
            && let Ok(val) = HeaderValue::from_bytes(value.as_bytes())
        {
            // An app must not plant cookies on Frona's origin: the proxy
            // already withholds the browser's cookies from it, so none of its
            // own could ever come back.
            if name == "set-cookie" || name == "set-cookie2" {
                continue;
            }
            if (name == "location" || name == "content-location")
                && let Ok(loc_str) = value.to_str()
                && let Some(rewritten) = rewrite_location(loc_str, &app_prefix)
                && let Ok(new_val) = HeaderValue::from_str(&rewritten)
            {
                builder = builder.header(name, new_val);
                continue;
            }
            builder = builder.header(name, val);
        }
    }

    match upstream_resp.bytes().await {
        Ok(body) => builder
            .body(Body::from(body))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_location_absolute_path() {
        assert_eq!(
            rewrite_location("/login", "/apps/x/"),
            Some("/apps/x/login".to_string())
        );
    }

    #[test]
    fn rewrite_location_root() {
        assert_eq!(
            rewrite_location("/", "/apps/x/"),
            Some("/apps/x/".to_string())
        );
    }

    #[test]
    fn rewrite_location_relative_path() {
        assert_eq!(rewrite_location("next-page", "/apps/x/"), None);
    }

    #[test]
    fn rewrite_location_external_url() {
        assert_eq!(
            rewrite_location("https://example.com/foo", "/apps/x/"),
            None
        );
    }

    #[test]
    fn rewrite_location_localhost_url() {
        assert_eq!(
            rewrite_location("http://127.0.0.1:3456/dashboard", "/apps/x/"),
            Some("/apps/x/dashboard".to_string())
        );
    }
}
