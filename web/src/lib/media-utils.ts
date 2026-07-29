export type MediaKind = "audio" | "video";

/** Fallback for attachments stored before the server's MIME table covered
 *  their extension — those sit on disk as application/octet-stream. */
const EXTENSION_MEDIA_KIND: Record<string, MediaKind> = {
  mp3: "audio",
  wav: "audio",
  m4a: "audio",
  aac: "audio",
  ogg: "audio",
  oga: "audio",
  opus: "audio",
  flac: "audio",
  weba: "audio",
  mp4: "video",
  webm: "video",
  m4v: "video",
  mov: "video",
  mkv: "video",
  avi: "video",
  ogv: "video",
  "3gp": "video",
  mpeg: "video",
  mpg: "video",
};

/**
 * Whether an attachment should render as a player, and which one. Returns
 * `null` for everything else.
 *
 * Being listed here is not a promise the browser can decode it — .mkv and
 * .flac play in some browsers and not others — so the player falls back to a
 * download link when the media element reports an error.
 */
export function mediaKind(
  contentType: string | undefined,
  filename: string,
): MediaKind | null {
  const ct = (contentType ?? "").toLowerCase();
  if (ct.startsWith("audio/")) return "audio";
  if (ct.startsWith("video/")) return "video";

  if (ct === "" || ct === "application/octet-stream") {
    const ext = filename.split(".").pop()?.toLowerCase();
    if (ext && ext in EXTENSION_MEDIA_KIND) return EXTENSION_MEDIA_KIND[ext];
  }

  return null;
}
