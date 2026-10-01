---
id: compare_models
provider: cost
parameters:
  candidates:
    type: array
    items:
      type: string
    description: "Candidate models to price, each as 'provider/model' (e.g. 'anthropic/claude-opus-4-5', 'openai/gpt-5')."
  baseline_model_ref:
    type: string
    description: "The model currently in use, as 'provider/model'. Deltas are computed against it."
  window_days:
    type: integer
    description: "Window whose observed traffic is repriced, in days, ending now. Defaults to 30."
  model_group:
    type: string
    description: "Restrict the observed traffic to one model group. Omit to reprice the baseline model's traffic across all groups."
  needs_tool_calling:
    type: boolean
    description: "Reject candidates that cannot call tools."
  needs_vision:
    type: boolean
    description: "Reject candidates that cannot accept image input."
  needs_reasoning:
    type: boolean
    description: "Reject candidates without reasoning support."
  needs_structured_output:
    type: boolean
    description: "Reject candidates without structured-output support."
  min_context_tokens:
    type: integer
    description: "Reject candidates whose context window is smaller than this. Use the model group's configured context window."
required:
  - candidates
---
Price candidate models against the traffic this server actually served — real prompt sizes, real cache hit rates, real output lengths — rather than against a headline rate per million tokens.

**Use this for every figure you quote. Never state a model's price from memory; catalogue prices change and yours will be stale.**

Each candidate comes back either priced (`cost_usd`, `delta_usd`, `delta_pct` against the baseline) or with a non-empty `rejected_because`. **A candidate with `rejected_because` must not be recommended** — it either cannot do the job (no tool calling, too small a context window, deprecated upstream) or cannot be priced at all. Report the reason instead of silently dropping it.

Two limits worth stating when a comparison is close:

- Cache *writes* are not broken out on stored usage rows, so a repriced cache write is charged at the candidate's fresh-input rate rather than its cache-write premium. This slightly understates candidates that charge a premium for writes (Anthropic).
- Deltas assume the same token mix on the new model. A model with different verbosity or reasoning behaviour will not reproduce it exactly.
