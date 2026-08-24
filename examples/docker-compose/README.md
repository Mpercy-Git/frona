# Frona — Docker Compose Deployment

A ready-to-use Docker Compose setup for running Frona with browser automation and web search.

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) (with Compose v2)

## Quick Start

```bash
# 1. Copy the example environment file
cp env.example .env

# 2. Edit .env — set your encryption secret and at least one LLM API key
#    FRONA_AUTH_ENCRYPTION_SECRET=<random-secret>
#    ANTHROPIC_API_KEY=sk-ant-...

# 3. Start all services
docker compose up -d

# 4. Open Frona
open http://localhost:3001
```

## Services

| Service | Description | Port |
|---|---|---|
| **frona** | Frona server | `3001` (host) |
| **browserless** | Headless Chromium for browser automation | internal only |
| **searxng** | Meta search engine for web search | internal only |

## Configuration

- **`.env`** — API keys and secrets (required)
- **`config.yaml`** — Frona settings: model groups, providers, server options (optional — defaults work out of the box). Copy it to `data/config.yaml`, where the server reads it from
- **`config.openrouter.yaml`** — an alternative `config.yaml` that runs every built-in agent through OpenRouter (see below)
- **SearXNG** — Search engine settings are defined inline in `docker-compose.yml` via Docker Compose configs

### Running everything through OpenRouter

`config.openrouter.yaml` is a drop-in replacement for `config.yaml` that needs
one API key instead of several. It assigns each built-in agent a model chosen
for cost against capability, and points the background utilities (chat titles,
compaction, image transcription, call summaries) at cheaper models than the
agents themselves:

| Group | Used by | Model |
|---|---|---|
| `primary` | Assistant | `google/gemini-3.7-flash` |
| `coding` | Developer | `z-ai/glm-5.2` |
| `reasoning` | Researcher, Receptionist | `z-ai/glm-5.3` |
| `title`, `call_summary` | background | `deepseek/deepseek-v4-flash` |
| `compaction`, `vision` | background | `google/gemini-3.7-flash` |

```bash
mkdir -p data && cp config.openrouter.yaml data/config.yaml
echo "OPENROUTER_API_KEY=sk-or-..." >> .env
docker compose up -d
```

The server reads its config from `data/config.yaml` (override the path with
`FRONA_CONFIG`), which is why both files are copied there rather than edited in
place.

The file documents what each choice trades off and lists a cheaper and a more
capable substitute for every group, so you can move one agent up or down
without touching the rest.

## Data

All persistent data is stored in `./data/`:

- `data/db/` — Database
- `data/workspaces/` — Agent workspaces
- `data/files/` — Uploaded files
- `data/skills/` — Installed skills
- `data/browser_profiles/` — Browser automation profiles

## Updating

```bash
docker compose pull
docker compose up -d
```
