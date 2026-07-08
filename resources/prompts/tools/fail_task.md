---
id: fail_task
provider: task
parameters:
  reason:
    type: string
    description: Why the task cannot be completed
required:
  - reason
---
Signal that the current task has failed and cannot be completed. The reason is delivered back to the requesting agent. Use this when the task is impossible to fulfill (e.g., unreachable contact, missing prerequisites, unrecoverable error).
