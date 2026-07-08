---
id: manage_policy
provider: policy
parameters:
  action:
    type: string
    description: "The action to perform: 'schema', 'create', 'update', 'delete', 'list', or 'validate'"
    enum:
      - schema
      - create
      - update
      - delete
      - list
      - validate
  id:
    type: string
    description: "Policy identifier (human-readable, unique per user). Required for create, update, and delete."
  description:
    type: string
    description: "A short description of what this policy does. Required for create."
  policy_text:
    type: string
    description: "The policy statements (without @id/@description annotations — those are added automatically from the id and description parameters). Required for create, update, and validate."
required:
  - action
---
Manage authorization policies that control agent permissions: tool access, delegation, communication, sandbox filesystem, and network access.

**Always call `schema` first** before creating or updating policies. The schema documents all available entity types, actions, valid formats, and includes examples.

## Workflow

1. Call `schema` to see available entity types, actions, and examples
2. Call `list` to see existing policies
3. Call `validate` to check syntax before saving
4. Call `create`, `update`, or `delete` to modify policies

A policy document can contain multiple statements.
