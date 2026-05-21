---
id: create_recurring_task
provider: task
parameters:
  title:
    type: string
    description: A short title for the recurring task.
  instruction:
    type: string
    description: "Self-contained instruction for each fire. The agent cannot see this conversation, so include all necessary context. Omit timing language — the scheduler handles the when. Avoid embedding stale dates that would be wrong on later fires."
  cron_expression:
    type: string
    description: "5-field cron expression (minute hour day-of-month month day-of-week). Interpreted in the user's local timezone (see <temporal_context>). Write '0 8 * * *' for '8am every day' — the server handles UTC conversion and DST automatically."
  target_agent:
    type: string
    description: "Optional: agent name to assign each fire to (from <available_agents>). Omit to fire for yourself."
  timezone:
    type: string
    description: "Optional IANA timezone (e.g. 'America/Los_Angeles', 'Asia/Tokyo') overriding the default for cron_expression interpretation. Default is the user's local timezone. Set only when the user explicitly names a different zone — 'every weekday at 9am Tokyo time'."
  cron_mode:
    type: string
    description: "How fires relate to each other. 'singleton' (default): only the latest fire matters — older runs are hidden from the task tree by default. Pick this for reminders, polling, and 'tell me if X' patterns. 'per_instance': each fire is a distinct audited work item — every run is visible. Pick this for monthly reports, recurring payments, and any pattern where each fire must not overlap with the previous."
  cron_concurrency:
    type: string
    description: "What to do when a previous fire is still in flight. 'allow': spawn anyway, runs in parallel. 'forbid': skip the new fire while previous is running. 'replace': cancel the previous fire and start fresh. Defaults: singleton→replace, per_instance→forbid."
  process_result:
    type: boolean
    description: "Default false — fire-and-forget: each cron run completes, its summary lands in this chat (the user sees each fire), and you don't re-engage. Set true if you'll process each run's result with a fresh inference turn — e.g., 'every hour check stock prices and tell me if AAPL moves >5%'. The completion summary lands here regardless; this flag only controls whether you re-engage after each fire."
required:
  - title
  - instruction
  - cron_expression
---
Create a recurring task that fires on a cron schedule. Use this for any work that should repeat: reminders, periodic polls, scheduled reports, regular check-ins.

Mode selection:
- "remind me to drink water every 2 minutes" → singleton + replace (newest fire matters; cancel older)
- "send me hourly news" → singleton + replace
- "generate a monthly report on the 1st" → per_instance + forbid (each report is its own audited work item; don't overlap)
- "process recurring payments daily at 9am" → per_instance + forbid

Set `process_result: true` when you'll react to each fire's result with a fresh inference turn — e.g., "every hour, check stock prices and tell me if AAPL moves >5%", "every morning summarize my calendar". Otherwise leave it off — fire-and-forget is the right default for reminders and routine background work, since the user sees each run's summary in this chat anyway.

For one-off or simply delayed work, use create_task. For periodic autonomous check-ins to your own state, use set_heartbeat.
