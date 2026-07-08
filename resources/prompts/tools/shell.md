---
name: shell
program: /bin/bash
args: ["-c", "${command}"]
parameters:
  command:
    type: string
    description: The shell command to execute
required:
  - command
---
Execute a shell command in the agent's sandboxed workspace. Standard Linux/macOS utilities are available (e.g. git, ls, find, grep, sed, awk, curl, tar, zip, jq, etc.).
