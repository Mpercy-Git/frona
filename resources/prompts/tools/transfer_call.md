---
id: transfer_call
provider: voice_call
parameters:
  target_agent:
    type: string
    description: The agent to transfer the caller to — id, handle, or display name. Must be one of the user's own agents (see <available_agents>).
  handoff_note:
    type: string
    description: A short note on why the caller is being transferred and any context they've already given, so the next agent doesn't ask them to repeat themselves.
required:
  - target_agent
  - handoff_note
---
Transfer the live call to a different one of the user's agents, who picks up with their own voice. Use this when the caller asks for someone else, or when their request matches a specialist agent better suited to help them.

Always say a brief handoff line to the caller in the same message as this tool call — e.g. "Sure, let me connect you with Billing." Whatever you say alongside this call is spoken to the caller before the transfer happens; the caller then hears a short pause before the next agent greets them.
