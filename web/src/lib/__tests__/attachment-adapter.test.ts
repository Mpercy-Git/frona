import { describe, it, expect, vi, beforeEach } from "vitest";

// uploadFile is the only api-client dependency the adapter exercises; keep the
// rest of the module real so importing use-chat-runtime doesn't break.
vi.mock("../api-client", async (importActual) => {
  const actual = await importActual<typeof import("../api-client")>();
  return { ...actual, uploadFile: vi.fn() };
});

import { createFronaAttachmentAdapter } from "../use-chat-runtime";
import { uploadFile } from "../api-client";
import type { PendingAttachment } from "@assistant-ui/react";
import type { Attachment } from "../types";

const mockUpload = uploadFile as unknown as ReturnType<typeof vi.fn>;

beforeEach(() => {
  mockUpload.mockReset();
});

describe("fronaAttachmentAdapter", () => {
  it('accepts every file — accept must be exactly "*"', () => {
    // assistant-ui's fileMatchesAccept treats ONLY the literal "*" as a
    // wildcard; "*/*" rejects every real MIME type, silently dropping uploads.
    expect(createFronaAttachmentAdapter().accept).toBe("*");
  });

  it("registers the uploaded attachment on success", async () => {
    const uploaded: Attachment = {
      filename: "photo.jpg",
      content_type: "image/jpeg",
      size_bytes: 123,
      owner: "user:1",
      path: "photo.jpg",
      url: "https://example.test/photo.jpg",
    };
    mockUpload.mockResolvedValue(uploaded);

    const adapter = createFronaAttachmentAdapter();
    const file = new File(["x"], "photo.jpg", { type: "image/jpeg" });
    const pending = (await adapter.add({ file })) as PendingAttachment;

    expect(pending.id).toBe("photo.jpg");
    expect(pending.name).toBe("photo.jpg");
    expect(pending.status).toEqual({ type: "requires-action", reason: "composer-send" });
  });

  it("surfaces upload failures via onError and re-throws", async () => {
    mockUpload.mockRejectedValue(new Error("File too large (max 10MB)"));
    const onError = vi.fn();

    const adapter = createFronaAttachmentAdapter(onError);
    const file = new File(["x"], "big.png", { type: "image/png" });

    await expect(adapter.add({ file })).rejects.toThrow("File too large");
    expect(onError).toHaveBeenCalledOnce();
    expect(onError.mock.calls[0][0]).toContain("big.png");
    expect(onError.mock.calls[0][0]).toContain("File too large");
  });
});
