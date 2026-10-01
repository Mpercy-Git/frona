---
id: validation_details
provider: memory
parameters:
  diagnostic_id:
    type: string
    description: Stable diagnostic identifier shown in graph-validation feedback.
required:
  - diagnostic_id
---
Retrieve the complete structured witness for one projection-validation diagnostic. Use
the `diagnostic_id` shown in rejection feedback when the inline examples do not contain
enough causal axioms, triples, pages, or values to choose a repair.

Arguments:
- `diagnostic_id`: the stable diagnostic identifier from validation feedback.
