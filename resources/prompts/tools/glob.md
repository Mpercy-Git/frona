---
id: glob
provider: file
parameters:
  pattern:
    type: string
    description: "Glob pattern. Supports **, *, ?, [...]. Matched against paths relative to the scope."
  path:
    type: string
    description: "Optional directory to search under. Defaults to your workspace. Cedar policy gates cross-workspace scopes."
required:
  - pattern
---
List files matching a glob pattern. Walks the directory tree respecting `.gitignore`, returning paths relative to the scope. Capped at 1000 results — narrow your pattern if you hit the cap. Prefer this over `find`-via-shell for typical file discovery.
