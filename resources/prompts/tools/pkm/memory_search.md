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
Search your knowledge base. This is a database lookup, so the same query always returns the same rows - one search plus at most one reformulation per thing you're after, and never a query you already ran this turn. When you need several things, pass them all as `queries` in a single call; each still counts as its own lookup, you just don't pay a tool round-trip per field. Returns up to 8 ranked pages per query (people, projects, services, places, topics, and playbooks), each with its name, a one-line description, a type tag, and an **absolute file path**. `read(paths=[…])` opens as many hits as you need in one call — pages are self-describing markdown: prose body plus YAML frontmatter carrying the structured facts (`attributes:`) and links. A `[playbook]` tag marks a reusable how-to procedure (for "how do I X?"); other tags are the concept kind (service, person, project…).
