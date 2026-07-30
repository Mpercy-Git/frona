import { detectContentType } from "@/lib/file-manager-utils";
import { mediaKind } from "@/lib/media-utils";

export type PreviewKind = "image" | "audio" | "video" | "text" | "pdf" | "unsupported";

/** Source files the MIME table leaves as application/octet-stream but that are
 *  still worth showing as text. Kept alongside the language map in
 *  `file-preview-content`; a miss here only costs syntax highlighting. */
const TEXT_EXTENSIONS = new Set([
  "tsx", "jsx", "mjs", "cjs", "mts", "cts",
  "c", "cpp", "cc", "cxx", "h", "hpp", "cs",
  "swift", "kt", "kts", "scala", "php", "pl", "lua", "r", "dart", "zig",
  "sql", "graphql", "gql", "proto", "tf", "hcl", "nix",
  "vue", "svelte", "astro", "tex",
  "ini", "cfg", "conf", "env", "properties",
  "gitignore", "dockerignore", "editorconfig",
  "dockerfile", "makefile", "cmake", "gradle",
  "log", "lock", "diff", "patch",
]);

function isTextContentType(ct: string): boolean {
  return (
    ct.startsWith("text/") ||
    ct === "application/json" ||
    ct === "application/xml" ||
    ct === "application/javascript" ||
    ct === "application/typescript" ||
    ct === "application/x-yaml" ||
    ct === "application/toml"
  );
}

/**
 * How the Files tab should preview a file.
 *
 * `contentType` is usually absent in the file manager — its listings carry only
 * names and sizes — so the extension is the normal path in.
 */
export function previewKind(
  contentType: string | undefined,
  filename: string,
): PreviewKind {
  const explicit = (contentType ?? "").toLowerCase();
  const ct = explicit && explicit !== "application/octet-stream"
    ? explicit
    : detectContentType(filename).toLowerCase();

  const media = mediaKind(ct, filename);
  if (media) return media;

  // SVG previews safely here even though `/api/files` serves it as an
  // attachment: that header guards navigation to the raw URL, and an <img>
  // context can't run the scripts an SVG may carry.
  if (ct.startsWith("image/")) return "image";
  if (ct === "application/pdf") return "pdf";
  if (isTextContentType(ct)) return "text";

  const ext = filename.split(".").pop()?.toLowerCase();
  // Extension-less names like `Dockerfile` or `Makefile` have no dot to split
  // on, so `pop()` hands back the whole name — which is what we want to match.
  if (ext && TEXT_EXTENSIONS.has(ext)) return "text";

  return "unsupported";
}
