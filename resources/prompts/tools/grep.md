---
id: grep
provider: file
parameters:
  pattern:
    type: string
    description: "Regex pattern, Rust regex flavor."
  path:
    type: string
    description: "Optional directory or file to search. Defaults to your workspace. Cedar policy gates cross-workspace scopes."
required:
  - pattern
---
Search files for a regex pattern, line by line. Walks the directory tree respecting `.gitignore`. Returns `{path}:{line_no}:{line_text}` for each matching line, line text truncated at 500 chars. Capped at 1000 matches — narrow your pattern if you hit the cap. Prefer this over `rg`-via-shell for typical content search.
