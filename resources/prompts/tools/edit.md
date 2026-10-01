---
id: edit
provider: file
parameters:
  path:
    type: string
    description: "Path to the file to edit. Bare paths resolve to your workspace."
  old_string:
    type: string
    description: "Text to find. Matched byte-for-byte first; if nothing matches, whitespace, smart quotes, Unicode dashes and line endings are normalized and it is tried again. Quote the file's bytes as exactly as you can — the exact pass is the safe one — but a drifted needle is still rescued. Must be unique in the file (or pass replace_all)."
  new_string:
    type: string
    description: "Replacement text. Substituted in place of old_string while preserving the file's original line endings."
  replace_all:
    type: boolean
    description: "Replace every occurrence of old_string instead of failing on multiple matches. Use for rename-everywhere refactors."
required:
  - path
  - old_string
  - new_string
---
Make a surgical edit to an existing file by exact-string replacement.

Matching runs in two passes. First `old_string` is matched **byte-for-byte**; that pass rewrites only the bytes you named, so quote the file as exactly as you can. If it finds nothing, a second pass normalises **whitespace and punctuation** (smart quotes, Unicode dashes, special spaces, CRLF/LF, trailing whitespace) and tries again — so a needle that has drifted from the file still lands. When that rescue pass is what matched, the result says so: treat it as a sign your picture of the file is stale and re-read it before building the next edit on it.

`old_string` must be unique in the file unless you set `replace_all: true`. A single byte-exact hit wins even if the normalising pass would find other near-misses. If `old_string` matches zero locations the file may have drifted; re-read it. Prefer this over `sed` for any change to a file you've already created.
