---
id: list_provider_billing
provider: cost
parameters: {}
required: []
---
List every configured provider with how it bills: pay-as-you-go, subscription (with fee, currency, included allowance and renewal day), or self-hosted. Also reports whether the provider is enabled, and whether the model catalogue has pricing for it.

Read this before drawing any conclusion about spend. The same recorded `cost_usd` means very different things across the three kinds:

- **metered** — the figure is money spent. Reducing it saves that money.
- **subscription** — the figure is the *list-price value* of usage already paid for by the fee. Within a period the fee is sunk: moving work off the provider saves nothing until you cancel the plan, and moving work *onto* it is free until the allowance runs out. Reason about remaining allowance and marginal cost, not about the fee.
- **self_hosted** — no money changes hands at all. Treat the figure as the avoided cost of running the same work on a hosted provider.

A provider with no explicit configuration is reported with its default: local runtimes (Ollama, llamafile) as self-hosted, everything else as metered.
