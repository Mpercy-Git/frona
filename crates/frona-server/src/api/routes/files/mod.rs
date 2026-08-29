mod browse;
mod models;
mod operations;
mod range;
mod upload;

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, FromRequestParts, Query};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

use range::RangeSpec;

use crate::storage::{VirtualPath, detect_content_type};

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;
use crate::core::error::{AppError, AuthErrorCode};
use crate::core::state::AppState;

use models::{FileAuth, PresignQuery};

const MAX_FILE_SIZE: usize = 50 * 1024 * 1024; // 50MB
// Multipart requests carry boundary markers plus the optional `path` field on
// top of the file bytes. Give the body limit headroom above MAX_FILE_SIZE so a
// file right at the cap isn't rejected with a bare 413 before the handler can
// return its friendly "File too large" message. Anything past this limit is
// caught by the handler and reported as a clear "File too large" error rather
// than a cryptic multipart parse failure (see upload::map_multipart_err).
const MAX_UPLOAD_BODY_SIZE: usize = MAX_FILE_SIZE + 1024 * 1024; // +1MB overhead

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/files",
            post(upload::upload_file)
                .layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY_SIZE)),
        )
        .route("/api/files/presign", post(upload::presign_file))
        .route(
            "/api/files/user/{handle}/{*filename}",
            get(browse::download_user_file).delete(browse::delete_user_file),
        )
        .route(
            "/api/files/agent/{agent_id}/{*filepath}",
            get(browse::download_agent_file),
        )
        .route("/api/files/browse/user", get(browse::list_user_files))
        .route(
            "/api/files/browse/user/{*dirpath}",
            get(browse::list_user_files),
        )
        .route(
            "/api/files/browse/agent/{agent_id}",
            get(browse::list_agent_files_root),
        )
        .route(
            "/api/files/browse/agent/{agent_id}/{*dirpath}",
            get(browse::list_agent_files_subdir),
        )
        .route("/api/files/search", get(browse::search_files))
        .route("/api/files/rename", post(operations::rename_user_file))
        .route("/api/files/copy", post(operations::copy_files))
        .route("/api/files/move", post(operations::move_files))
        .route("/api/files/mkdir", post(operations::create_user_folder))
}

impl FromRequestParts<AppState> for FileAuth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Ok(auth) = AuthUser::from_request_parts(parts, state).await {
            return Ok(FileAuth::User(auth));
        }

        let query: Query<PresignQuery> = Query::try_from_uri(&parts.uri).map_err(|_| {
            ApiError(AppError::Auth {
                message: "Missing authorization".into(),
                code: AuthErrorCode::InvalidCredentials,
            })
        })?;

        let token = query.presign.as_deref().ok_or_else(|| {
            ApiError(AppError::Auth {
                message: "Missing authorization".into(),
                code: AuthErrorCode::InvalidCredentials,
            })
        })?;

        let claims = state.presign_service.verify(token).await?;

        Ok(FileAuth::Presigned {
            sub: claims.sub,
            owner: claims.owner,
            path: claims.path,
        })
    }
}

pub(super) async fn serve_file(
    vpath: &VirtualPath,
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let resolved = state.storage_service.resolve_virtual_path(vpath)?;
    serve_path(&resolved, headers).await
}

/// Caller is responsible for path traversal / ownership checks.
///
/// Honours a single `Range` request so browsers can seek in audio and video
/// served from here; `headers` is the request's header map.
pub(super) async fn serve_path(
    path: &std::path::Path,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let metadata = fs::metadata(path)
        .await
        .map_err(|_| ApiError(AppError::NotFound("File not found".into())))?;
    if !metadata.is_file() {
        return Err(ApiError(AppError::NotFound("File not found".into())));
    }
    let file_len = metadata.len();

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download");
    let content_type = detect_content_type(filename);

    // SVG and other script-executable types must be served as attachments to
    // prevent stored XSS via <script> tags executing at the app origin.
    let disposition_type = match content_type {
        "image/svg+xml" | "text/html" | "text/javascript" | "application/javascript" => {
            "attachment"
        }
        _ => "inline",
    };

    // `Accept-Ranges` on every response, including the full-file one: it's how
    // a media element learns it may seek at all.
    let base = || {
        Response::builder()
            .header(header::CONTENT_TYPE, content_type)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(
                header::CONTENT_DISPOSITION,
                format!("{disposition_type}; filename=\"{filename}\""),
            )
    };

    let requested = range::parse(
        headers.get(header::RANGE).and_then(|v| v.to_str().ok()),
        file_len,
    );

    if requested == RangeSpec::Unsatisfiable {
        return Ok(base()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{file_len}"))
            .body(Body::empty())
            .unwrap());
    }

    let mut file = fs::File::open(path)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;

    let response = match requested {
        RangeSpec::Partial(range) => {
            file.seek(std::io::SeekFrom::Start(range.start))
                .await
                .map_err(|e| ApiError(AppError::Internal(e.to_string())))?;
            let body = Body::from_stream(ReaderStream::new(file.take(range.len())));
            base()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {}-{}/{file_len}", range.start, range.end),
                )
                .header(header::CONTENT_LENGTH, range.len())
                .body(body)
        }
        // `Content-Length` matters even without a range: media elements use it
        // to compute duration and to decide whether seeking is worth trying.
        _ => base()
            .header(header::CONTENT_LENGTH, file_len)
            .body(Body::from_stream(ReaderStream::new(file))),
    };

    Ok(response.unwrap())
}
