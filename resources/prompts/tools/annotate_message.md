---
id: annotate_message
provider: builtin
parameters:
  categories:
    type: array
    items: { type: string }
    description: Categorize this inbound — pick from the core taxonomy in your inbound instructions, plus up to 3 free-form labels. e.g. ["verification_code","auth"] or ["scheduling","personal","ready"].
  summary:
    type: string
    description: One-sentence summary of what the message is about. Helps signal-owner agents decide quickly.
required:
  - categories
---
Annotate an inbound message you just observed with structured features
(categories + optional summary). Used by the channel-agent when the
message looks like it could match a pending await_signal anywhere under
this user (e.g. a verification code, a contact's reply you've been
waiting on, an alert).

Calling annotate_message routes the message to SignalService for matching
against pending watches; if a watch matches and policy allows, the
signal-owner agent is invoked to evaluate.

Annotate generously — false positives are cheap (the signal-owner agent
discards non-matches), false negatives miss the signal.
