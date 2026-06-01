---
id: await_signal
provider: builtin
parameters:
  title:
    type: string
    description: Short label for the signal task (≤ 80 chars). Shown in task lists and notifications. Keep it concise — "Verification code from BankX", "Reply from Sarah re launch", etc.
  instructions:
    type: string
    description: Plain-English instructions for the signal-owner agent — what kind of inbound qualifies as a match, any context the matcher needs to judge candidates correctly. Stored in full and replayed to that agent on every candidate evaluation.
  expected_categories:
    type: array
    items: { type: string }
    description: Categorical labels that classify the kind of message you're waiting for (e.g. ["verification_code","auth"]). Used by SignalService to score candidate matches via category-annotation overlap.
  expected_channels:
    type: array
    items: { type: string }
    description: Optional hard filter — only candidates from these channel ids (e.g. ["sms","telegram"]) will be evaluated. Empty means any channel.
  expected_contacts:
    type: array
    items: { type: string }
    description: Optional hard filter — only candidates whose contact_id is in this list will be evaluated. Empty means any contact.
  expires_in_minutes:
    type: number
    description: Auto-fail the watch this many minutes from now if no match arrives. Preferred over `expires_at` when you can express the timeout as a relative duration.
  expires_at:
    type: string
    description: Absolute expiry — unix timestamp (seconds) or ISO 8601 datetime. Use `expires_in_minutes` instead unless you have an external deadline. Must be in the future.
  resume_parent:
    type: boolean
    description: When true (default), the parent chat resumes with a TaskCompletion message when the signal fires. When false, the result is delivered to the signal task's chat only. For continuous watches this only applies on terminal events (complete_task / fail_task / expiry) — per-match resumes are never triggered.
  mode:
    type: string
    enum: [once, continuous]
    description: |
      "once" (default) is a one-shot wait — the first match completes the task. Best for verification codes, single replies, single webhook callbacks. "continuous" keeps the task alive across many matches; each match invokes the signal-task agent which records the hit via `report_signal` without ending the watch. Best for monitoring a channel, tracking mentions, or watching for any of an open-ended set of events. Continuous watches stop on `expires_at` / `expires_in_minutes`, when `max_evaluations` is exhausted, or when the agent calls `complete_task` to deliberately stop monitoring. Size the expiry generously for continuous mode.
  max_evaluations:
    type: number
    description: Maximum number of candidate messages this watch can be evaluated against. For "once" mode, exhausting this fails the task. For "continuous" mode, exhausting it cleanly completes the watch (success-equivalent — it ran for its budgeted lifetime). Defaults to a config-driven cap that's higher for continuous than one-shot.
  result_schema:
    type: object
    description: |
      Optional JSON Schema document describing the shape of the value passed to the terminal/match-recording tool's `result` argument (`complete_task` for "once" mode, `report_signal` for "continuous" mode). Validated provider-side (LLM is constrained at generation time when supported) and server-side. Use this whenever the result has a known shape — extracted codes, structured judgments, enum replies — so attacker-controlled candidate content can't smuggle arbitrary text into the parent chat. Object-shaped schemas require the agent to JSON-encode the object as the `result` string.
required:
  - title
  - instructions
  - expected_categories
---
Register a long-lived "wait for X" task. Returns immediately — the current
chat continues. When a matching inbound event arrives, you (this same
agent) are invoked in the signal task's chat to decide whether the
candidate is what you were waiting for.

Two modes:

- **`mode: "once"` (default)** — first match completes the task. Use for
  verification codes, a single reply, a single webhook callback. The
  signal-task agent uses `complete_task` to record the result.
- **`mode: "continuous"`** — task stays alive across many matches. Each
  match invokes the signal-task agent in its dedicated chat; the agent
  records the hit via `report_signal` (non-terminal) and the watch stays
  active for the next candidate. Termination is driven by `expires_at` /
  `expires_in_minutes` (the primary stop), `max_evaluations` exhaustion,
  or an explicit `complete_task` call to deliberately stop monitoring.
  Use for: monitoring a channel for important alerts, tracking mentions
  over a window, watching for any of an open-ended set of events.

You MUST provide at least one of `expected_categories`,
`expected_channels`, or `expected_contacts`. Category-annotation overlap
is the primary match signal; channels and contacts act as hard filters.

When invoked in the signal task's chat:
- "once" mode: call `complete_task` with the relevant value if the
  candidate IS the signal; otherwise do nothing.
- "continuous" mode: call `report_signal` with a summary (and structured
  `result` if a schema is set) for each relevant match; otherwise do
  nothing. Call `complete_task` only to stop monitoring entirely.

Signal tasks always run with a restricted tool registry. "Once" mode
exposes `complete_task`, `fail_task`, `defer_task`. "Continuous" mode
exposes `report_signal`, `complete_task`, `fail_task` (no defer).
You cannot turn this off; it exists so attacker-controlled candidate
content cannot talk you into calling any other tool.

Examples of `result_schema`:

1. 6-digit 2FA code:
```json
"result_schema": {
  "type": "string",
  "pattern": "^[0-9]{6}$",
  "description": "6-digit numeric code"
}
```

2. Yes / no / cancelled reply:
```json
"result_schema": {
  "type": "string",
  "enum": ["yes", "no", "cancelled"]
}
```

3. Structured monitoring judgment (JSON-encode as the `result` string):
```json
"result_schema": {
  "type": "object",
  "properties": {
    "is_important": {"type": "string", "enum": ["yes", "no"]},
    "category": {"type": "string", "enum": ["dismissal", "schedule", "fundraiser", "other"]},
    "evidence_quote": {"type": "string", "maxLength": 300}
  },
  "required": ["is_important", "category"],
  "additionalProperties": false
}
```
