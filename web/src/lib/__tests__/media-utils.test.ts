import { describe, it, expect } from "vitest";

import { mediaKind } from "../media-utils";
import { detectContentType } from "../file-manager-utils";

describe("mediaKind", () => {
  it("classifies by content type", () => {
    expect(mediaKind("audio/mpeg", "song.mp3")).toBe("audio");
    expect(mediaKind("audio/ogg", "note.ogg")).toBe("audio");
    expect(mediaKind("video/mp4", "clip.mp4")).toBe("video");
    expect(mediaKind("video/quicktime", "clip.mov")).toBe("video");
  });

  it("returns null for non-media", () => {
    expect(mediaKind("image/png", "shot.png")).toBeNull();
    expect(mediaKind("text/markdown", "notes.md")).toBeNull();
    expect(mediaKind("application/pdf", "report.pdf")).toBeNull();
    expect(mediaKind(undefined, "report.pdf")).toBeNull();
  });

  it("falls back to the extension when the type is unknown", () => {
    // Files uploaded before the server's MIME table covered these extensions
    // are stored as application/octet-stream.
    expect(mediaKind("application/octet-stream", "voice.m4a")).toBe("audio");
    expect(mediaKind("application/octet-stream", "clip.MKV")).toBe("video");
    expect(mediaKind("", "track.flac")).toBe("audio");
    expect(mediaKind("application/octet-stream", "archive.zip")).toBeNull();
    expect(mediaKind("application/octet-stream", "noextension")).toBeNull();
  });

  it("trusts an explicit content type over the extension", () => {
    // A .bin holding audio still plays; a text file named .mov does not.
    expect(mediaKind("audio/mpeg", "download.bin")).toBe("audio");
    expect(mediaKind("text/plain", "notes.mov")).toBeNull();
  });
});

describe("detectContentType media coverage", () => {
  it("types every extension mediaKind can classify", () => {
    for (const name of [
      "a.mp3", "a.wav", "a.m4a", "a.aac", "a.ogg", "a.oga", "a.opus",
      "a.flac", "a.weba", "a.mp4", "a.webm", "a.m4v", "a.mov", "a.mkv",
      "a.avi", "a.ogv", "a.3gp", "a.mpeg", "a.mpg",
    ]) {
      const ct = detectContentType(name);
      expect(ct, name).not.toBe("application/octet-stream");
      expect(mediaKind(ct, name), name).not.toBeNull();
    }
  });
});
