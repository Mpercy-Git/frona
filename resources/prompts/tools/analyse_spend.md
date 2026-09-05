---
id: analyse_spend
provider: cost
parameters:
  window_days:
    type: integer
    description: "Size of the window to analyse, in days, ending now. Defaults to 30."
  group_by:
    type: string
    enum: ["provider", "model", "model_group", "kind", "user"]
    description: "Which breakdown to include in full. The headline totals and per-provider billing status are always returned. Defaults to model."
required: []
---
Read instance-wide inference usage and cost for a time window — every user's spend on this server, not just your own.

Returns:

- **totals** — tokens, calls and list-price cost across the window.
- **metered_cost_usd** — money that actually left the account: the list-price cost of calls served by providers configured as pay-as-you-go.
- **subscription_cost_usd** — subscription fees attributable to this window, pro-rated from each provider's configured monthly cost.
- **subscription_list_value_usd** — what the calls covered by those subscriptions would have cost at list price. Compare it against `subscription_cost_usd` to judge whether a plan earns its fee.
- **self_hosted_list_value_usd** — list-price value of calls served by hardware the operator runs. Never billed, but it is what the same work would have cost hosted.
- **uncosted_calls** — calls the model catalogue had no price for. Their cost is *unmeasured, not zero*, so every total above understates by that amount. Say so if the count is material.
- **providers** — per provider: configured billing terms, the billing kinds the rows were actually written under, and allowance consumption where a subscription declares one.
- the requested breakdown, plus per-model rows carrying the observed token mix you need for `compare_models`.

Costs are list price in USD throughout. A provider configured in another currency reports its fee in that currency under `billing.currency` — do not convert it.
