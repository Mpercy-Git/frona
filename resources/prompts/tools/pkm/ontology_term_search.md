---
id: ontology_term_search
provider: memory
parameters:
  term:
    type: string
    description: A word or partial name to search for, such as "organization", "worksFor", or "email". Matching is case-insensitive.
required:
  - term
---
Search ontology terms before minting a new `frona:` term. The search includes terms that
the current user directly uses, terms declared or proposed in the user's schema, and the
whole ontology catalogue. Exact matches rank before partial matches. A directly used user
term ranks before an equally good unused catalogue match. Results include each term's kind,
origin, user relevance, and label.
