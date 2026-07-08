---
id: defer_task
provider: task
parameters:
  delay_minutes:
    type: integer
    description: Number of minutes to wait before resuming the task
  reason:
    type: string
    description: Why the task is being deferred (e.g., "waiting for external response", "retry after cooldown")
required:
  - delay_minutes
  - reason
---
Pause the current task and schedule it to resume after a delay. The task will be picked up again by the scheduler after the specified number of minutes. Use this when the task needs to wait for an external event, retry later, or when periodic follow-up is needed.
