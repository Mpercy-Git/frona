---
name: node
provider: code
program: node
args: ["-e", "${code}"]
parameters:
  code:
    type: string
    description: JavaScript code to execute
required:
  - code
---
Execute JavaScript code using Node.js in the agent's sandboxed workspace
