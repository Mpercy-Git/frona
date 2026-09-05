<p align="center">
    <a href="https://docs.frona.ai/" target="_blank">
        <img width="300" src="https://docs.frona.ai/logo-light.svg" alt="Frona AI">
    </a>
</p>

<p align="center">
    <a href="https://github.com/fronalabs/frona/releases"><img src="https://img.shields.io/github/v/release/fronalabs/frona?style=flat-square&color=blue" alt="Latest release"></a>
    <img src="https://img.shields.io/badge/built_with-Rust-dea584?style=flat-square&logo=rust" alt="Built with Rust">
    <a href="https://github.com/fronalabs/frona/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-BSL_1.1-blue?style=flat-square" alt="License"></a>
    <a href="https://github.com/fronalabs/frona/stargazers"><img src="https://img.shields.io/github/stars/fronalabs/frona?style=flat-square&logo=github" alt="GitHub stars"></a>
    <a href="https://github.com/Mpercy-Git/frona/pkgs/container/frona"><img src="https://img.shields.io/badge/ghcr.io-mpercy--git%2Ffrona-2496ed?style=flat-square&logo=docker&logoColor=white" alt="Container image"></a>
    <a href="https://docs.frona.ai/"><img src="https://img.shields.io/badge/docs-frona.ai-8a3ffc?style=flat-square" alt="Documentation"></a>
</p>

Frona is a personal AI assistant. You create autonomous agents that browse the web, run code, build applications, make phone calls, connect to messaging channels, delegate work to each other, and remember context across conversations, all within sandboxed environments with controlled access to your files, network, and credentials. You give them a task and they figure out how to get it done.

You deploy Frona on your own infrastructure and keep full control of your data. The platform is built from the ground up with security in mind, and the engine is written in Rust. So it's fast, lightweight, and runs everything in a single process.

