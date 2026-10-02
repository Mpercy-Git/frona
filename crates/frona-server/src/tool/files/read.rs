use std::sync::Arc;

use serde_json::Value;

use crate::agent::prompt::PromptLoader;
use crate::core::error::AppError;
use crate::storage::service::StorageService;
use frona_derive::agent_tool;

use super::super::sandbox::SandboxManager;
use super::super::{ImageData, InferenceContext, ToolOutput, str_list_arg};

const MAX_LINES: usize = 2000;
const MAX_BYTES: usize = 50 * 1024;
const IMAGE_MAX_DIM: u32 = 2000;
/// Pages returned per `render` call. Each page is a full image in the model's
/// context, so the agent pages through a long PDF rather than taking it all.
const PDF_RENDER_MAX_PAGES: u32 = 5;
/// Longest side, in pixels, of a rendered PDF page.
const PDF_RENDER_SCALE_TO: u32 = 1600;
const PDF_RENDER_TIMEOUT_SECS: u64 = 60;

pub struct ReadTool {
    pub storage: StorageService,
    pub sandbox_manager: Arc<SandboxManager>,
    pub prompts: PromptLoader,
}

impl ReadTool {
    pub fn new(
        storage: StorageService,
        sandbox_manager: Arc<SandboxManager>,
        prompts: PromptLoader,
    ) -> Self {
        Self {
            storage,
            sandbox_manager,
            prompts,
        }
    }
}

#[agent_tool]
impl ReadTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let paths = str_list_arg(&arguments, "path", "paths");
        let [first, rest @ ..] = paths.as_slice() else {
            return Err(AppError::Validation("Missing 'path' parameter".into()));
        };
        let offset = arguments
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let limit = arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        let render = arguments
            .get("render")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            .then(|| {
                arguments
                    .get("pages")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            });

        let sandbox = self.sandbox_manager.for_tool(ctx).await?;
        if rest.is_empty() {
            return self
                .read_one(first, offset, limit, render.as_ref(), ctx, &sandbox)
                .await;
        }

        // A batch is what the agent would otherwise spend one tool turn per file
        // on, so one bad path must not sink the rest: each file gets its own
        // section, and the call only fails when every file did. `offset`/`limit`
        // address a single file's lines, so they are not applied here - a
        // paginated read is a single-path read.
        let mut body = String::new();
        let mut images = Vec::new();
        let mut any_ok = false;
        for path in &paths {
            let mut out = self.read_one(path, None, None, None, ctx, &sandbox).await?;
            any_ok |= out.is_success();
            let text = out.text_content().to_string();
            images.extend(out.take_images());
            body.push_str(&format!("===== {path} =====\n{text}\n\n"));
        }
        let body = body.trim_end().to_string();
        if any_ok {
            Ok(ToolOutput::mixed(body, images))
        } else {
            Ok(ToolOutput::error(body))
        }
    }
}

impl ReadTool {
    async fn read_one(
        &self,
        path_arg: &str,
        offset: Option<usize>,
        limit: Option<usize>,
        render: Option<&Option<String>>,
        ctx: &InferenceContext,
        sandbox: &crate::tool::sandbox::Sandbox,
    ) -> Result<ToolOutput, AppError> {
        let resolved = super::resolve_path(path_arg, ctx, &self.storage)?;
        if !sandbox.is_readable(&resolved) {
            return Ok(ToolOutput::error(format!(
                "Read denied by sandbox policy: {} (resolved: {})",
                path_arg,
                resolved.display(),
            )));
        }
        if !tokio::fs::try_exists(&resolved).await.unwrap_or(false) {
            return Ok(ToolOutput::error(format!("file not found: {}", path_arg)));
        }

        if tokio::fs::metadata(&resolved)
            .await
            .is_ok_and(|m| m.is_dir())
        {
            return Ok(ToolOutput::error(directory_message(&resolved, path_arg).await));
        }

        let bytes = match tokio::fs::read(&resolved).await {
            Ok(b) => b,
            Err(e) => {
                return Ok(ToolOutput::error(format!(
                    "could not read {path_arg}: {e}"
                )));
            }
        };
        let size = bytes.len();

        let mime = infer::get(&bytes).map(|t| t.mime_type().to_string());
        if let Some(ref m) = mime
            && is_supported_image(m)
        {
            return read_image(&bytes, m, path_arg);
        }
        if let Some(ref m) = mime
            && m == "application/pdf"
        {
            if let Some(pages) = render {
                return Ok(render_pdf(sandbox, &resolved, path_arg, pages.as_deref()).await);
            }
            return Ok(read_pdf(&bytes, path_arg, offset, limit));
        }

        match std::str::from_utf8(&bytes) {
            Ok(text) => Ok(read_text(text, offset, limit)),
            Err(_) => Ok(ToolOutput::error(format!(
                "Read got a binary file ({}, {} bytes) at {}. Use produce_file to surface it to the user, or pass through CliTool if you need raw bytes.",
                mime.as_deref().unwrap_or("application/octet-stream"),
                size,
                path_arg,
            ))),
        }
    }
}

