---
id: store_agent_memory
provider: memory
parameters:
  memory:
    type: string
    description: A short, atomic insight for this agent's working memory
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
Store one or more memories in this agent's long-term context — pass every new item from this turn in a single call via `memories` rather than calling once per item. IMPORTANT: Before calling, carefully review <agent_memory>. Do NOT call this tool if the memory — or something very similar — is already listed there, even if worded differently. Each memory should be a short, atomic statement — working context, project details, decisions, or anything relevant to this agent's work. Set overrides to true ONLY when the new memory contradicts or updates a previously stored one.
