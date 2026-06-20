---
id: read
provider: file
parameters:
  path:
    type: string
    description: "Path to the file. Bare paths (e.g. notes.md) resolve to your workspace; agent://other-agent/foo and user://me/bar are accepted if your policy allows them."
  offset:
    type: integer
    description: "Line number to start reading from (1-indexed). Combine with limit for paginated reads of large files."
  limit:
    type: integer
    description: "Maximum number of lines to read. Hard caps at 2000 lines or 50KB, whichever comes first."
required:
  - path
---
Read a text or image file. Images (PNG/JPG/GIF/WebP) return as inline image content, auto-resized to ≤2000×2000. Text files return their raw bytes, truncated to 2000 lines or 50KB with a continuation hint. Binary files (PDF, archives, etc.) return an error — use produce_file to surface those to the user. Prefer this over `cat`-via-shell for any file you intend to reason about.
