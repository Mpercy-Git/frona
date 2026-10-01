---
id: inspect_ontology_terms
provider: memory
parameters:
  terms:
    type: array
    items:
      type: string
    maxItems: 10
    description: One to ten class or property CURIEs to inspect and compare.
required:
  - terms
---
Inspect a bounded ontology hierarchy slice without querying the knowledge graph. The result
combines the whole ontology catalogue with the current user's committed and proposed schema.
For classes it returns direct parents, ancestors, capped direct children, equivalence, and
disjointness. For properties it also returns domain, range, and inverse terms. When several
terms are supplied, the result compares each pair as same, equivalent, subclass, superclass,
disjoint, or unrelated.