/// Explain that `path_arg` is a directory and list its entries, so the agent
/// can pick a file instead of retrying the same path.
async fn directory_message(resolved: &std::path::Path, path_arg: &str) -> String {
    const MAX_ENTRIES: usize = 200;
    let mut names = Vec::new();
    if let Ok(mut rd) = tokio::fs::read_dir(resolved).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let is_dir = entry.file_type().await.is_ok_and(|t| t.is_dir());
            let name = entry.file_name().to_string_lossy().into_owned();
            names.push(if is_dir { format!("{name}/") } else { name });
        }
    }
    names.sort();
    let total = names.len();
    names.truncate(MAX_ENTRIES);
    let mut msg = format!("{path_arg} is a directory, not a file. Read a file inside it");
    if names.is_empty() {
        msg.push_str(" (the directory is empty).");
    } else {
        msg.push_str(":\n");
        msg.push_str(&names.join("\n"));
        if total > MAX_ENTRIES {
            msg.push_str(&format!("\n... and {} more", total - MAX_ENTRIES));
        }
    }
    msg
}

pub(crate) fn is_supported_image(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

/// Decode an image and shrink it to fit within `IMAGE_MAX_DIM` on each side,
/// ready to hand to a model. Images already within bounds are passed through
/// untouched. The error is a message for the agent, naming `path_arg`.
pub(crate) fn prepare_image(bytes: &[u8], mime: &str, path_arg: &str) -> Result<ImageData, String> {
    let img = image::load_from_memory(bytes)
        .map_err(|_| format!("could not decode image at {} ({})", path_arg, mime))?;
    let (w, h) = (img.width(), img.height());
    if w <= IMAGE_MAX_DIM && h <= IMAGE_MAX_DIM {
        return Ok(ImageData {
            bytes: bytes.to_vec(),
            media_type: mime.to_string(),
        });
    }
    let resized = img.resize(
        IMAGE_MAX_DIM,
        IMAGE_MAX_DIM,
        image::imageops::FilterType::Triangle,
    );
    let mut buf = std::io::Cursor::new(Vec::new());
    let fmt = match mime {
        "image/jpeg" => image::ImageFormat::Jpeg,
        "image/gif" => image::ImageFormat::Gif,
        "image/webp" => image::ImageFormat::WebP,
        _ => image::ImageFormat::Png,
    };
    resized
        .write_to(&mut buf, fmt)
        .map_err(|_| format!("could not re-encode image at {}", path_arg))?;
    Ok(ImageData {
        bytes: buf.into_inner(),
        media_type: mime.to_string(),
    })
}

fn read_image(bytes: &[u8], mime: &str, path_arg: &str) -> Result<ToolOutput, AppError> {
    match prepare_image(bytes, mime, path_arg) {
        Ok(img) => Ok(ToolOutput::mixed(
            format!("Read image file [{}]", img.media_type),
            vec![img],
        )),
        Err(msg) => Ok(ToolOutput::error(msg)),
    }
}

/// Parse a 1-indexed page spec ("3" or "3-5") into an inclusive range, clamped
/// to `PDF_RENDER_MAX_PAGES` pages. `None` means the first page(s).
fn parse_page_range(spec: Option<&str>) -> Result<(u32, u32), String> {
    let bad = || {
        format!(
            "invalid pages '{}': use e.g. \"3\" or \"3-5\"",
            spec.unwrap_or("")
        )
    };
    let (first, last) = match spec.map(str::trim).filter(|s| !s.is_empty()) {
        None => (1, PDF_RENDER_MAX_PAGES),
        Some(s) => match s.split_once('-') {
            Some((a, b)) => (
                a.trim().parse().map_err(|_| bad())?,
                b.trim().parse().map_err(|_| bad())?,
            ),
            None => {
                let n: u32 = s.parse().map_err(|_| bad())?;
                (n, n)
            }
        },
    };
    if first == 0 || last < first {
        return Err(bad());
    }
    Ok((first, last.min(first + PDF_RENDER_MAX_PAGES - 1)))
}

/// Rasterise PDF pages to images with poppler's `pdftoppm`, for PDFs whose
/// text layer is missing or unhelpful (scans, charts, layout-heavy pages).
///
/// PDF parsers are a classic source of memory-safety bugs and the input is
/// untrusted, so `pdftoppm` runs inside the agent's sandbox (filesystem and
/// network restricted, timeout enforced) rather than in the server process.
async fn render_pdf(
    sandbox: &crate::tool::sandbox::Sandbox,
    path: &std::path::Path,
    path_arg: &str,
    pages: Option<&str>,
) -> ToolOutput {
    let (first, last) = match parse_page_range(pages) {
        Ok(r) => r,
        Err(msg) => return ToolOutput::error(msg),
    };
    // Output lands in the workspace because that's what the sandbox may write.
    let dir = match tempfile::Builder::new()
        .prefix(".pdf-render-")
        .tempdir_in(sandbox.path())
    {
        Ok(d) => d,
        Err(e) => return ToolOutput::error(format!("could not create render dir: {e}")),
    };
    let prefix = dir.path().join("page");
    if !sandbox.is_writable(&prefix) {
        return ToolOutput::error("the sandbox does not allow writing rendered pages");
    }

    let scale = PDF_RENDER_SCALE_TO.to_string();
    let (first_s, last_s) = (first.to_string(), last.to_string());
    let path_s = path.to_string_lossy();
    let prefix_s = prefix.to_string_lossy();
    let output = match sandbox
        .execute(
            "pdftoppm",
            &[
                "-png",
                "-scale-to",
                &scale,
                "-f",
                &first_s,
                "-l",
                &last_s,
                &path_s,
                &prefix_s,
            ],
            PDF_RENDER_TIMEOUT_SECS,
            None,
            None,
            None,
        )
        .await
    {
        Ok(o) => o,
        Err(e) => return ToolOutput::error(format!("could not run pdftoppm: {e}")),
    };
    if output.timed_out {
        return ToolOutput::error(format!("rendering {path_arg} timed out"));
    }
    if output.resource_killed {
        return ToolOutput::error(format!("rendering {path_arg} exceeded resource limits"));
    }

    let mut files = match std::fs::read_dir(dir.path()) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect::<Vec<_>>(),
        Err(e) => return ToolOutput::error(format!("could not read rendered pages: {e}")),
    };
    files.sort();
    if files.is_empty() {
        let detail = if output.exit_code == Some(127) || output.stderr.contains("not found") {
            "pdftoppm (poppler-utils) may not be installed on the server".to_string()
        } else {
            output.stderr.trim().to_string()
        };
        return ToolOutput::error(format!(
            "no pages rendered from {path_arg} (pages {first}-{last}): {detail}"
        ));
    }

    let mut images = Vec::new();
    for file in &files {
        let Ok(bytes) = std::fs::read(file) else {
            continue;
        };
        match prepare_image(&bytes, "image/png", path_arg) {
            Ok(img) => images.push(img),
            Err(msg) => return ToolOutput::error(msg),
        }
    }
    let shown_last = first + images.len() as u32 - 1;
    let mut note = format!("Rendered {path_arg} pages {first}-{shown_last} as images.");
    if shown_last == last {
        note.push_str(&format!(
            " There may be more pages; use pages=\"{}-{}\" to continue.",
            last + 1,
            last + PDF_RENDER_MAX_PAGES
        ));
    }
    ToolOutput::mixed(note, images)
}

