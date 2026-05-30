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
  result_schema:
    type: object
    description: "Required. JSON Schema describing the shape of the task's `result` argument to `complete_task`. Drives both validation and how the result is rendered into this chat. Use the simplest shape that fits — patterns below."
required:
  - title
  - instruction
  - result_schema
---
Create a one-off task — immediate or deferred, for yourself or another agent. Use to parallelize work by splitting a problem into independent pieces, each running in its own chat. Target another agent from <available_agents> when expertise matches; target yourself to spawn parallel slices of your own work. Default behavior is fire-and-forget — the task runs and the completion summary lands in this chat for the user to read. Set `process_result: true` only when you'll synthesize the result with a fresh inference turn. For recurring work, use create_recurring_task. For periodic autonomous check-ins, use set_heartbeat.

## result_schema (required)

Every task must declare the shape of its `result`. The schema is injected into `complete_task`'s `result` parameter at run time, validated when the task closes, and used to render the completion summary into this chat. Pick the simplest shape that fits:

- **Single-value answer** — top-level scalar. The agent must pass a value.
  ```json
  { "type": "string", "description": "the joke text" }
  { "type": "number", "description": "computed total" }
  ```

- **Optional answer** — nullable scalar. Pass `null` to close silently (no row in this chat); pass a value to deliver.
  ```json
  { "type": ["string", "null"], "description": "alert text (null = nothing to report)" }
  ```

- **List output** — array of scalars; renders as a bullet list. Empty arrays render silently.
  ```json
  { "type": "array", "items": { "type": "string" }, "description": "found items" }
  ```

- **Multi-field structured output** — object with scalar properties. Each present property renders as `<description>: <value>` on its own line. Use `required` for mandatory fields.
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

- **Complex / nested schemas** (objects whose properties are themselves objects, arrays-of-objects, etc.) — require `process_result: true` so you (the parent) render the structured result with a fresh inference turn. Otherwise creation is rejected.
