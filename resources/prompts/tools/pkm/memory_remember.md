---
id: memory_remember
provider: memory
parameters:
  content:
    type: string
    description: One concrete, self-contained statement to remember (one sentence).
  contents:
    type: array
    items:
      type: string
    description: Several statements to remember in ONE call, one sentence each. Use this whenever a conversation turned up more than one fact — they are stored and filed separately either way.
anyOf:
  - required: [content]
  - required: [contents]
---
Append short, concrete memories to short memory. One sentence per statement — but pass every statement you have in a single call via `contents` rather than calling once per fact. The background process will later fact-check each, attach it to the right page, chain it over any older fact it supersedes, and decay it. Use during conversation for anything you want to recall next turn or next chat — names, values, dates, paths, debugging finds.
