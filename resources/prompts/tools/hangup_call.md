---
id: hangup_call
provider: voice_call
parameters: {}
required: []
---
End the active voice call and hang up. Call this when the conversation is complete or you need to terminate the call.

Always say a brief closing line to the caller in the same message as this tool call — e.g. "Thanks for calling, goodbye." Whatever you say alongside this call is spoken to the caller before the line drops; if you call this tool with no accompanying text, the call ends abruptly on silence.
