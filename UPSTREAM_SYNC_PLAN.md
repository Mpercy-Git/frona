# Upstream sync plan — `fronalabs/frona`

Status as of **2026-09-24**. This fork tracks upstream by *porting content*, not by
merging history (`main` and `upstream/main` share no common ancestor — see prior
upstream PRs #104, #111, #112, #113). So "how far behind" is not a `git log
a..b` count; it's which upstream commits' changes are, and aren't, present in
this tree. This document is that audit, plus a proposed order of work to close
the gap while keeping this fork's own features intact.

Upstream remote used for this audit: `https://github.com/fronalabs/frona`
(`main` @ `ac847fc9`, 2026-09-22). Baseline: the last commit a prior port PR
confirmed applied, `04c59d0` / fork commit `247da2d` (PR #113, merged
2026-09-18). 66 upstream commits exist after that point; this plan accounts
for all 66.

## Already covered — no action

Confirmed by finding a matching fork-side commit with the same content, not
just by date:

- The four SSE/task-stream fixes from Sep 3–4 (`0107ad0d`, `ec3772b7`,
  `05976267`, `e2055115`) — ported as `27afd8b9`, `7e651751`, `05976267`(fork),
  `c8e6970e` in PR #104.
- `18c591f1` "clarify task result continuation" — ported as `9e9b003`.
- `88641e05` "repair JSON text in structured submissions" — covered by
  `a857804c` (PR #104's title calls out fixing this exact area; worth a
  one-commit diff check before closing, but not a gap in behavior).
- The seven-commit memory/PKM cluster and the eight tool-view/activity/files
  commits from PR #112 and #113 (`b537c83`, `63297a5`, `0eedfca`, `d7761c1`,
  `652c3e0`, `7cd838e`, `07c3247`, `3bf1277`, `cfe0665`, `c89e25f`, `8e0b37d`,
  `9811c71`, `2dac0a9`, `6960ac9`, `04c59d0`).
- `f69cf35f` "release: v2026.9.0" — a version tag commit, nothing to port.

## Genuine gaps

### Group A — small, independent, low-risk (do first)

No structural conflict with fork-specific code. Straightforward content ports,
same shape as PRs #104/#111/#112.

| Commit | Date | What |
|---|---|---|
| `22210fe9` | 09-02 | Exclude non-chat usage from "top chats" — usage-accounting bug fix |
| `98cc7742` | 09-03 | Configure development web search — **check against fork's own `083cd3b` (disable Next telemetry, bind-mount dev SearXNG) first; may already be superseded by a fork-specific approach, in which case skip** |
| `d4186276` | 09-14 | Persist structured message processing errors (chat) |
| `c5e95988` | 09-14 | Display structured failures in chat replies (web) |
| `058b8ba4` | 09-14 | Fall back to the chat model for memory compaction |
| `137de6e2` | 09-15 | Display labels consistently in settings controls |
| `5aee1ead` | 09-15 | Inline reset controls on settings inputs |
| `ae403078` | 09-06 | Require the installed skill directory at construction |
| `ea7cbb0d` | 09-06 | Centralize administrator authorization (33-line, self-contained) |
| `a444b12a` | 09-05 | Describe handle constraints in configuration schemas |

Recommend one PR, same style as prior ports: content-diff each commit against
this tree, port what's missing, call out anything that touches fork-diverged
files (settings UI in particular has the fork's own voice-settings and
cost-analyst additions nearby — check for adjacency conflicts, not logic
conflicts).

### Group B — build/test/CI infrastructure (do when convenient, low urgency)

All ten remaining commits from 09-17 through 09-22: `385e5dd6`, `38d4f5a5`,
`b3052bd2`, `f104bd85`, `557dd9b6`, `2b9dfe28`, `341280b7`, `047c9920`,
`2fcf37c5`, `ac847fc9`. These are upstream's own dev-container/CI tooling
(Podman parallel builds, Kache-shared compiler artifacts, a low-disk test
runner, environment-isolated config tests). None of it is user-facing product
behavior. Worth adopting for maintainability, but this fork's CI/dev
environment (ghcr.io images, its own container workflow) has already diverged
some — each commit needs a quick read for whether it assumes upstream's
specific CI wiring before blindly applying. Not time-sensitive; batch these
into their own PR separate from Group A so a build-tooling review doesn't
block behavior fixes.

### Group C — the big one: managed credentials, vault, and a provider-adapter rewrite (needs a design decision, not just a port)

This is the substantial finding of this audit. Between 09-05 and 09-16,
upstream landed a **~20-commit, architecture-level rewrite** of exactly the
subsystem this fork has invested the most unique work in (see README's "Fork
Enhancements": Azure, generic OpenAI-compatible, Z.ai/Venice/MiniMax,
llamafile, BytePlus, OpenRouter routing/caching/cost accounting, the
cost-analyst agent). Commits:

