---
id: memory_cite
provider: memory
parameters:
  path:
    type: string
    description: The absolute path of the page you used, exactly as returned by memory_search.
required:
  - path
---
Record that a page HELPED you answer the user — bumps its usefulness so it ranks earlier next time. Pass the absolute path `memory_search` returned. Call this AFTER you read a page AND actually used it to answer — not for tangential or merely-opened pages.
