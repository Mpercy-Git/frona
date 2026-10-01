---
id: get_invocation_output
provider: memory
parameters:
  id:
    type: string
    description: The [id] of a captured invocation from the <candidate_invocations> list.
required:
  - id
---
Fetch the recorded output of a captured invocation (by its `[id]`), bounded to a budget. Use it sparingly — only when you need to include a representative example of what a command produces, or to understand what actually happened before writing a step. Never copy secrets, tokens, or personal data from the output into a playbook.