fn read_pdf(
    bytes: &[u8],
    path_arg: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> ToolOutput {
    let text = match pdf_extract::extract_text_from_mem(bytes) {
        Ok(t) => t,
        Err(e) => {
            return ToolOutput::error(format!(
                "could not extract text from PDF at {path_arg} ({e})"
            ));
        }
    };
    if text.trim().is_empty() {
        return ToolOutput::error(format!(
            "PDF at {path_arg} has no extractable text layer (likely a scanned or image-only PDF); retry with render=true to view its pages as images"
        ));
    }
    read_text(&text, offset, limit)
}

fn read_text(text: &str, offset: Option<usize>, limit: Option<usize>) -> ToolOutput {
    let lines: Vec<&str> = text.lines().collect();
    let total_lines = lines.len();
    let start = offset.map(|n| n.saturating_sub(1)).unwrap_or(0);
    if start >= total_lines && total_lines > 0 {
        return ToolOutput::error(format!(
            "offset {} is past end of file ({} lines)",
            start + 1,
            total_lines
        ));
    }

    let end_unlimited = match limit {
        Some(l) => (start + l).min(total_lines),
        None => total_lines,
    };

    let mut byte_count = 0usize;
    let mut last_line = start;
    for (i, line) in lines[start..end_unlimited].iter().enumerate() {
        let next = byte_count + line.len() + 1; // +1 for the trailing newline
        if i >= MAX_LINES || next > MAX_BYTES {
            break;
        }
        byte_count = next;
        last_line = start + i + 1;
    }

    let mut body = lines[start..last_line].join("\n");
    let truncated_by_cap = last_line < end_unlimited;
    let more_after_limit = end_unlimited < total_lines;

    if truncated_by_cap || more_after_limit {
        let shown_first = start + 1;
        let shown_last = last_line.max(start + 1);
        let next_offset = last_line + 1;
        body.push_str(&format!(
            "\n\n[Showing lines {}-{} of {}. Use offset={} to continue.]",
            shown_first, shown_last, total_lines, next_offset
        ));
    }

    if body.is_empty() {
        ToolOutput::text(String::new())
    } else {
        ToolOutput::text(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_text_no_truncation() {
        let out = read_text("hello\nworld", None, None);
        assert_eq!(out.text_content(), "hello\nworld");
    }

    #[test]
    fn read_text_offset_limit() {
        let text = "a\nb\nc\nd\ne";
        let out = read_text(text, Some(2), Some(2));
        // start at line 2 ("b"), 2 lines → "b\nc"
        assert!(out.text_content().contains("b\nc"));
        // more remain, so continuation hint
        assert!(out.text_content().contains("Use offset=4"));
    }

    #[test]
    fn read_text_offset_past_end() {
        let out = read_text("a\nb", Some(99), None);
        assert!(!out.is_success());
    }

    #[test]
    fn is_image_branch() {
        assert!(is_supported_image("image/png"));
        assert!(is_supported_image("image/jpeg"));
        assert!(!is_supported_image("application/pdf"));
    }

    #[test]
    fn page_range_parsing() {
        assert_eq!(parse_page_range(None), Ok((1, 5)));
        assert_eq!(parse_page_range(Some("3")), Ok((3, 3)));
        assert_eq!(parse_page_range(Some("3-5")), Ok((3, 5)));
        assert_eq!(parse_page_range(Some("2-99")), Ok((2, 6)));
        assert!(parse_page_range(Some("0")).is_err());
        assert!(parse_page_range(Some("5-2")).is_err());
        assert!(parse_page_range(Some("x")).is_err());
    }

    /// A sandbox over a temp workspace. The driver is disabled so the test
    /// doesn't depend on syd/landlock being usable on the machine; it still
    /// exercises the `Sandbox::execute` path `render_pdf` goes through.
    fn test_sandbox(workspace: &std::path::Path) -> crate::tool::sandbox::Sandbox {
        use crate::tool::sandbox::SandboxFactory;
        use crate::tool::sandbox::driver::resource_monitor::SystemResourceManager;
        SandboxFactory::new(
            true,
            Arc::new(SystemResourceManager::new(80.0, 80.0, 90.0, 90.0)),
        )
        .get_sandbox(workspace.to_path_buf(), "test", false, Vec::new())
    }

    #[tokio::test]
    async fn render_pdf_returns_page_image() {
        if std::process::Command::new("pdftoppm")
            .arg("-v")
            .output()
            .is_err()
        {
            return; // poppler not installed on this machine
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.pdf");
        std::fs::write(&path, MINIMAL_PDF).unwrap();
        let mut out = render_pdf(&test_sandbox(dir.path()), &path, "t.pdf", None).await;
        assert!(out.is_success(), "{}", out.text_content());
        assert_eq!(out.take_images().len(), 1);
    }

    #[tokio::test]
    async fn render_pdf_page_out_of_range_errors() {
        if std::process::Command::new("pdftoppm")
            .arg("-v")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.pdf");
        std::fs::write(&path, MINIMAL_PDF).unwrap();
        let out = render_pdf(&test_sandbox(dir.path()), &path, "t.pdf", Some("9")).await;
        assert!(!out.is_success());
    }

    const MINIMAL_PDF: &[u8] = include_bytes!("testdata/minimal.pdf");

    #[test]
    fn read_pdf_extracts_text() {
        let out = read_pdf(MINIMAL_PDF, "ticket.pdf", None, None);
        assert!(out.is_success());
        assert!(out.text_content().contains("Hello World"));
    }

    #[test]
    fn read_pdf_garbage_bytes_errors() {
        let out = read_pdf(b"not a pdf", "bad.pdf", None, None);
        assert!(!out.is_success());
    }
}
