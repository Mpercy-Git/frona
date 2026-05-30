<p align="center">
    <a href="https://docs.frona.ai/" target="_blank">
        <img width="300" src="https://docs.frona.ai/logo-light.svg" alt="Frona AI">
    </a>
</p>

Frona is a personal AI assistant. You create autonomous agents that browse the web, run code, build applications, make phone calls, delegate work to each other, and remember context across conversations, all within sandboxed environments with controlled access to your files, network, and credentials. You give them a task and they figure out how to get it done.

You deploy Frona on your own infrastructure and keep full control of your data. The platform is built from the ground up with security in mind, and the engine is written in Rust. So it's fast, lightweight, and runs everything in a single process.

## Security First

AI agents are powerful. They can execute code, browse websites, and access your data. No platform can make LLMs perfectly safe. They will make mistakes. The goal is to isolate those mistakes and reduce the blast radius when they happen.

- **Sandboxed execution:** when agents run shell commands, they execute inside a sandbox that restricts filesystem access to the agent's workspace directory, controls network access per agent, and enforces execution timeouts. An agent can't read files outside its workspace or make network calls you didn't allow
- **Agent isolation:** each agent gets its own set of tools, its own workspace directory, and its own credentials. Create narrow, purpose-built agents instead of one agent that can do everything
- **Isolated browser sessions:** each user gets separate browser profiles. Different credentials get separate browser states. One user's cookies and sessions are never visible to another
- **Credential vault:** agents request credentials when they need them, and you approve or deny in real time. Supports 1Password, Bitwarden, HashiCorp Vault, KeePass, and Keeper. Secrets are never stored in agent memory or sent to LLM providers
- **Self-hosted by design:** your data lives on your servers. You choose which LLM provider to use, and traffic goes directly from your instance to that provider

## Features

- **Autonomous agents with tools:** agents decide which tools to use and execute multi-step tasks on their own. Agents can also build their own tools
- **Browser automation:** headless Chrome via Browserless for navigating websites, filling forms, and extracting data. Persistent browser profiles keep sessions across conversations
- **Web search:** built-in search via SearXNG, Tavily, or Brave Search
- **Code execution:** sandboxed shell commands with filesystem, network, and resource restrictions per agent
- **App deployment:** agents build and deploy web applications and services on your behalf, with an approval workflow before anything goes live
- **Skills:** instruction packages that teach agents new capabilities. Install shared skills or create agent-specific ones
- **Scheduling:** recurring tasks via cron expressions and agent-managed heartbeat checklists
- **Voice calls:** outbound phone calls via Twilio with speech recognition and DTMF navigation (optional)
- **Persistent memory:** agents remember facts across conversations with automatic compaction and deduplication. User-scoped facts are shared across agents, agent-scoped facts are private
- **Agent-to-agent delegation:** agents hand off tasks to specialized agents and get results back
- **Spaces:** group conversations that share context. The platform summarizes linked conversations and feeds the context into new chats
- **Real-time streaming:** token-by-token response streaming over Server-Sent Events
- **SSO:** OpenID Connect support for single sign-on with Google, Keycloak, and other OIDC providers
- **Single-container deployment:** the entire backend (API server, embedded database, scheduler, tool execution) runs in one rootless OCI container (compatible with Docker, Podman, and other OCI runtimes)

## Core Concepts

- **Agents** are the main building blocks. Each agent has a name, a system prompt that defines its behavior, a model group that determines which LLM it uses, and a list of tools it can access. Frona ships with built-in agents (Assistant, Researcher, Developer, Receptionist) and you can create your own.
- **Memory** lets agents remember things across conversations. There are user-scoped facts (shared across all agents) and agent-scoped facts (private to one agent). The platform automatically compacts and deduplicates memories over time.
- **Tools** are capabilities you give to agents. Browser automation, web search, file operations, shell commands, voice calls, task scheduling, and more. Tools run server-side and return results to the agent.
- **Tasks** represent units of work. They can be direct (run immediately), delegated (from one agent to another), or scheduled (recurring via cron expressions).
- **Chat** is how you interact with agents. Each conversation belongs to one agent, but multiple agents can contribute to it through delegation. Messages stream in real-time over Server-Sent Events.
- **Spaces** are groups of chats that share the same context. When you link conversations to a space, the platform summarizes those conversations and feeds the context back into new chats.
- **Skills** are instruction packages you install on agents. They can be built-in, shared across all agents, or scoped to a single agent.

## Quickstart

