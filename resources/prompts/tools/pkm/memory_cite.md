---
id: memory_cite
provider: memory
parameters:
  path:
    type: string
    description: The absolute path of the page you used, exactly as returned by memory_search.
  paths:
    type: array
    items:
      type: string
    description: The absolute paths of every page you used, in one call. Citing is bookkeeping — it never deserves a tool call per page.
anyOf:
  - required: [path]
  - required: [paths]
---
Record that a page HELPED you answer the user — bumps its usefulness so it ranks earlier next time. Pass the absolute path(s) `memory_search` returned; if you used several pages, cite them all in one call. Call this AFTER you read a page AND actually used it to answer — not for tangential or merely-opened pages.