`fcd3d16f`, `b31f3382` (new `frona-model-catalog` crate + derived model
metadata) → `86f9f346`, `5836a602`, `a1c35f0a`, `9f74cd9b`, `62026d73`,
`610d1e3c`, `67a0995c` (a **managed-credential vault**: encrypted storage,
interactive OAuth-style login flows, and key rotation, for ChatGPT
subscription, GitHub Copilot, and OpenRouter accounts) → `13aadef9`,
`35d82d28`, `645a383e`, `3268c03e`, `0f9d74bf`, `76f52a0b`, `92efd2bf`,
`ff4f9c8f`, `3f510307` (provider logic split into a new
`inference/provider/adapter/{azure,bedrock,chatgpt,copilot,cohere,together,
hyperbolic,huggingface,perplexity}.rs` structure — one file per provider,
replacing a flat module) → `7c1392f9` (**119 files, +15858/-3721** — the
adapter split lands everywhere: "named provider connections" and model
groups resolve through the new structure) → `915dc5bb`, `9a352152`,
`74f28d06`, `d3bf621e`, `f13693f2` (vault wiring into app state, provider
validation/credential-admin APIs) → `a908db4c`, `99db1d24`, `ee6ab0b1`,
`7e745ac4` (settings UI: connect named providers, manage vault logins,
configure model groups from live provider directories).

**Why this needs a decision before any porting starts:**

1. **Structural collision.** This fork's provider code is one flat
   `crates/frona-server/src/inference/provider.rs` with the fork's own
   `azure`, `generic`, `byteplus`, `zai`, `venice`, `minimax`, `llamafile`
   entries inline in it. Upstream deleted that shape entirely in favor of
   `provider/adapter/*.rs`. There is no clean line-level merge here — every
   fork-added provider has to be re-homed into the new structure, or the new
   structure has to be declined and its content ported into the old one by
   hand. Either choice is a multi-file rewrite, not a port.
2. **Feature overlap, not just structure.** Upstream's `13aadef9` adds its own
   Azure OpenAI adapter — this fork already has one (README: "☁️ Azure OpenAI
   (net-new)"), built independently and for reasons upstream's commit message
   likely doesn't share (this fork's own `api_version`/deployment-name
   handling, the `Into<String>` api-key-vs-bearer-token fix). These two Azure
   implementations need a real reconciliation, not a fast-forward.
3. **A second, different vault.** This fork already has
   `credential/vault/` (its own provider API-key vault) and `credential/
   share/`. Upstream's new `credential/managed/` is a *different* kind of
   credential — OAuth login sessions for subscription accounts (ChatGPT,
   Copilot, OpenRouter), not API keys. These can coexist, but the naming and
   the fork's existing vault UI need a coherent story before the settings-UI
   commits (Group C's tail) can land without confusing the two.
4. **Bedrock and GitHub Copilot and ChatGPT-subscription are entirely new
   capability**, not currently in the fork at all. Porting them is additive
   and lower-risk than the Azure overlap — they just need the adapter
   scaffolding decision made first, since they're written against it.

**Recommended approach**, mirroring how PR #112 scoped the memory/PKM cluster
before #113 executed it:

- **Step 1 (scoping PR/spike, no behavior change):** decide whether to adopt
  upstream's `provider/adapter/*` module layout wholesale (probably right —
  fighting a 119-file upstream refactor forever is not sustainable) and write
  down, provider by provider, which of the fork's seven added providers
  (azure, generic, byteplus, zai, venice, minimax, llamafile) map onto an
  upstream adapter that now exists (azure, bedrock, chatgpt, copilot, cohere,
  together, hyperbolic, huggingface, perplexity) vs. which stay fork-only
  (generic-openai-compatible, byteplus, zai, venice, minimax, llamafile have
  no upstream equivalent yet and become new files in the adopted structure).
- **Step 2:** port `frona-model-catalog` extraction and the adapter-split
  commits for the providers upstream and the fork have *in common*
  (`13aadef9` Azure being the one requiring hand reconciliation with the
  fork's existing Azure code, not a straight take).
- **Step 3:** port the managed-credential vault (`86f9f346` through
  `67a0995c`) and the new-capability adapters (Bedrock, ChatGPT subscription,
  GitHub Copilot) — additive, no fork equivalent to reconcile.
- **Step 4:** re-home the fork's own providers (generic, byteplus, zai,
  venice, minimax, llamafile) into the adopted adapter structure — mechanical
  once Step 1's mapping exists.
- **Step 5:** port the settings-UI commits (`a908db4c`, `99db1d24`,
  `ee6ab0b1`, `7e745ac4`) last, since they're written against the finished
  structure and touch the same settings page the fork's voice/cost-analyst
  settings live in.

This is materially larger than any prior upstream-port PR in this fork's
history (PR #112/#113 together were ~5,400 lines across 15 commits; Group C
alone is ~20 commits with one single commit at +15,858/-3,721). It should be
its own multi-PR effort, tracked separately from Groups A and B, and is the
one place in this sync where "port everything" is the wrong instinct —
scoping which upstream pieces are genuinely new capability vs. which
reimplement something this fork already shipped independently is the load-
bearing decision.

## Suggested order

1. Group A (small independent fixes) — one PR, low risk, fast.
2. Group C Step 1 (scoping doc/spike only) — establishes the provider mapping
   before any more upstream provider work lands and the gap widens further.
3. Group C Steps 2–5 — the multi-PR provider/credential rewrite, in the order
   above.
4. Group B (build/CI infra) — whenever convenient; doesn't block anything
   else and nothing else blocks it.
