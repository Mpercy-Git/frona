use crate::core::Handle;
use crate::core::error::AppError;
use crate::storage::{Attachment, StorageService, dedup_filename};

pub(crate) fn write_attachment_bytes(
    storage: &StorageService,
    handle: &Handle,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
) -> Result<Attachment, AppError> {
    let workspace = storage.user_workspace(handle);
    let inbox = workspace.base_path().join("inbox");
    let safe_name = dedup_filename(inbox.as_path(), filename);
    let rel_path = format!("inbox/{safe_name}");
    workspace.write_bytes(&rel_path, bytes)?;
    Ok(Attachment {
        filename: safe_name,
        content_type: content_type.to_string(),
        size_bytes: bytes.len() as u64,
        owner: format!("user:{handle}"),
        path: rel_path,
        url: None,
    })
}

pub(crate) async fn download_to_attachment(
    client: &reqwest::Client,
    storage: &StorageService,
    handle: &Handle,
    url: &str,
    bearer: Option<&str>,
    filename: &str,
    content_type: &str,
) -> Result<Attachment, AppError> {
    let mut req = client.get(url);
    if let Some(token) = bearer {
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("media download failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Internal(format!(
            "media download {url} returned status {}",
            resp.status(),
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Internal(format!("media download body read failed: {e}")))?;
    write_attachment_bytes(storage, handle, filename, content_type, &bytes)
}
