---
id: delete_task
provider: task
parameters:
  task_id:
    type: string
    description: The ID of the task to cancel
required:
  - task_id
---
Cancel a task by ID. Use list_tasks to find task IDs.
