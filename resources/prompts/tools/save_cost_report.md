---
id: save_cost_report
provider: cost
parameters:
  summary:
    type: string
    description: "Markdown narrative of what you found. Lead with the headline spend figure and the single most valuable change. A few short paragraphs, not an essay."
  window_days:
    type: integer
    description: "Window the report covers, in days, ending now. Use the same window you analysed. Defaults to 30."
  recommendations:
    type: array
    description: "Concrete, actionable recommendations. An empty list is a valid and useful outcome when the current setup is sound — say so in the summary."
    items:
      type: object
      properties:
        kind:
          type: string
          enum: ["switch_model", "rebalance_provider", "enable_caching", "subscription_underused", "subscription_overrun", "pricing_gap"]
          description: "What sort of change this is."
        model_group:
          type: string
          description: "The model group the change applies to, e.g. 'primary' or 'reasoning'. This is the unit an operator edits in config."
        from:
          type: string
          description: "Current model or provider, as 'provider/model'."
        to:
          type: string
          description: "Proposed model or provider, as 'provider/model'. Must be a candidate compare_models priced without rejecting."
        rationale:
          type: string
          description: "Why, in one or two sentences, citing the figures you were given."
        estimated_monthly_delta_usd:
          type: number
          description: "Signed monthly change in USD: negative saves money, positive costs more. Omit when you could not quantify it rather than guessing."
        confidence:
          type: string
          enum: ["high", "medium", "low"]
          description: "high only when compare_models priced it and the capability check passed cleanly."
      required: ["kind", "rationale", "confidence"]
required:
  - summary
  - recommendations
---
File the finished cost report. It is saved for admins to read and raised in the notification feed.

This records a recommendation — it changes nothing. Model groups, providers and config are edited by a human who reads what you wrote, so make the report enough to decide from: what it costs now, what you propose, what it would cost instead, and what the change would give up.

Call this once, at the end of an analysis. Do not file a report you have not backed with `analyse_spend` and, for any figure about an alternative model, `compare_models`.
