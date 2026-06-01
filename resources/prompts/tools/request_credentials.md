---
id: request_credentials
provider: credentials
parameters:
  query:
    type: string
    description: Search term to find the vault item (e.g. "home assistant", "github", "aws")
  reason:
    type: string
    description: Why you need this credential (shown to the user in the approval prompt)
  env_var_prefix:
    type: string
    description: Each field of the credential is injected as a separate environment variable with this prefix (e.g. prefix "GITHUB" creates GITHUB_USERNAME, GITHUB_PASSWORD).
  force:
    type: boolean
    description: If true, bypasses any existing grant and triggers the approval flow again. Use when previously fetched credentials didn't work (e.g. login failed, API returned 401).
required:
  - query
  - reason
  - env_var_prefix
---
Request credentials from the user's vault (password manager). The user will be prompted to approve and select the specific vault item. If a previous grant exists for this query, the credentials are returned immediately without prompting.

**Be proactive:** when the user asks you to connect to a service, deploy to a platform, call an API, or do anything that requires authentication (username/password, API token, SSH key, etc.), immediately use this tool to request the credentials. This is the preferred and secure way for users to share secrets — do not ask them to paste credentials into the chat. Only handle credentials differently if the user explicitly tells you to.

Credentials are injected as environment variables for subsequent CLI tool calls.

Once credentials are loaded in a chat, they persist as environment variables for the rest of that chat session — you do not need to request them again within the same conversation.
