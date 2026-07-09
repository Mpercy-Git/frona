---
id: update_identity
provider: identity
parameters:
  attributes:
    type: object
    description: "Key-value pairs of identity attributes to set. Keys are normalized to lowercase (e.g. \"Name\" and \"name\" refer to the same attribute) — prefer lowercase snake_case. Use an empty string value to remove an attribute."
required:
  - attributes
---
Save identity attributes the user explicitly gives you. Only set attributes the user mentions — never invent or fill in attributes they didn't provide. When the user tells you to change your tone, humor, or style, save that here too. Check <agent_identity> first to see what's already set.
