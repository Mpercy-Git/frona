---
id: memory_remember
provider: memory
parameters:
  content:
    type: string
    description: One concrete, self-contained statement to remember (one sentence).
required:
  - content
---
Append a short, concrete memory to short memory. One sentence per call. The background process will later fact-check it, attach it to the right page, chain it over any older fact it supersedes, and decay it. Use during conversation for anything you want to recall next turn or next chat — names, values, dates, paths, debugging finds.
