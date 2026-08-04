import { describe, it, expect } from "vitest";

import { previewKind } from "../preview-kind";

describe("previewKind", () => {
  it("classifies by extension when no content type is known", () => {
    // The file manager's listings carry names and sizes only, so this is the
    // normal path in.
    expect(previewKind(undefined, "shot.png")).toBe("image");
    expect(previewKind(undefined, "song.mp3")).toBe("audio");
    expect(previewKind(undefined, "clip.mp4")).toBe("video");
    expect(previewKind(undefined, "report.pdf")).toBe("pdf");
    expect(previewKind(undefined, "notes.md")).toBe("text");
    expect(previewKind(undefined, "main.rs")).toBe("text");
  });

  it("prefers an explicit content type", () => {
    expect(previewKind("image/webp", "download")).toBe("image");
    expect(previewKind("audio/mpeg", "download.bin")).toBe("audio");
  });

  it("falls back to the extension for octet-stream", () => {
    expect(previewKind("application/octet-stream", "voice.m4a")).toBe("audio");
    expect(previewKind("application/octet-stream", "shot.jpg")).toBe("image");
  });

  it("previews source files the MIME table doesn't type", () => {
    expect(previewKind(undefined, "component.tsx")).toBe("text");
    expect(previewKind(undefined, "query.sql")).toBe("text");
    expect(previewKind(undefined, "main.cpp")).toBe("text");
    // No dot to split on — the whole name is matched.
    expect(previewKind(undefined, "Dockerfile")).toBe("text");
    expect(previewKind(undefined, "Makefile")).toBe("text");
  });

  it("previews SVG as an image", () => {
    // /api/files serves SVG as an attachment so navigating to the raw URL
    // can't run its scripts; an <img> context is script-free, so previewing
    // it here is safe.
    expect(previewKind(undefined, "logo.svg")).toBe("image");
  });

  it("reports everything else as unsupported", () => {
    expect(previewKind(undefined, "archive.zip")).toBe("unsupported");
    expect(previewKind(undefined, "app.wasm")).toBe("unsupported");
    expect(previewKind(undefined, "noextension")).toBe("unsupported");
    expect(previewKind("application/zip", "bundle.zip")).toBe("unsupported");
  });
});
