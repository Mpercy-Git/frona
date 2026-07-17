---
id: request_credentials
provider: credentials
parameters:
  query:
    type: string
    description: Search term to find a single vault item (e.g. "home assistant", "github", "aws"). Use this for one credential; use `queries` when a service needs several.
  queries:
    type: array
    description: >-
      Request several credentials at once so the user can provide them all in
      one approval. Use this when a service needs more than one secret (e.g. an
      app key AND a user key, or a client id + client secret). Each element is
      either a plain search string, or an object {"query": "...", "label":
      "..."} where `label` is a short human hint shown next to that slot so the
      user knows which key is which. Prefer this over asking one key at a time.
    items:
      type: object
  reason:
    type: string
    description: Why you need these credentials (shown to the user in the approval prompt)
  force:
    type: boolean
    description: If true, bypasses any existing grant and triggers the approval flow again. Use when previously fetched credentials didn't work (e.g. login failed, API returned 401).
required:
  - reason
---
Request credentials from the user's vault (password manager). The user is prompted to approve, select the specific vault item(s), and choose how each secret is bound (all fields under a prefix, or one specific field). If a previous grant already covers a query, that credential is returned immediately without prompting.

**Ask for everything at once.** Many APIs need more than one secret — an application key *and* a user key, a client id *and* a client secret, a token *and* a host. When that's the case, pass all of them in a single `queries` array with a clear `label` on each, so the user provides them all in one approval instead of you asking for one, resuming, then asking for the next. Only fall back to the single `query` parameter when exactly one secret is needed.

**Be proactive:** when the user asks you to connect to a service, deploy to a platform, call an API, or do anything that requires authentication (username/password, API token, SSH key, etc.), immediately use this tool to request the credentials. This is the preferred and secure way for users to share secrets — do not ask them to paste credentials into the chat. Only handle credentials differently if the user explicitly tells you to.

Credentials are injected as environment variables for subsequent CLI tool calls. The tool result lists the exact env var names that were set — use those names.

Once credentials are loaded in a chat, they persist as environment variables for the rest of that chat session — you do not need to request them again within the same conversation.
