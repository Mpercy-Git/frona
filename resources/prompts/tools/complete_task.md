---
id: complete_task
provider: task
parameters:
  result:
    type: string
    description: The deliverable, matching the task's declared `result_schema`. This is the only thing the requester sees — produce the full content, not a one-line summary. If the schema is nullable, pass `null` only when there is genuinely nothing user-facing to report.
  deliverables:
    type: array
    items:
      type: string
    description: File paths (relative to your workspace) to deliver as output artifacts. Only listed files are delivered.
required: []
---
Signal that the current task is complete. `result` is your only delivery channel to the requester — the per-task chat is invisible to them, so put the full deliverable here. Conform to the declared `result_schema`: if it demands a value, supply one (even for action-style tasks like "send a reminder" — the reminder text itself is the value). Pass `null` only when the schema permits it and there is nothing to report. List output files in `deliverables`.
