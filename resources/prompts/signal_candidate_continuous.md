<signal_candidate>
Inbound message from {{channel}} / {{sender}}:

{{content}}

Summary: {{summary}}

Is this a relevant match for what you're monitoring?
- If yes, call `report_signal` with a summary (and a structured `result` if
  the watch has a result_schema). The watch stays active and will be
  invoked again on the next matching candidate.
- If no, do nothing — the watch stays pending for the next candidate.
- If you've reached the natural end of monitoring (e.g. enough matches
  collected, the situation has resolved), call `complete_task` to stop
  the watch.
</signal_candidate>
