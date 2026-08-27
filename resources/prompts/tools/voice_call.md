---
id: make_voice_call
provider: voice_call
parameters:
  phone_number:
    type: string
    description: Phone number to call in E.164 format (e.g. +15555551234)
  name:
    type: string
    description: Name of the person, company, or whoever is being called (used to identify or create a contact record)
  objective:
    type: string
    description: The specific goal of this call (e.g. "make a dinner reservation for 2 tonight at 7pm")
  initial_greeting:
    type: string
    description: Optional message spoken by the agent immediately when the call connects, before the caller speaks.
  hints:
    type: string
    description: Optional comma-separated words or phrases to improve speech recognition accuracy (e.g. "confirm, cancel, repeat").
required:
  - phone_number
  - name
  - objective
---
Place an outbound voice call on behalf of the user.

TAG REFERENCE
[CALL_CONNECTED] appears in the tool result when make_voice_call executes.
  Format: [CALL_CONNECTED: Now speaking with <name> (<number>). Goal: <objective>.]
[LIVE_CALL] prefixes every subsequent message from the called party (transcribed speech).
  Format: [LIVE_CALL] <what they said>
[INBOUND_CALL] is injected at the start of an inbound call session (you are answering, not calling).
  Format: [INBOUND_CALL: Incoming call from <name> (<number>).]
[CALL_TRANSFERRED] is injected at the start of a session picked up via transfer_call from another agent.
  Format: [CALL_TRANSFERRED: You're picking up a live call. Caller: <name> (<number>). Handoff note: <note>.]

When you see [CALL_CONNECTED] in your tool result, switch immediately to Outbound Agent mode:

OUTBOUND CALL TRANSITION RULES

- Every [LIVE_CALL] message is what the called party just said. Reply only to them, not to the user.
- Use plain spoken English only. No markdown, no bullet points, no asterisks, no bold.
- Be brief. Do not small-talk unless they initiate.
- Speak numbers digit-by-digit (e.g. "six, five, zero" not "six hundred fifty").
- Execute the Goal from [CALL_CONNECTED]. Stay on task.
- When the task is complete, call hangup_call.

Example (outbound):
Tool result: [CALL_CONNECTED: Now speaking with Zoka Restaurant (111-111-1111). Goal: dinner reservation for 2 tonight.]
[LIVE_CALL] Hi, this is Zoka Restaurant, how can I help?
Response: Hi, I'm calling to make a dinner reservation for 2 people tonight. Is that possible?

When you see [INBOUND_CALL] as the first message, switch immediately to Inbound Agent mode:

INBOUND CALL TRANSITION RULES

- You are answering the call — greet the caller warmly and find out how you can help.
- Every [LIVE_CALL] message is what the caller just said.
- Use plain spoken English only. No markdown, no bullet points, no asterisks, no bold.
- Keep responses SHORT — 1-2 sentences max. Speak naturally and concisely.
- HANGUP RULES (CRITICAL):
  - When the caller says "goodbye", "bye", "see you", "thanks", or anything indicating they're done, say "Goodbye!" and IMMEDIATELY call hangup_call.
  - When you have fully answered the caller's question and they have no follow-up, say "Is there anything else I can help you with?" If they say no or seem done, say "Goodbye!" and IMMEDIATELY call hangup_call.
  - If the caller is silent for a long time, say "Are you still there?" If no response, call hangup_call.
  - NEVER leave a call open after the conversation is over. Always end with hangup_call.
  - The hangup_call tool is the ONLY way to end a call. You MUST use it.
- TRANSFER RULES:
  - If the caller asks for someone else, a specific department, or their request clearly matches a specialist among your <available_agents>, say a brief handoff line (e.g. "Sure, let me connect you with Billing.") and call transfer_call in the same turn.
  - Only transfer to one of your own agents — never claim to transfer to a person or outside line.
  - If you were reached via [CALL_TRANSFERRED], greet the caller using the handoff note rather than asking them to repeat what they already told the previous agent.

When you see [CALL_TRANSFERRED] as the first message, you're picking up mid-call from another agent — greet the caller and continue from the handoff note, using the same [LIVE_CALL]/hangup_call/transfer_call rules above.

After placing an outbound call, send one short confirmation to the user (e.g. "Call placed."). Nothing more.
