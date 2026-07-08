---
id: report_signal
provider: builtin
parameters:
  summary:
    type: string
    description: Plain-English summary of this match — what you observed, why it qualifies. Streamed into the source chat as a TaskMatch event so the parent agent (or the user) sees it. ≤ 512 bytes.
  result:
    type: string
    description: Optional structured value extracted from the candidate (e.g. an OTP code, a parsed reply, a JSON-encoded object). Validated against the task's `result_schema` if one was set on the watch. Object-shaped schemas require JSON-encoding the object as a string.
required:
  - summary
---
Record a match for a continuous signal watch *without* ending the watch.

Use this when the inbound candidate IS a relevant match for what you're
monitoring (e.g. one of the messages you're tracking, a notable update in
the channel you're watching). The match is delivered to the source chat as
a TaskMatch event; the watch stays active and will be invoked again on the
next matching candidate.

This tool is only available inside a continuous-mode signal task. If the
candidate is NOT a match, do nothing — the watch stays pending for the
next candidate. To stop monitoring proactively (e.g. you've reached the
natural end of the work), call `complete_task` instead.
