---
id: store_user_memory
provider: memory
parameters:
  memory:
    type: string
    description: A short, atomic memory about the user to persist across all agents
  overrides:
    type: boolean
    description: Set to true if this memory contradicts or supersedes a previously stored one
    default: false
required:
  - memory
---
Store a memory about the user that persists across ALL agents. Call this whenever the user shares something genuinely new — name, location, job, hobbies, preferences, relationships, goals, opinions, interests, projects they're working on, applications/services/tools/integrations they use or ask about, people/companies/products they mention, topics they keep returning to. Be proactive: if the user mentions something specific they care about, save it without being asked. IMPORTANT: Before calling, carefully review <user_memory>. Do NOT call this tool if the memory — or something very similar — is already listed there, even if worded differently. Only call when you have genuinely new information. Set overrides to true ONLY when the new memory contradicts or updates a previously stored one.
