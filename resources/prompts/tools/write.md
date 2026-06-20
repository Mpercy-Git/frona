---
id: write
provider: file
parameters:
  path:
    type: string
    description: "Path to write. Bare paths resolve to your workspace. Parent directories are created automatically."
  content:
    type: string
    description: "Full contents to write."
  overwrite:
    type: boolean
    description: "Set to true to replace an existing file. Defaults to false — by default, write refuses if the path already exists."
required:
  - path
  - content
---
Create a new file at the given path. **Default is create-only**: if the path already exists, the call fails with an explicit error pointing you at `edit` (for surgical changes) or `overwrite: true` (for a full rewrite from scratch). Use `edit` whenever you want to change part of an existing file — it's cheaper and safer than rewriting from scratch with `overwrite: true`.
