use std::io;
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::agent::prompt::PromptLoader;
use crate::core::error::AppError;
use crate::storage::service::StorageService;
use frona_derive::agent_tool;

use super::super::image::{is_supported_image, prepare_image};
use super::super::sandbox::SandboxManager;
use super::super::{InferenceContext, ToolOutput, str_list_arg};

const MAX_LINES: usize = 2000;
const MAX_BYTES: usize = 50 * 1024;

// Scan only a small directory before sorting. If it exceeds either bound,
// omit the listing rather than display an unstable filesystem-order subset.
const MAX_DIRECTORY_ENTRIES: usize = 128;
const MAX_DIRECTORY_NAME_BYTES: usize = 16 * 1024;
const MAX_DIRECTORY_DISPLAY_ENTRIES: usize = 50;
const MAX_DIRECTORY_OUTPUT_BYTES: usize = 8 * 1024;
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
        let metadata = match tokio::fs::metadata(&resolved).await {
            Ok(metadata) => metadata,
            Err(error) => return read_error(path_arg, error),
        };
        if metadata.is_dir() {
            return read_directory(&resolved).await;
        }

        let bytes = match tokio::fs::read(&resolved).await {
            Ok(bytes) => bytes,
            Err(error) => return read_error(path_arg, error),
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

fn read_error(path: &str, error: io::Error) -> Result<ToolOutput, AppError> {
    match error.kind() {
        io::ErrorKind::NotFound => Ok(ToolOutput::error(format!("file not found: {path}"))),
        io::ErrorKind::PermissionDenied
        | io::ErrorKind::InvalidInput
        | io::ErrorKind::InvalidData
        | io::ErrorKind::NotADirectory
        | io::ErrorKind::IsADirectory => {
            Ok(ToolOutput::error(format!("Could not read {path}: {error}")))
        }
        _ => Err(AppError::Internal(format!("read {path}: {error}"))),
    }
}

async fn read_directory(path: &Path) -> Result<ToolOutput, AppError> {
    const GUIDANCE: &str =
        "Read got a directory. Choose a file to read; directory names end with '/'.";
    const OMITTED: &str = "[Listing omitted: directory exceeds the scan limit (128 entries or 16 KiB of names). Use Glob with a narrower pattern to find a file.]";
    const TRUNCATED: &str =
        "\n[Listing truncated. Use Glob with a narrower pattern to find a file.]";

    let mut directory = match tokio::fs::read_dir(path).await {
        Ok(directory) => directory,
        Err(error) => return read_error("directory (could not list entries)", error),
    };
    let mut names = Vec::new();
    let mut name_bytes = 0;
    loop {
        let entry = match directory.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(error) => return read_error("directory (could not list entries)", error),
        };
        if names.len() == MAX_DIRECTORY_ENTRIES {
            return Ok(ToolOutput::error(format!("{GUIDANCE}\n{OMITTED}")));
        }
        let mut name = entry.file_name().to_string_lossy().into_owned();
        let file_type = match entry.file_type().await {
            Ok(file_type) => file_type,
            Err(error) => return read_error("directory (could not list entries)", error),
        };
        if file_type.is_dir() {
            name.push('/');
        }
        name_bytes += name.len();
        if name_bytes > MAX_DIRECTORY_NAME_BYTES {
            return Ok(ToolOutput::error(format!("{GUIDANCE}\n{OMITTED}")));
        }
        names.push(name);
    }
    if names.is_empty() {
        return Ok(ToolOutput::error(format!(
            "{GUIDANCE}\n[Directory is empty.]"
        )));
    }
    names.sort();
    let mut output = GUIDANCE.to_string();
    for (index, name) in names.iter().enumerate() {
        // Quote/escape names so embedded newlines cannot masquerade as entries.
        let line = format!("\n{name:?}");
        if index == MAX_DIRECTORY_DISPLAY_ENTRIES
            || output.len() + line.len() + TRUNCATED.len() > MAX_DIRECTORY_OUTPUT_BYTES
        {
            output.push_str(TRUNCATED);
            break;
        }
        output.push_str(&line);
    }
    Ok(ToolOutput::error(output))
}

fn read_image(bytes: &[u8], mime: &str, path_arg: &str) -> Result<ToolOutput, AppError> {
    Ok(match prepare_image(bytes, mime) {
        Ok(image) => ToolOutput::mixed(format!("Read image file [{mime}]"), vec![image]),
        Err(reason) => ToolOutput::error(format!("{reason} at {path_arg} ({mime})")),
    })
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
        match prepare_image(&bytes, "image/png") {
            Ok(img) => images.push(img),
            Err(reason) => return ToolOutput::error(format!("{reason} at {path_arg}")),
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
    fn unexpected_io_failures_remain_internal() {
        assert!(matches!(
            read_error("file", io::Error::other("unexpected I/O failure")),
            Err(AppError::Internal(_))
        ));
    }

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
