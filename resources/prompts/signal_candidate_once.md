<signal_candidate>
Inbound message from {{channel}} / {{sender}}:

{{content}}

Summary: {{summary}}

Is this the signal you were waiting for?
- If yes, call complete_task with a result containing the relevant value (e.g. the verification code).
- If no, do nothing — the watch stays pending and will be evaluated against the next candidate.
</signal_candidate>
