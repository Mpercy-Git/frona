---
id: store_user_memory
provider: memory
parameters:
  memory:
    type: string
    description: A short, atomic memory about the user to persist across all agents
  memories:
    type: array
    items:
      type: string
    description: Several memories to store in ONE call, each a short atomic statement. Use this whenever a turn produced more than one — they are stored (and compacted) separately either way.
  overrides:
    type: boolean
    description: Set to true if this memory contradicts or supersedes a previously stored one
    default: false
anyOf:
  - required: [memory]
  - required: [memories]
---
Store one or more memories about the user that persist across ALL agents — pass every new fact from this turn in a single call via `memories` rather than calling once per fact. Call this whenever the user shares something genuinely new — name, location, job, hobbies, preferences, relationships, goals, opinions, interests, projects they're working on, applications/services/tools/integrations they use or ask about, people/companies/products they mention, topics they keep returning to. Be proactive: if the user mentions something specific they care about, save it without being asked. IMPORTANT: Before calling, carefully review <user_memory>. Do NOT call this tool if the memory — or something very similar — is already listed there, even if worded differently. Only call when you have genuinely new information. Set overrides to true ONLY when the new memory contradicts or updates a previously stored one.
