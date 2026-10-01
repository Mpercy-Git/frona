---
id: search_skills
provider: skills
parameters:
  query:
    type: string
    description: Search term for the skill registry (e.g. "pdf", "spreadsheet", "kubernetes"). Returns matching skill names and the repositories they live in.
  repo:
    type: string
    description: A GitHub repository in "owner/repo" form. Lists every skill in that repository with its description, so you can see what each one actually does before proposing an install.
anyOf:
  - required: [query]
  - required: [repo]
---
Search the skill registry for skills that are **not** installed yet. `<available_skills>` only lists what this agent already has; this tool is how you find the rest.

Two modes:
- `query` — search the registry by keyword. Returns skill names, their repositories, and install counts. Descriptions are not available at this stage.
- `repo` — list every skill in one repository, **with descriptions**. Use this on a repository that came back from a `query` search (or one the user named) to find out what the skills do.

Results are marked `[already available to you]` when the skill is one you can already use — don't propose installing those.

**When to reach for this:** the user asks for something that clearly has an established workflow you don't have instructions for (a document format, a platform's deployment flow, a specialised analysis), and nothing in `<available_skills>` covers it. Search before improvising a long, error-prone approach from scratch.

Do not search on every turn. One or two searches per task at most, and only when a skill would plausibly change how you do the work.

Once you've found something worth having, call `add_skill` — installing always requires the user's approval.
