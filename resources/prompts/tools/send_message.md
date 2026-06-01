---
id: send_message
provider: messaging
parameters:
  content:
    type: string
    description: The message text to send to the user (supports markdown)
  attachments:
    type: array
    items:
      type: string
    description: File paths to attach to the message
required:
  - content
---
Initiate a new message to the user in their primary chat. Only available during autonomous heartbeat execution, where your current chat is the heartbeat scratch thread and you need to reach out to the user to surface something actionable. Do not use to confirm or restate work already done — inside a task, `complete_task.result` is the delivery channel; inside a chat, your reply text is already visible.
