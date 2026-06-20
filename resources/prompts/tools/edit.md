---
id: edit
provider: file
parameters:
  path:
    type: string
    description: "Path to the file to edit. Bare paths resolve to your workspace."
  old_string:
    type: string
    description: "Text to find. Whitespace, smart quotes, Unicode dashes, and line endings are normalized — you don't need byte-perfect matches, but the text must be unique in the file (or pass replace_all)."
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
Make a surgical edit to an existing file by exact-string replacement. Matching is **fuzzy on whitespace/punctuation** (smart quotes, Unicode dashes, special spaces, CRLF/LF, trailing whitespace) so you don't have to byte-match — but `old_string` must be unique in the file unless you set `replace_all: true`. If `old_string` matches zero locations the file may have drifted; re-read it. Prefer this over `sed` for any change to a file you've already created.
