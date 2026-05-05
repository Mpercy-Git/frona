---
id: await_signal
provider: builtin
parameters:
  title:
    type: string
    description: Short label for the signal task (≤ 80 chars). Shown in task lists and notifications. Keep it concise — "Verification code from BankX", "Reply from Sarah re launch", etc.
  description:
    type: string
    description: Plain-English description of what you're waiting for. Stored in full and used as context for the signal-owner agent when evaluating candidate messages.
  tags:
    type: array
    items: { type: string }
    description: Tags that classify the kind of message you're waiting for (e.g. ["verification_code","auth"]). Used by SignalService to score candidate matches via tag overlap.
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
    description: When true (default), the parent chat resumes with a TaskCompletion message when the signal fires. When false, the result is delivered to the signal task's chat only.
  max_evaluations:
    type: number
    description: Maximum number of candidate messages this watch can be evaluated against before auto-failing. Defaults to a config-driven cap.
required:
  - title
  - description
  - tags
---
Register a long-lived "wait for X" task. Returns immediately — the current
chat continues. When a matching inbound event arrives, you (this same
agent) are invoked in the signal task's chat to decide whether the
candidate is what you were waiting for.

Use cases:
- Verification codes from SMS/email after initiating a sign-up
- A reply from a specific contact ("tell me when Sarah says she's ready")
- Webhook callbacks from third-party services

You MUST provide at least one of `tags`, `expected_channels`, or
`expected_contacts`. Tag overlap is the primary match signal; channels and
contacts act as hard filters.

When invoked in the signal task's chat:
- If the candidate IS the signal, call `complete_task` with the relevant
  value (e.g. the verification code).
- If it is NOT, do nothing — the watch stays pending for the next candidate.
