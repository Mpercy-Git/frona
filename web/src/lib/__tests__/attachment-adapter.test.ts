import { describe, it, expect, vi, beforeEach } from "vitest";

// uploadFile is the only api-client dependency the adapter exercises; keep the
// rest of the module real so importing use-chat-runtime doesn't break.
vi.mock("../api-client", async (importActual) => {
  const actual = await importActual<typeof import("../api-client")>();
  return { ...actual, uploadFile: vi.fn() };
});

import { createFronaAttachmentAdapter, getBackendAttachment } from "../use-chat-runtime";
import { uploadFile } from "../api-client";
import type { PendingAttachment } from "@assistant-ui/react";

async function collect(gen: unknown): Promise<PendingAttachment[]> {
  const out: PendingAttachment[] = [];
  for await (const item of gen as AsyncIterable<PendingAttachment>) out.push(item);
  return out;
}
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
    const [running, pending] = await collect(adapter.add({ file }));

    expect(running.status.type).toBe("running");
    // The id is a client-generated key, stable across the upload's phases.
    expect(pending.id).toBe(running.id);
    expect(pending.name).toBe("photo.jpg");
    expect(pending.status).toEqual({ type: "requires-action", reason: "composer-send" });
    expect(getBackendAttachment(pending.id)).toEqual(uploaded);
  });

  it("surfaces upload failures via onError and keeps the attachment as an error", async () => {
    mockUpload.mockRejectedValue(new Error("File too large (max 10MB)"));
    const onError = vi.fn();

    const adapter = createFronaAttachmentAdapter(onError);
    const file = new File(["x"], "big.png", { type: "image/png" });

    const states = await collect(adapter.add({ file }));
    const last = states[states.length - 1];
    expect(last.status.type).toBe("incomplete");
    expect(onError).toHaveBeenCalledOnce();
    expect(onError.mock.calls[0][0]).toContain("big.png");
    expect(onError.mock.calls[0][0]).toContain("File too large");
  });
});
