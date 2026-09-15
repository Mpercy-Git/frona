---
id: memory_search
provider: memory
parameters:
  query:
    type: string
    description: What you're looking for. Use the user's terms — names like 'home assistant', 'frona', or short descriptive phrases.
  queries:
    type: array
    items:
      type: string
    description: Several things to look up in ONE call, each its own query (e.g. ["postgres host","postgres port","postgres password"]). Every field of a config, every entity in the question — ask for them together rather than one call each. Results come back under a `### <query>` heading per query.
anyOf:
  - required: [query]
  - required: [queries]
---
Search your knowledge base — notes written *about* the user's world, not the world itself. It holds no live state, so a question about what a connected system is doing right now (a sensor's reading, today's calendar, whether a service is up) belongs to that system's MCP server, not here; use memory for the user's own naming and preferences, then call the server. This is a database lookup, so the same query always returns the same rows - one search plus at most one reformulation per thing you're after, and never a query you already ran this turn. When you need several things, pass them all as `queries` in a single call; each still counts as its own lookup, you just don't pay a tool round-trip per field. Returns up to 8 ranked pages per query (people, projects, services, places, topics, and playbooks), each with its name, a one-line description, a type tag, an **absolute file path**, and **the page's own text** in a `<page>` block. That text is usually the whole answer: read a page only when its block says the text was cut off or not shown, or when you need the YAML frontmatter (structured `attributes:` and links) rather than the prose — and then open them all in one `read(paths=[…])`. A `[playbook]` tag marks a reusable how-to procedure (for "how do I X?"); other tags are the concept kind (service, person, project…).
