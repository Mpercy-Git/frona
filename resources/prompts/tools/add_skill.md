---
id: add_skill
provider: skills
parameters:
  repo:
    type: string
    description: The GitHub repository the skills come from, in "owner/repo" form (e.g. "anthropics/skills").
  skills:
    type: array
    description: Names of the skills to install, exactly as `search_skills` reported them. Ask for everything you need in one call so the user approves once.
    items:
      type: string
  reason:
    type: string
    description: Why you want these skills, in one sentence. Shown to the user in the approval prompt — say what you'll do with them, not just what they are.
  scope:
    type: string
    description: '"agent" (default) installs for this agent only. "user" installs for every agent the user owns — use it when they say the skill should be available everywhere.'
    enum: [agent, user]
required:
  - repo
  - skills
  - reason
---
Propose installing one or more skills from a repository. **Nothing is written until the user approves** — the request pauses your turn and shows them the skill names, descriptions and the reason you gave. They can decline.

Use it after `search_skills` has shown you a skill that fits the task at hand.

**Ask for everything at once.** If a job needs two skills, list both in `skills` so the user answers one prompt instead of two.

**Be honest in `reason`.** It is the only context the user has when deciding — "The user asked me to fill in a PDF form and I have no PDF instructions" is useful; "Installing a skill" is not.

Prefer the default `agent` scope. Use `scope: "user"` only when the user says they want the skill available to all of their agents.

On approval the tool reports each installed skill and the path to its `SKILL.md`. Read that file before using the skill; from the next turn on it also appears in `<available_skills>`.

If the user declines, continue the task without the skill — don't ask again for the same skill in the same conversation.
