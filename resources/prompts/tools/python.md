---
name: python
provider: code
program: python3
args: ["-c", "${code}"]
parameters:
  code:
    type: string
    description: Python code to execute
required:
  - code
---
Execute Python code in the agent's sandboxed workspace
