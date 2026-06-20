---
id: create_task
provider: task
parameters:
  title:
    type: string
    description: A short title for the task
  instruction:
    type: string
    description: "Detailed, self-contained instructions. The target agent cannot see this conversation, so include all necessary context. When run_at or delay_minutes is set, omit timing language — the scheduler handles the when, and the instruction text is what the agent sees at fire time. Avoid embedding stale date/time references that will be wrong when the task actually runs."
  target_agent:
    type: string
    description: "Optional: agent name to assign to (from <available_agents>). Omit to create a task for yourself."
  process_result:
    type: boolean
    description: "Default false — fire-and-forget: the task runs, its completion summary lands in this chat (the user sees the result), and you don't re-engage. Set true if you'll process the result with a fresh inference turn — useful for parallelizing work (spawn multiple tasks and synthesize once they all return) or composing results across subtasks. The completion summary lands here regardless; this flag only controls whether you re-engage to compose, synthesize, or follow up."
  delay_minutes:
    type: integer
    description: "Defer execution by N minutes. Best choice for 'in N minutes/hours' — no date math needed. Cannot be used with run_at."
  run_at:
    type: string
    description: "Defer execution to a specific time. Accepts a unix timestamp OR an ISO 8601 datetime without offset like '2026-05-20T22:00:00' (interpreted in the user's local timezone, or the per-task `timezone` override). Prefer the naive form for natural requests like 'remind me at 10pm tomorrow' — the server handles the conversion. Do not include 'Z' or a numeric offset. Must be in the future. Cannot be used with delay_minutes."
  timezone:
    type: string
    description: "Optional IANA timezone (e.g. 'America/Los_Angeles', 'Asia/Tokyo') overriding the default for naive run_at in this task. Default is the user's local timezone. Set only when the user explicitly names a different zone — 'wake me at 6am London time'."
  result_description:
    type: string
    description: "One-line description of the result the executing agent should produce. The executor fills `complete_task.result` with a prose/markdown string matching this description, and it renders directly into this chat. Use this for almost every task; pass either this OR `result_schema`, not both. See examples below."
  result_schema:
    type: object
    description: "Advanced: JSON Schema describing the typed shape of `complete_task.result`. Use only when you'll programmatically consume structured fields (process_result=true and you'll read individual properties). For prose results delivered to a human, prefer `result_description`. Pass either this OR `result_description`, not both."
required:
  - title
  - instruction
---
Create a one-off task — immediate or deferred, for yourself or another agent. Use to parallelize work by splitting a problem into independent pieces, each running in its own chat. Target another agent from <available_agents> when expertise matches; target yourself to spawn parallel slices of your own work. Default behavior is fire-and-forget — the task runs and the completion summary lands in this chat for the user to read. Set `process_result: true` only when you'll synthesize the result with a fresh inference turn. For recurring work, use create_recurring_task. For periodic autonomous check-ins, use set_heartbeat.

## Describing the result (required: pick one)

The result spec tells the executing agent what to put in `complete_task.result` and shapes how it renders in this chat. Pass **exactly one** of:

### `result_description` (default — prose answer for a human)

Use for any task whose output is a message, report, or document the user reads directly. One line describing what the executing agent should produce. The executor returns a markdown string and it renders as-is in the chat.

```text
result_description: "A short, friendly reminder to drink water"
result_description: "Markdown research report on used H100 GPU prices, with sources"
result_description: "The verification code as a string"
result_description: "A summary of today's calendar with one bullet per event"
```

This is the right choice for >90% of tasks — research, summaries, reminders, drafted replies, extracted values, anything a user reads.

### `result_schema` (advanced — typed structure for programmatic use)

Use only when `process_result: true` and you'll read individual properties from the result. Pick the simplest shape that fits:

- **Single scalar** — top-level scalar.
  ```json
  { "type": "number", "description": "computed total" }
  ```

- **Optional answer** — nullable scalar. Pass `null` to close silently; pass a value to deliver.
  ```json
  { "type": ["string", "null"], "description": "alert text (null = nothing to report)" }
  ```

- **List output** — array of scalars; renders as a bullet list. Empty arrays render silently.
  ```json
  { "type": "array", "items": { "type": "string" }, "description": "found items" }
  ```

- **Multi-field structured output** — object with scalar properties.
  ```json
  {
    "type": "object",
    "properties": {
      "verdict":   { "type": "string", "description": "decision" },
      "rationale": { "type": "string", "description": "reasoning" }
    },
    "required": ["verdict", "rationale"]
  }
  ```

- **Complex / nested schemas** — avoid unless you actually need them. If you must, set `process_result: true` and include a required top-level `summary` string property (only that field renders to the user; everything else is for parent-agent consumption).

When in doubt, use `result_description`. Schemas are for handing typed data to a parent agent that will process it programmatically — not for steering the executor to produce a particular text format.