> Comparing Frona to other open-source agent platforms? See [Frona vs. OpenClaw vs. Hermes Agent](https://docs.frona.ai/platform/comparison.html).

## ⭐ Fork Enhancements — Unique to This Repository

> **This is a fork of [fronalabs/frona](https://github.com/fronalabs/frona).** Everything in this section is **exclusive to this fork** and is *not* present in upstream. Images are published to `ghcr.io/mpercy-git/frona`, not the upstream registry.

The features and fixes below have been added on top of upstream. They are maintained here and do not exist in `fronalabs/frona`.

### 📞 Inbound voice calls (upstream is outbound-only)

Upstream Frona can only place **outbound** calls via Twilio. This fork adds full **inbound call answering**:

- **Answer incoming calls** and route them to an agent, with a **per-user allowlist** that locks who can reach which agent (malformed rows are skipped gracefully, ownership is enforced)
- **Plivo voice provider** alongside Twilio — pick `twilio`, `plivo`, or `none`
- **ElevenLabs TTS** for Twilio ConversationRelay (in addition to the default Polly)
- **Agent-narrated silence filling** — the agent speaks contextual filler during long tool calls instead of dead air, staying silent on outbound narration and narrating on inbound
- **Caller resolution by handle or name**, not just user ID, with a username (handle) inbound fallback
- **International number normalization** — `00`-prefixed UK/Europe numbers are handled correctly
- **Reverse-proxy-aware Twilio signature validation** (tries multiple URL variants; optional skip for debugging)
- **Streaming agent speech** — the agent's reply reaches the caller as it is generated, instead of the whole loop (tool rounds included) having to finish before a single word is spoken. A retried turn no longer speaks its opening twice
- Voice settings surfaced in the UI: inbound enable, silence-fill phrases/timing, caller allowlist, phone profile field

### 🔔 Web Push notifications & PWA (net-new)

- **Web Push with VAPID** — a service worker delivers OS-level push notifications for agent replies to subscribed devices, including mobile
- **Zero-config keys** — the server generates its own VAPID key pair on first start and keeps it in `{data_dir}/system/vapid.json`, so push works without anyone running `npx web-push generate-vapid-keys`; `FRONA_PUSH_VAPID_PUBLIC_KEY`/`FRONA_PUSH_VAPID_PRIVATE_KEY` still pin a pair of your own
- **High-urgency delivery** — pushes are sent with `Urgency: high`, so Android wakes the device and raises the notification immediately instead of holding it until Doze's next maintenance window
- **Smart suppression that fails towards notifying** — a push is held back only when an open page positively confirms it is focused on that exact chat; the service worker never infers this from `visibilityState`, which Android reports as "visible" for a locked or backgrounded window
- **Diagnosable** — a *Send test notification* button reports what each push service actually did with the message, so "nothing arrived" separates into no subscription, no VAPID key, a rejected signature, or an OS-level setting
- **Installable PWA with an Android app-like feel** — web app manifest, generated app icons (standard + maskable + apple-touch), `viewport-fit=cover`, safe-area insets, dynamic viewport height (`100dvh`), no accidental pull-to-refresh, and virtual-keyboard-aware composer scrolling
- Push subscriptions re-sync on mobile, iOS gets explicit install guidance, and the composer stays above the on-screen keyboard

### 🧭 OpenRouter routing, caching & cost accounting (net-new)

- **Routing preferences actually reach OpenRouter.** The config models provider preferences as `provider_routing` so the key doesn't collide with the `provider` enum discriminant, but the live request path never renamed it back — so `order`, `sort`, `ignore` and `quantizations` shipped under a key the API doesn't read and were silently dropped. They are now sent as the `provider` object OpenRouter documents
- **Prompt caching on by default.** A `cache_control` breakpoint is placed on the system prompt, so the large, stable system prompt and tool definitions that every turn of a tool loop resends are billed at the cache-read rate instead of the fresh-input rate on providers that honour explicit breakpoints (Anthropic, Gemini). Providers that cache automatically ignore the marker. Per-model-group opt-out via `prompt_caching: false` for one-shot workloads, where the cache write is never read back
- **More of the cost and privacy controls exposed**: a hard `max_price` ceiling (a request no provider can serve under it fails instead of routing somewhere expensive), an `only` hard allowlist, `data_collection: deny`, and `zdr`
- **`route` is documented and constrained correctly** — it is model-level fallback routing whose only accepted value is `fallback`, not a provider name as the UI previously suggested
- **Cost and context windows resolve for aggregator model ids.** `openrouter` + `anthropic/claude-sonnet-4-6` is not a literal models.dev key, so the exact-match lookup missed: every OpenRouter call recorded `cost_usd: None`, and the context window fell back to the 128K floor, firing compaction — and the extra summarisation call it costs — far earlier than a 200K/1M model requires. Both now resolve through the vendor-prefix walk
- **Cache writes are priced.** They were billed at the plain input rate; Anthropic charges a premium (1.25×) for them, which matters as soon as caching is on

### 💷 Cost analyst agent & provider billing model (net-new)

Every call has always recorded a `cost_usd` priced from the models.dev catalogue, but nobody was looking at the total, and the figure meant the same thing whether you paid per token or paid a flat monthly fee. Two gaps, and together they made "are we on the right provider and model mix?" unanswerable.

- **Providers now declare how they bill.** A `billing` block on each provider says `metered` (pay per token), `subscription` (flat fee, with optional included credit or token allowance, currency and renewal day), or `self_hosted`. Local runtimes default to self-hosted; everything else defaults to metered, so existing configs behave exactly as before. The classification is snapshotted onto each usage row alongside `pricing_version`, so switching to a subscription in March doesn't retroactively reclassify February's spend
- **`cost_usd` deliberately still means list price for every billing kind.** Zeroing it under a subscription would destroy the one number that says whether the plan is worth its fee. The classification travels alongside instead, so a report can separate **money actually spent** from **list-price value consumed under a fee already paid** — and tell you that you pay £20/month for a plan you used £4 of, or that £180 of usage rides a £20 plan
- **Instance-wide usage, admin-only.** Every rollup upstream is scoped to one user, and the usage API hard-blocks cross-user reads with no admin escape hatch, so an operator could see one person's spend and never the server's. New unscoped aggregations (by provider, model, model group, call kind, user) sit behind a new `view_usage_analytics` Cedar action — kept separate from `list_users` so spend visibility can be granted to, say, a finance group without granting account administration
- **A `cost-analyst` built-in agent** that admins chat with, and that files a report monthly on its own. It reprices the traffic actually served — real prompt sizes, real cache hit rates, real output lengths — against candidates from the model catalogue, and runs a capability gate (tool calling, vision, structured output, context window, deprecation) so it *cannot* recommend a model that can't do the job: a rejected candidate comes back with its reason attached rather than silently vanishing from the shortlist
- **The repricing path shares `ModelEntry::cost_for` with live costing**, so a recommendation and an invoice cannot drift apart; a round-trip test pins that a repriced stored row lands on exactly what the live call was charged, for both the Anthropic and OpenAI-shaped token conventions
- **Reports recommend, never apply.** Model groups and providers stay edited by a human who read the report. Calls the catalogue can't price are surfaced as a *pricing gap* rather than counted as free — unmeasured spend being the more dangerous of the two failure modes
- Built-in agents can now declare `groups:` (restricting a privileged built-in to a user group instead of cloning it into every account as an agent that can do nothing) and `cron:` (seeding a recurring task on first clone, editable and deletable like any other task)

### ☁️ Azure OpenAI (net-new)

Upstream has no Azure entry, and Azure doesn't fit the shared provider plumbing: it keys off a per-resource endpoint plus a data-plane version in the query string, and addresses models by *deployment* name.

- **`azure` provider** — `base_url` is your resource endpoint, `api_version` pins the data-plane version (it gates which request fields are accepted), and a model group's `model` is the deployment name you chose in Azure
- `api_version` is a new field on provider config, surfaced in the settings UI for Azure only, and auto-discovered from `AZURE_OPENAI_API_KEY` / `AZURE_OPENAI_ENDPOINT` / `AZURE_OPENAI_API_VERSION`
- Azure hosts the same gpt-5/o-series models as OpenAI, so it gets the same `max_tokens` → `max_completion_tokens` rewrite those models require
- A configured `api_key` is sent as the resource key, not as an Entra ID bearer token — rig's default `Into<String>` conversion picks the token path, which is not what an `api_key` field means everywhere else in this config

### 🔌 Any OpenAI-compatible endpoint, plus four more providers (net-new)

Upstream wires each provider by name, and `init_provider` hard-errors on anything else. `provider: generic` *parsed* — `ProviderModel::from_name` has always mapped it — but had no arm in the registry, and provider init only logs a warning on failure, so the provider silently didn't exist and the config surfaced as `ProviderNotConfigured` much later. The only workaround was naming your provider `openai` with a custom `base_url`, which drags in the gpt-5/o-series `max_tokens` → `max_completion_tokens` rewrite that vLLM, LM Studio and llama.cpp's server all reject.

- **`generic` now works** — point it at any OpenAI-compatible `/chat/completions` endpoint. `base_url` required, `api_key` optional (local servers usually ignore it), and `max_tokens` is sent unchanged. Covers vLLM, LM Studio, llama.cpp's server, LiteLLM and other proxies, and hosted services with no dedicated entry, without a code change each
- **Z.ai (GLM)**, **Venice** (privacy-focused, no-logging), and **MiniMax** added as named API-key providers, with auto-discovery from `ZAI_API_KEY` / `VENICE_API_KEY` / `MINIMAX_API_KEY`
- **llamafile** added as a second local option beside Ollama, discovered from `LLAMAFILE_API_BASE_URL`
- `generic` also carries the full OpenAI-compatible parameter set (`top_p`, `seed`, `stop`, penalties), so a local endpoint is tunable rather than a bare marker

### 🌏 BytePlus ModelArk (net-new)

Rig has no ModelArk adapter, and it needs none — Ark's data plane is OpenAI chat-completions under an `/api/v3` prefix, so Rig's OpenAI client reaches it unchanged. What it does need is to be kept *off* the OpenAI request hook, and to be findable in the catalogue.

- **`byteplus` provider** — `api_key` is required; `base_url` is optional and selects the account, defaulting to BytePlus international (`https://ark.ap-southeast.bytepluses.com/api/v3`) and overridable to the mainland Volcengine Ark host. Auto-discovered from `BYTEPLUS_API_KEY` / `BYTEPLUS_API_BASE_URL`
- **No `max_tokens` → `max_completion_tokens` rewrite.** Ark serves Seed/Doubao over plain chat-completions and rejects the rewritten field, so it sits with the compatible providers rather than with `openai`/`azure`. This is precisely why configuring it as `provider: openai` with a custom `base_url` does not work
- **Cost and context windows resolve.** models.dev files these models under `volcengine` — the international and mainland halves of one platform — so a `byteplus/…` ref matched nothing: every call recorded `cost_usd: None` and the window fell to the 128K default, firing compaction far earlier than a 256K Seed model requires. A catalogue alias maps `byteplus` onto the `volcengine` section, and the existing prefix walk then handles dated ids like `doubao-seed-1-8-251228`
- Model ids may be either a foundation-model id (`seed-1-6-250615`) or an inference endpoint id from your account (`ep-…`); both go in the model group's `model` field
- Structured output rides Ark's **function calling**, not its beta `response_format` — frona gets typed results through a forced `submit` tool call, so nothing here depends on that feature
- The settings "browse models" button probes OpenAI-shaped `GET /models`. Ark's documented model listing is a Volcengine Management API with its own signing, so this may come back empty; type the model id in that case. Everything on the inference path is unaffected

### 🤝 Sharing agents and chats (net-new)

Upstream has no sharing concept at all — a `user_id` equality check gated every read. This fork adds two independent grants:

- **Share an agent (use-only)** — a recipient can chat with and run an agent they don't own, but not edit it. Definition-scoped lookups (workspace, skills, sandbox policy) resolve under the **owner**, so a shared agent behaves exactly as its owner configured it no matter who drives it. Share by handle or email; re-sharing is idempotent; sharing with yourself is rejected
- **Optional credential delegation**, off by default — when the owner opts in, the recipient's runs may use the credentials the owner granted that agent. The ephemeral run token stays scoped to the runner regardless
- **Share a chat read-only** — the recipient views messages and attachments but cannot send, archive, delete, or resolve human-in-the-loop prompts. Attachments presign under the chat owner's identity so a non-owner viewer can actually load them. Shared chats are merged into listing and navigation with `is_shared`/`shared_by`, and the composer renders read-only for chats you don't own
- Registered-user sharing only; a public/anonymous read-only link is a separate follow-up

### 📂 Files, media & previews (net-new)

- **In-app preview dialog in the Files tab** — images, audio and video, text/markdown/source, and PDFs render without leaving the app. Anything else falls back to Open / Download. Previously the only way to view a file was a context-menu action that presigned it into a new tab
- SVG previews render in an `<img>` context, while `/api/files` continues to serve SVG as an attachment so script execution on direct navigation stays blocked
- **Inline audio/video playback for chat attachments** — media in a conversation plays in place instead of rendering as a download link, across assistant messages, user messages, and the Attachments tool UI

### 🧩 Skills & tasks (net-new)

- **Add skills manually, without a repo** — hand-write a `SKILL.md` or fix one whose upstream frontmatter is wrong, instead of being limited to installing from GitHub
- **The agent can find and add skills itself** — upstream, an agent only sees the skills already installed, so a task nobody installed a skill for looks like a task no skill exists for. `search_skills` reads the registry (keyword search, or a repo listing with descriptions) and `add_skill` proposes an install that pauses for the user's approval — the same consent shape as the vault: the agent can ask, but nothing is written without a yes. Approved skills land on this agent or on every agent the user owns, and are usable on the very next turn
- **Website citations in task completion summaries** — sources from `web_search`/`web_fetch` are preserved structurally rather than surviving only if the model happens to retype them

### 🛡️ Security & correctness hardening (fork-only fixes)

A dedicated review pass fixed issues not present upstream, including:

- SSRF guard on push subscription endpoints; IDOR fixes on tool-call and MCP log endpoints; presigned-token subject validation; refresh-token replay race; OAuth CSRF state TTL; path-traversal guards on workspace and file routes; stored-XSS mitigation for served SVG/HTML
- UTF-8 byte-index truncation panics fixed; cancelled-scheduler push state fix; config `GET /api/config` now reads from disk so saved settings don't silently revert; a channel leaves `Setup` for `Disconnected` once required fields are provided
- **Space management:** spaces can be archived or deleted from the chat sidebar (mirroring chats); deleting a channel no longer deletes its space, since spaces are independent and can hold chats
- Chat file-upload robustness: friendlier over-size errors, no silent attachment drops, no leaked blob URLs
- **Shared-agent workspace paths:** the file tools resolved relative paths under the *runner* while the sandbox mounts the *owner's* workspace. Harmless for an owned agent (the two are the same user) but it put every path in a shared run outside the mount, so reads and writes were denied outright. Both now resolve under the owner, and the resolver takes the whole inference context so the pairing can't be mismatched again
- SMTP password redacted in config responses; mail and login-lockout settings surfaced in the UI
- A superseded turn's cancellation no longer resets the UI mid-interrupt; the tasks tab sorts by status rather than just active vs finished

### 🔑 Account recovery & login lockout (net-new)

Local-auth accounts previously had no way back in: no password reset, no
self-service change, and no admin override — a forgotten password meant
deleting and recreating the account.

- **Failed-login lockout** that can't be sidestepped by re-casing the identifier, runs for its full duration from the moment it engages, and counts only genuine credential rejections (a server error or a deactivated account no longer burns a user's budget). Configurable via `auth.max_login_attempts` and `auth.lockout_minutes`; `0` disables it. A locked identifier answers `429` with `Retry-After`
- **Forgot-password over SMTP** — single-use, hash-at-rest reset tokens with a short TTL, sent via `mail.*` config. The request endpoint answers identically for registered and unregistered addresses, so it can't be used to enumerate accounts
- **Self-service password change** (`PUT /api/auth/password`) requiring the current password, which revokes every other session
- **Admin reset and unlock** (`PUT /api/admin/users/{id}/password`, `POST /api/admin/users/{id}/unlock`), surfaced in Settings → Users
- **Break-glass CLI** (`frona reset-password --handle <h>`) for when the sole admin is locked out and no authenticated caller exists
- Failed logins and lockouts are now logged and exported as `frona_auth_login_failures_total` / `frona_auth_lockouts_total`

### 🏗️ Build & release (fork-only)

- Release workflow publishing multi-arch images to **this fork's GHCR** (`ghcr.io/mpercy-git/frona`)
- Faster Docker builds: shared `rust-base` stage (installs toolchain once), persistent local BuildKit cache, and a `Makefile` for native-arch local builds

## Security First

AI agents are powerful. They can execute code, browse websites, and access your data. No platform can make LLMs perfectly safe. They will make mistakes. The goal is to isolate those mistakes and reduce the blast radius when they happen.

- **Per-principal sandboxing:** every actor (agent, MCP server, app, channel) is its own principal with its own policies. Each CLI tool call, each MCP server, each deployed app runs in its own sandboxed Linux process with policy-driven syscall filtering. There's no Docker container per agent and no daemon to manage; the engine spawns and reaps sandboxes on demand
- **One policy engine:** tool access *and* sandbox rules (read/write paths, network destinations, port binds) are written in the same policy language and evaluated by a single engine. One language, one decision point, no glue code between authorization and isolation
- **Isolated browser sessions:** each user gets separate browser profiles. Different credentials get separate browser states. One user's cookies and sessions are never visible to another
- **Credential vault:** agents request credentials when they need them, and you approve or deny in real time. Supports 1Password, Bitwarden, HashiCorp Vault, and KeePass. Secrets are never stored in agent memory or sent to LLM providers
- **Dual LLM dispatch on inbound:** untrusted channel messages can be routed to a quarantined LLM with a restricted tool registry, so a hostile inbound message can't talk the agent into running tools or leaking data on its behalf
- **Self-hosted by design:** your data lives on your servers. You choose which LLM provider to use, and traffic goes directly from your instance to that provider

## Features

- **Two memory backends:** use lightweight Basic memory or ontology-backed PKM, which turns conversations into evidence-grounded memories, typed entities and relationships, reusable playbooks, and searchable Markdown pages. Inspect the resulting knowledge graph and consolidation history from the Memory UI
- **Autonomous agents with tools:** agents decide which tools to use and execute multi-step tasks on their own. Agents can also build their own tools
- **Channels:** connect agents through Telegram, Slack, Discord, WhatsApp, Signal, or Twilio SMS so the same agent, with the same memory and tools, follows you outside the web UI. Channels support device pairing, policy-gated Message and Signal modes, interactive approval prompts, and automatic reconnection
- **Signals:** an agent can pause a conversation and wait for a matching inbound (a 2FA code, a reply, a class of message) and resume automatically when something arrives, or run continuous monitors with structured results
- **MCP with bridge mode:** install [Model Context Protocol](https://modelcontextprotocol.io) servers from the public registry in a click. Bridge mode advertises a single `mcpctl` CLI to the LLM instead of every MCP tool individually, saving thousands of tokens per turn on agents with many servers connected
- **Browser automation:** headless Chrome via Browserless for navigating websites, filling forms, and extracting data. Persistent browser profiles keep sessions across conversations
- **Web search:** built-in search via SearXNG, Tavily, or Brave Search
- **Code execution:** sandboxed shell, Python, and Node.js with per-principal filesystem, network, and resource restrictions
- **App deployment:** agents build and deploy web applications and services on your behalf, with an approval workflow before anything goes live
- **Skills:** instruction packages that teach agents new capabilities. Install shared skills or create agent-specific ones
- **Scheduling and heartbeats:** recurring tasks via cron and agent-managed heartbeat checklists for ongoing monitoring
- **Voice calls:** outbound phone calls via Twilio with speech recognition and DTMF navigation (optional)
- **Agent-to-agent delegation:** agents hand off tasks to specialized agents and get results back
- **Sharing:** hand another registered user an agent to run (without letting them edit it) or a chat to read, with optional credential delegation on shared agents
- **Spaces:** group conversations that share context. The platform summarizes linked conversations and feeds the context into new chats
- **Usage and cost visibility:** monitor tokens, cost, context-window usage, and model fallbacks live in each chat, with per-user dashboards for spend, latency, cache efficiency, models, and call types. Admins additionally get server-wide spend and a [cost analyst agent](#-cost-analyst-agent--provider-billing-model-net-new) that reviews the provider and model mix on a schedule
- **Commands:** use slash commands and mentions to invoke built-in actions, installed skills, or other agents directly from chat
- **Notifications:** agents push status updates (task finished, app deployed, credential needs approval) into a feed in the top bar so nothing important gets lost
- **Real-time streaming:** token-by-token response streaming over Server-Sent Events
- **SSO:** OpenID Connect support for single sign-on with Google, Keycloak, and other OIDC providers
- **Single-container deployment:** the entire backend (API server, embedded database, scheduler, tool execution) runs in one rootless OCI container (compatible with Docker, Podman, and other OCI runtimes). No per-agent containers, even at scale

## Core Concepts

- **Agents** are the main building blocks. Each agent has a name, a system prompt that defines its behavior, a model group that determines which LLM it uses, and a list of tools it can access. Frona ships with built-in agents (Assistant, Researcher, Developer, Receptionist) and you can create your own.
- **Policies** authorize every action: tool calls, delegations, file reads, network connections, and inbound channel messages. The same engine controls tool access and sandbox rules, so authorization lives in one place.
- **Memory** persists knowledge across conversations. Basic memory maintains compact user-scoped facts shared across agents and private agent-scoped notes. PKM builds a user-scoped knowledge graph of grounded atomic memories, entities, relationships, attributes, playbooks, and readable Markdown pages backed by an ontology.
- **Tools** are capabilities you give to agents. Browser automation, web search, file operations, shell commands, voice calls, task scheduling, and more. Tools run server-side and return results to the agent.
- **MCP servers** are first-class citizens. Each runs in its own sandbox as its own principal with its own filesystem, network, and resource policies, and surfaces its tools to agents through bridge mode by default.
- **Channels** connect an agent to messaging providers. Each channel is bound to a single agent and space, with policy-gated `receive_message` and `receive_signal` actions deciding what an inbound is allowed to do.
- **Signals** are "wait for X to happen" tasks. An agent calls `await_signal` and the conversation resumes when an inbound message matches.
- **Tasks** represent units of work. They can be direct (run immediately), delegated (from one agent to another), or scheduled (recurring via cron expressions).
- **Chat** is how you interact with agents. Each conversation belongs to one agent, but multiple agents can contribute to it through delegation. Messages stream in real-time over Server-Sent Events.
- **Spaces** are groups of chats that share the same context. When you link conversations to a space, the platform summarizes those conversations and feeds the context back into new chats.
- **Shares** grant another registered user access to something you own. An agent share is use-only — they can run it, they can't edit it, and it executes under your workspace, skills, and sandbox policy. A chat share is read-only. Neither transfers ownership, and both are revocable.
- **Skills** are instruction packages you install on agents. They can be built-in, shared across all agents, or scoped to a single agent.

## Quickstart

You'll need an OCI runtime with Compose v2 support, such as [Docker](https://docs.docker.com/get-docker/) or [Podman](https://podman.io/).

```yaml
# docker-compose.yml
services:
  frona:
    image: ghcr.io/mpercy-git/frona:latest
    ports:
      - "3001:3001"
    volumes:
      - ./data:/app/data
    environment:
      - FRONA_BROWSER_WS_URL=ws://browserless:3333
      - FRONA_SEARCH_SEARXNG_BASE_URL=http://searxng:8080
    # Only needed if you plan to restrict agent network destinations.
    # See https://docs.frona.ai/platform/security/sandbox.html
    security_opt:
      - seccomp:unconfined
    depends_on:
      - browserless
      - searxng
    restart: unless-stopped

  browserless:
    image: ghcr.io/browserless/chromium:v2.42.0
    environment:
      - CONCURRENT=10
    volumes:
      - ./data/browser_profiles:/profiles
    restart: unless-stopped

  searxng:
    image: searxng/searxng:latest
    environment:
      - SEARXNG_BASE_URL=http://searxng:8080
      - SEARXNG_SECRET=change-me-to-something-random
    configs:
      - source: searxng-settings
        target: /etc/searxng/settings.yml
    restart: unless-stopped

configs:
  searxng-settings:
    content: |
      use_default_settings: true
      server:
        limiter: false
      engines:
        - name: ahmia
          disabled: true
        - name: torch
          disabled: true
        - name: radio browser
          disabled: true
      search:
        formats:
          - html
          - json
```

```bash
docker compose up -d   # or: podman compose up -d
open http://localhost:3001
```

The setup wizard will guide you through creating your account and configuring your LLM provider.

See the [docker-compose example](examples/docker-compose) for a full deployment with environment configuration, the [documentation](https://docs.frona.ai) for detailed guides, or [screenshots](https://docs.frona.ai/platform/screenshots.html) to see the platform in action.

## Providers

Frona auto-discovers providers from your configuration and routes different tasks to the right one. Configure them in the [config file](https://docs.frona.ai/platform/deployment/config-file.html).

**LLM:** Anthropic, OpenAI, Google Gemini, DeepSeek, Mistral, Cohere, xAI (Grok), Groq, OpenRouter, Together, Perplexity, Hyperbolic, Moonshot, Hugging Face, Mira, Galadriel, Ollama (local).

**Search:** SearXNG (self-hosted), Tavily, Brave Search.

**Voice:** Twilio and Plivo, with ElevenLabs or Polly TTS — inbound *and* outbound calls (inbound answering is a [fork enhancement](#-fork-enhancements--unique-to-this-repository)).

**Channels:** Telegram, Slack, Discord, WhatsApp Cloud API, WhatsApp Personal, Signal, and Twilio SMS. WhatsApp Personal and Signal use linked-device integrations; review the provider-specific notices in the [channel documentation](https://docs.frona.ai/platform/agents/channels/overview.html) before enabling them.

## Architecture

Frona ships as a single rootless OCI image containing two main components:

- **Engine:** a Rust backend (Axum) that handles agents, chat, tools, authentication, the policy engine, and an embedded SurrealDB database with RocksDB storage. The engine spawns sandboxed child processes for tool calls, MCP servers, and apps; it does not spin up containers per agent
- **Frontend:** a statically exported Next.js application, served by the engine, that provides the chat interface, agent management, and workspace UI

External services plug in for specific capabilities:

- **Browserless:** headless Chrome for browser automation
- **SearXNG:** web search
- **Twilio:** voice calls and SMS (optional)

The Frona application runs in one OCI container and works with any OCI-compatible runtime (Docker, Podman, etc.). A typical `docker-compose.yml` runs that container alongside optional supporting services such as Browserless and SearXNG. See the [Kubernetes example](examples/kubernetes) for cluster deployments.

## Documentation

- [Overview](https://docs.frona.ai/platform/overview.html) - what Frona is and how it works
- [Quickstart](https://docs.frona.ai/platform/quickstart.html) - get running with Docker in minutes
- [Comparison](https://docs.frona.ai/platform/comparison.html) - Frona vs. OpenClaw vs. Hermes Agent
- [Agents](https://docs.frona.ai/platform/agents/overview.html) - agent types, configuration, and delegation
- [Channels](https://docs.frona.ai/platform/agents/channels/overview.html) - Telegram, Slack, Discord, WhatsApp, Signal, SMS, pairing, and dispatch modes
- [Memory](https://docs.frona.ai/platform/agents/memory/overview.html) - Basic and PKM memory backends
- [Personal Knowledge Management](https://docs.frona.ai/platform/agents/memory/pkm.html) - grounded, ontology-backed long-term memory
- [Usage and Costs](https://docs.frona.ai/platform/agents/chat/usage.html) - token, cost, latency, and cache dashboards
- [Signals](https://docs.frona.ai/platform/agents/signals.html) - pause-and-resume on inbound messages
- [Skills](https://docs.frona.ai/platform/agents/skills/overview.html) - extend agents with reusable instruction packages
- [Apps](https://docs.frona.ai/platform/agents/apps/overview.html) - let agents build and deploy applications
- [Tools](https://docs.frona.ai/platform/tools/overview.html) - browser, search, CLI, voice, and more
- [MCP](https://docs.frona.ai/platform/tools/mcp/overview.html) - install MCP servers and bridge mode
- [Sandbox](https://docs.frona.ai/platform/security/sandbox.html) - filesystem, network, and resource controls
- [Policies](https://docs.frona.ai/platform/security/policies.html) - policy reference for tools and sandbox rules
- [Credentials](https://docs.frona.ai/platform/credentials/overview.html) - vault integration and approval workflows
- [Deployment](https://docs.frona.ai/platform/deployment/docker-compose.html) - Docker Compose and Kubernetes guides

## Development

All commands use [mise](https://mise.jdx.dev/) as the task runner:

```bash
mise run dev              # Run the backend and frontend on the host
mise run container:dev    # Run the containerized dev stack with hot-reload
mise run container:prod   # Build and run the production container stack

mise run check            # Check all Rust crates
mise run lint             # Check formatting and lint backend + frontend
mise run test             # Run the workspace test suite
```

See [mise.toml](mise.toml) for all available targets.

## License

Frona is licensed under the [Business Source License 1.1](LICENSE). You can use, modify, and self-host it freely. The only restriction is that you may not use it to provide an AI agent platform as a service to third parties. On 2029-02-28, the license converts to Apache 2.0.
