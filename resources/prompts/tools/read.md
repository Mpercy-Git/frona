---
id: read
provider: file
parameters:
  path:
    type: string
    description: "Path to the file. Bare paths (e.g. notes.md) resolve to your workspace; agent://other-agent/foo and user://me/bar are accepted if your policy allows them."
  paths:
    type: array
    items:
      type: string
    description: "Several files to read in ONE call, instead of one call each. Use this whenever you already know the files you want — a search result set, a page plus the pages it links to, a config plus its schema. Each file comes back under its own `===== path =====` header, and a file that can't be read reports there without sinking the others."
  offset:
    type: integer
    description: "Line number to start reading from (1-indexed). Combine with limit for paginated reads of large files. Single-file reads only — ignored when you pass `paths`."
  limit:
    type: integer
    description: "Maximum number of lines to read. Hard caps at 2000 lines or 50KB, whichever comes first. Single-file reads only — ignored when you pass `paths`."
  render:
    type: boolean
    description: "PDFs only. Return pages as images instead of extracted text — use for scanned PDFs, charts, tables or layouts where the text comes out garbled or empty. Up to 5 pages per call. Single-file reads only."
  pages:
    type: string
    description: "With render=true: page or inclusive range to render, 1-indexed (e.g. \"3\" or \"3-7\"). Defaults to the first 5 pages; a longer range is cut to 5."
anyOf:
  - required: [path]
  - required: [paths]
---
Read a text, PDF or image file — or several at once via `paths`. Images (PNG/JPG/GIF/WebP) return as inline image content, auto-resized to ≤2000×2000 — if you can't view images, or only need one detail from one, use analyze_image instead. Text files return their raw bytes, truncated to 2000 lines or 50KB with a continuation hint. PDFs return their extracted text (same truncation and `offset`/`limit` paging); if the text is empty or garbled, pass `render=true` (optionally with `pages`) to see the pages as images. Other binary files (archives, etc.) return an error — use produce_file to surface those to the user. Prefer this over `cat`-via-shell for any file you intend to reason about.
