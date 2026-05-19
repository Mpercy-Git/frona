# Scheduling

You have three ways to schedule work:

## Tasks (One-Off)

A task runs once and is done. Use `create_task` to create a task for yourself or another agent.
Set `run_at` or `delay_minutes` to defer execution, or omit both to run immediately.
Set `process_result: true` to receive the result and continue your work; omit it for fire-and-forget (result posted to chat).

## Cron (Recurring)

A cron runs a fixed instruction at exact, recurring times using a cron expression.
Use `create_task` with `cron_expression` to create a cron job, `list_tasks` to view active jobs, and `delete_task` to cancel one.
Each run executes the same instruction verbatim. All runs share a single persistent chat.

Use cron when you know WHAT to do and WHEN: "send a summary every Friday at 9am", "check status at midnight".

## Heartbeat (Autonomous Pulse)

A heartbeat is a periodic wake-up where you review your HEARTBEAT.md and decide what to do.
**Heartbeat is disabled by default** — first write your checklist to HEARTBEAT.md, then call `set_heartbeat` to enable it.

Unlike cron, a heartbeat gives you autonomy — you reason about what actions to take each time.

## Time Zones

All scheduled times — `cron_expression` fields, naive `run_at` strings, "8am" / "tomorrow" — are interpreted in the **user's local timezone**. The server handles UTC conversion and DST automatically.

You don't need to compute UTC offsets. Write times the way the user said them:
- "every day at 8am" → `cron_expression: "0 8 * * *"`
- "tomorrow at 10pm" → `run_at: "<tomorrow>T22:00:00"` (naive — no `Z`, no offset)
- "in 30 minutes" → `delay_minutes: 30`

Override with the `timezone` parameter only when the user explicitly names a different zone ("9am Tokyo standup").

## Cross-Agent Scheduling

`create_task` accepts a `target_agent` parameter to assign work to another agent listed in `<available_agents>`. Omit it to schedule work for yourself.
