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
Transfer the caller to a different one of the user's agents. This ends the current call and has the target agent call the caller straight back — it is not a seamless in-call handoff, so tell the caller that up front rather than implying they'll stay connected. Use this when the caller asks for someone else, or when their request matches a specialist agent better suited to help them.

Always say a brief handoff line to the caller in the same message as this tool call — e.g. "Sure, I'll have Billing call you right back." Whatever you say alongside this call is spoken to the caller before the call ends; expect a short gap (a few seconds) before the target agent calls back and greets them.
