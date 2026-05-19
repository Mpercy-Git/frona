---
id: create_task
provider: task
parameters:
  title:
    type: string
    description: A short title for the task
  instruction:
    type: string
    description: "Detailed, self-contained instructions. The target agent cannot see this conversation, so include all necessary context. When run_at, delay_minutes, or cron_expression is set, omit timing language — the scheduler handles the when, and the instruction text is what the agent sees at fire time. Avoid embedding stale date/time references that will be wrong when the task actually runs."
  target_agent:
    type: string
    description: "Optional: agent name to assign to (from <available_agents>). Omit to create a task for yourself."
  process_result:
    type: boolean
    description: "If true, you will be resumed with the task result when it completes. Multiple tasks can run in parallel — you resume when all complete. Default: false (fire-and-forget, result posted to chat)."
  cron_expression:
    type: string
    description: "5-field cron expression (minute hour day-of-month month day-of-week). Interpreted in the user's local timezone (see <temporal_context>). Write '0 8 * * *' for '8am every day' — the server handles UTC conversion and DST automatically. Omit for one-off tasks."
  delay_minutes:
    type: integer
    description: "Defer execution by N minutes. Best choice for 'in N minutes/hours' — no date math needed. Cannot be used with run_at or cron_expression."
  run_at:
    type: string
    description: "Defer execution to a specific time. Accepts a unix timestamp OR an ISO 8601 datetime without offset like '2026-05-20T22:00:00' (interpreted in the user's local timezone, or the per-task `timezone` override). Prefer the naive form for natural requests like 'remind me at 10pm tomorrow' — the server handles the conversion. Do not include 'Z' or a numeric offset. Must be in the future. Cannot be used with delay_minutes."
  timezone:
    type: string
    description: "Optional IANA timezone (e.g. 'America/Los_Angeles', 'Asia/Tokyo') overriding the default for both cron_expression and naive run_at in this task. Default is the user's local timezone. Set only when the user explicitly names a different zone — 'every weekday at 9am Tokyo time', 'wake me at 6am London'."
required:
  - title
  - instruction
---
Create a task — one-off or recurring, for yourself or another agent. When a specialist agent in <available_agents> matches the work, always delegate by setting target_agent. Also use to: defer work to a later time, run background work in a separate context, or parallelize across multiple agents. Omit target_agent to create a task for yourself. Set cron_expression for recurring work. Set process_result to receive and act on the result yourself; omit it to let the result post directly to the chat. For periodic autonomous check-ins, use set_heartbeat.