You'll need an OCI runtime with Compose v2 support, such as [Docker](https://docs.docker.com/get-docker/) or [Podman](https://podman.io/).

```yaml
# docker-compose.yml
services:
  frona:
    image: ghcr.io/fronalabs/frona:latest
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
    image: ghcr.io/browserless/chromium:latest
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

## Model Providers

Frona connects to any of the following LLM providers. Set the corresponding API key in your env and the provider is auto-discovered. Configure model groups (primary, coding, reasoning) to route different tasks to different models.

| Provider | Environment Variable |
|---|---|
| Anthropic | `ANTHROPIC_API_KEY` |
| OpenAI | `OPENAI_API_KEY` |
| Google Gemini | `GEMINI_API_KEY` |
| DeepSeek | `DEEPSEEK_API_KEY` |
| Mistral | `MISTRAL_API_KEY` |
| Cohere | `COHERE_API_KEY` |
| xAI (Grok) | `XAI_API_KEY` |
| Groq | `GROQ_API_KEY` |
| OpenRouter | `OPENROUTER_API_KEY` |
| Together | `TOGETHER_API_KEY` |
| Perplexity | `PERPLEXITY_API_KEY` |
| Hyperbolic | `HYPERBOLIC_API_KEY` |
| Moonshot | `MOONSHOT_API_KEY` |
| Hugging Face | `HUGGINGFACE_API_KEY` |
| Mira | `MIRA_API_KEY` |
| Galadriel | `GALADRIEL_API_KEY` |
| Ollama (local) | `OLLAMA_API_BASE_URL` |

## Search Providers

Agents can search the web using any of the following providers. Set `FRONA_SEARCH_PROVIDER` or let Frona auto-detect from available API keys.

| Provider | Environment Variable |
|---|---|
| SearXNG (self-hosted) | `FRONA_SEARCH_SEARXNG_BASE_URL` |
| Tavily | `TAVILY_API_KEY` |
| Brave Search | `BRAVE_API_KEY` |

## Voice Providers

Agents can make and receive phone calls. Set `FRONA_VOICE_PROVIDER` or let Frona auto-detect from available credentials.

| Provider | Environment Variables |
|---|---|
| Twilio | `FRONA_VOICE_TWILIO_ACCOUNT_SID`, `FRONA_VOICE_TWILIO_AUTH_TOKEN`, `FRONA_VOICE_TWILIO_FROM_NUMBER` |

## Architecture

Frona has two main components:

- **Engine:** a Rust backend (Axum) that handles agents, chat, tools, authentication, and an embedded SurrealDB database with RocksDB storage
- **Frontend:** a Next.js application that provides the chat interface, agent management, and workspace UI

External services plug in for specific capabilities:

- **Browserless:** headless Chrome for browser automation
- **SearXNG:** web search
- **Twilio:** voice calls (optional)

Everything runs in OCI containers and works with any OCI-compatible runtime (Docker, Podman, etc.). A typical deployment is a single `docker-compose.yml` that brings up the engine, frontend, and supporting services. See the [Kubernetes example](examples/kubernetes) for cluster deployments.

## Documentation

- [Overview](https://docs.frona.ai/platform/overview.html) — what Frona is and how it works
- [Quickstart](https://docs.frona.ai/platform/quickstart.html) — get running with Docker in minutes
- [Agents](https://docs.frona.ai/platform/agents/overview.html) — agent types, configuration, and delegation
- [Tools](https://docs.frona.ai/platform/tools/overview.html) — browser, search, CLI, voice, and more
- [Sandbox](https://docs.frona.ai/platform/security/sandbox.html) — filesystem, network, and resource controls
- [Credentials](https://docs.frona.ai/platform/credentials/overview.html) — vault integration and approval workflows
- [Deployment](https://docs.frona.ai/platform/deployment/docker-compose.html) — Docker Compose and Kubernetes guides
- [Configuration](https://docs.frona.ai/platform/deployment/config-file.html) — full config file and environment variable reference

## Development

All commands use [mise](https://mise.jdx.dev/) as the task runner:

```bash
mise run docker:dev       # Run full dev stack in Docker with hot-reload
mise run docker:prod      # Run production stack in Docker
```

See [mise.toml](mise.toml) for all available targets.

## License

Frona is licensed under the [Business Source License 1.1](LICENSE). You can use, modify, and self-host it freely. The only restriction is that you may not use it to provide an AI agent platform as a service to third parties. On 2029-02-28, the license converts to Apache 2.0.
