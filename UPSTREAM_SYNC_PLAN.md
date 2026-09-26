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
- `22210fe9` "exclude non-chat usage from top chats" — already present verbatim,
  including the regression test, at `inference_usage.rs:326`.
- `98cc7742` "configure development web search" — covered by fork's own
  `083cd3b`, which explicitly backports it alongside `e3ad1c4`.

## Ported this session (2026-09-25)

Six of Group A's nine commits, verified against `cargo check --lib --tests`,
`npx tsc --noEmit`, `npx vitest run`, and `npx eslint`:

- `058b8ba4` "fall back to the chat model for memory compaction" — adapted to
  the fork's actual `ModelProviderRegistry` API (`get_model_group` returning
  `&ModelGroup`, not upstream's post-Group-C `resolve(&ModelRef)`); added a
  unit test in the new `memory::basic::tests` module covering the
  most-recent-chat / explicit-group-override behavior.
- `ae403078` "require the installed skill directory at construction" — ported
  plus fixed every fork-added call site upstream's commit didn't know about
  (`tool/skills.rs`, three MCP integration tests) on top of the four upstream
  touched.
- `ea7cbb0d` "centralize administrator authorization" — ported as-is
  (self-contained `AdminUser` extractor + `AuthUser::require_admin`); existing
  ad-hoc admin checks (e.g. `api/routes/skills.rs`) were left alone since
  upstream's commit didn't migrate them either.
- `a444b12a` "describe handle constraints in configuration schemas" — ported
  as-is.
- `137de6e2` + `5aee1ead` (settings field label/reset-control fixes) —
  hand-merged onto the fork's already-diverged `field.tsx`/`combobox.tsx`
  (different internal state management, an extra fork-only `disabled` prop
  upstream didn't have yet); ported the accompanying `combobox.test.tsx`
  unchanged.

## Ported this session (2026-09-25, continued)

Two of Group B's ten commits, verified against `cargo fmt --check`,
`cargo clippy --workspace --locked -- -D warnings`, and the relevant unit
tests; a smoke-run of the third confirms it works but it's not wired
anywhere yet:

- `557dd9b6` "isolate config tests from environment overrides" — adapted to
  this fork's flat `core/config.rs` (upstream had already refactored config
  into a `core/config/` submodule with a `ConfigService` struct in a commit
  this fork never had; here it's `Config::load()` /
  `try_build_effective_config()`). Added `*_with_env` variants taking the
  environment as a `HashMap` instead of reading `std::env::vars()` directly;
  pointed the six `FRONA_*` override tests at them instead of mutating
  process-global env vars with `unsafe set_var`/`remove_var`.
- `b3052bd2` "add a low disk Rust integration test runner" — ported
  verbatim as `build/validate-rust-integration.mjs`. Smoke-tested against
  this workspace: disk stayed flat (~3.3–4.5G free) across several targets
  where a normal `cargo test` run on this same workspace has exhausted the
  session container's ~38G writable allowance more than once. Not wired
  into CI or `mise.toml` — upstream's own CI wiring for this script isn't
  visible in-repo (this fork's `.github/workflows/ci.yml` is fork-only;
  upstream has no `.github/` at all), so wiring it in is a fork decision,
  not part of the port.

## Genuine gaps

### Group A — small, independent, low-risk

Six of nine landed this session; see "Ported this session" above. Two
(`22210fe9`, `98cc7742`) were already covered. What's left:

| Commit | Date | What |
|---|---|---|
| `d4186276` | 09-14 | Persist structured message processing errors (chat) |
| `c5e95988` | 09-14 | Display structured failures in chat replies (web) |

**Not actually small — reclassify as its own effort.** Upstream's
`chat/message/error.rs` builds a persistable `MessageError` by pattern-matching
on `AppError::Inference(InferenceError)` (typed) and a new
`AppError::ToolExecution { tool_name, source: Box<AppError> }` variant. Neither
exists in this fork: `AppError::Inference` is a bare `String` here (only 4
call sites construct it), and no tool-execution error carries the tool's name
anywhere in the tool-call path. Upstream's own unit test for this commit also
assumes the post-Group-C `ModelConfig { catalog_provider, provider_handle:
Handle, .. }` shape, which doesn't exist in this fork's provider layer either.

Porting this requires, in order:
1. Change `AppError::Inference(String)` → `AppError::Inference(InferenceError)`
   and rewire the 4 call sites (`api/error.rs`, `inference/tool_loop.rs`,
   `inference/error.rs`, `inference/retry.rs`).
2. Add `AppError::ToolExecution { tool_name, source: Box<AppError> }` and wire
   the tool-call path (`tool/registry.rs`, `agent/task/executor.rs`) to
   construct it instead of flattening tool errors to `AppError::Tool(String)`.
3. Build `chat/message/error.rs`'s `MessageError`/`MessageErrorDetails`
   against the fork's actual `InferenceError` shape — notably
   `AllFallbacksFailed(Vec<(String, String)>)` here vs. upstream's
   `Vec<InferenceError>`, so per-fallback `retry_count`/`http_status` can't be
   reconstructed the same way without a matching upstream-side redesign of
   that variant too.
4. Only then port the persistence wiring (`chat/service.rs`,
   `chat/message/models.rs`, `chat/broadcast.rs`) and the frontend
   (`c5e95988`).

Treat this like Group C: a design decision first, then a port — not a
same-day PR.

### Group B — build/test/CI infrastructure (do when convenient, low urgency)

Ten commits from 09-17 through 09-22. Two landed this session (see "Ported
this session" above: `557dd9b6`, `b3052bd2`). The other eight, by why they
haven't:

**Blocked on Group C — not portable until the provider-adapter rewrite lands:**

| Commit | Date | Why it's blocked |
|---|---|---|
| `385e5dd6` | 09-17 | Dockerfile stage builds and runs `frona-model-catalog` — the crate now exists in this fork (Step 2, see above), but the commit itself is a Docker build stage this sandbox has no Podman/build validation for, same gap as the four Group B commits held back below. |
| `38d4f5a5` | 09-17 | `validate-provider-artifact.mjs` also invokes `frona-model-catalog` — same crate-now-exists-but-unvalidatable-Docker-change situation as `385e5dd6`. |
| `f104bd85` | 09-18 | `managed_cli_feasibility.rs` imports `inference::credential::store::CredentialMethod` and `inference::provider::platform::ProviderPlatform` — Group C's managed-credential vault types. |
| `2b9dfe28` | 09-19 | Edits a Bedrock `provider_workflow.rs` test; Bedrock isn't a fork provider yet (Group C Step 3). |

**Held back — real, but I can't validate them in this sandbox:**

| Commit | Date | What |
|---|---|---|
| `341280b7` | 09-20 | Reorganizes build output into mount-safe paths (`web/out` → `web/target/out`, a new `.dv/workspace.yaml`) across Dockerfile, docker-compose, `container.sh`, `mise.toml`, `next.config.ts`, `tsconfig.json`, `vitest.config.ts`, and the example configs. |
| `047c9920` | 09-20 | Shares dev compiler artifacts through Kache (`build/dev/kache.toml`, `install-kache.sh`), reworks `.cargo/config.toml` and the dev docker-compose profile. |
| `2fcf37c5` | 09-21 | Builds missing Podman images with parallel stages (`build/dev/test-container.py`). |
| `ac847fc9` | 09-22 | Coordinates stopping dev watchers and containers together. |

This fork's `build/container.sh` already does Podman-first runtime detection
matching upstream, and its `docker-compose.yml` volume names still match
upstream's *pre*-Group-B baseline exactly, so these four would likely apply
close to verbatim rather than needing deep reconciliation. But they compound
(each edits the Dockerfile/docker-compose/`container.sh` again on top of the
last) and touch a dev-container tool (`.dv/`) this fork shows no other
evidence of using. This sandbox has `docker` but no `podman` daemon and no
`dv` tool, so none of it can be build-tested here — only read for plausibility.
Land these once someone can actually run `build/container.sh` against the
result before merging, not on read-through confidence alone.

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
   `provider/adapter/*.rs`.
2. **Feature overlap, not just structure.** Upstream's `13aadef9` adds its own
   Azure OpenAI adapter — this fork already has one.
3. **A second, different vault.** This fork already has
   `credential/vault/` (its own provider API-key vault) and `credential/
   share/`. Upstream's new `credential/managed/` is a *different* kind of
   credential — OAuth login sessions for subscription accounts (ChatGPT,
   Copilot, OpenRouter), not API keys.
4. **Bedrock and GitHub Copilot and ChatGPT-subscription are entirely new
   capability**, not currently in the fork at all.

**Step 1 (scoping spike) is done** — see
[`GROUP_C_PROVIDER_SCOPING.md`](GROUP_C_PROVIDER_SCOPING.md) for the full
analysis, built from actually reading both trees rather than the surface-level
concerns above. The short version: concerns 1 and 3 turned out to be much
smaller than they looked. Upstream's `provider/adapter/*` structure is a
generalization of the same rig-based approach this fork already uses — the
"generic" provider *is* upstream's dynamic-adapter mechanism, just hand-rolled
once instead of built as a general path, and five of this fork's six other
additions (byteplus, zai, venice, minimax, llamafile) are one
`Recipe::openai_compatible(FactoryKind::X)` line each once the structure is
adopted, matching eleven recipes upstream already carries the same way.
Concern 3 (the vault) isn't a naming conflict at all: upstream's
`credential/managed/` is a new sibling directory, and this fork's
`credential/vault/`/`credential/share/` are untouched by it. Only concern 2
(Azure) is real reconciliation work — and even there, upstream hit the same
`AzureOpenAIAuth::ApiKey` footgun this fork's code comments describe and fixed
it identically, so it reads as independent convergence rather than two
designs to merge from scratch.

**Steps 2–5**, revised with the scoping doc's findings (see the doc for full
detail):

- **Step 2:** port `frona-model-catalog` extraction, the core `provider/*.rs`
  files, and the adapter files for the six brands upstream and the fork
  already share (azure, cohere, huggingface, hyperbolic, perplexity,
  together). Reconcile Azure per the doc (likely close to a straight take).
  Merge `ModelProviderConfig`'s schema, preserving this fork's `billing`
  field, which upstream's version doesn't have.
- **Step 3:** port the managed-credential vault (`86f9f346` through
  `67a0995c`) and the new-capability adapters (Bedrock, ChatGPT subscription,
  GitHub Copilot) — additive, no fork equivalent to reconcile, confirmed no
  naming collision with the fork's existing vault.
- **Step 4:** add five one-line recipe entries (byteplus, zai, venice,
  minimax, llamafile) and drop the fork's `generic` special case in favor of
  upstream's dynamic-adapter path — mechanical, not a redesign.
- **Step 5:** port the settings-UI commits (`a908db4c`, `99db1d24`,
  `ee6ab0b1`, `7e745ac4`) last, since they're written against the finished
  structure and touch the same settings page the fork's voice/cost-analyst
  settings live in.

This is still materially larger than any prior upstream-port PR in this
fork's history (PR #112/#113 together were ~5,400 lines across 15 commits;
Group C alone is ~20 commits with one single commit at +15,858/-3,721) and
should stay its own multi-PR effort, tracked separately from Groups A and B.
Steps 2, 3, and 5 are still substantial; what shrank is Step 4 and the
perceived size of the Step 1 decision itself.

## Suggested order

1. ~~Group A (small independent fixes)~~ — done except `d4186276`/`c5e95988`,
   reclassified above as its own effort.
2. ~~Group B, the two portable commits~~ — done (`557dd9b6`, `b3052bd2`).
3. ~~Group C Step 1 (scoping spike)~~ — done, see
   [`GROUP_C_PROVIDER_SCOPING.md`](GROUP_C_PROVIDER_SCOPING.md). Unblocks
   Group C Steps 2–5 below and Group B's four Group-C-dependent commits
   (`385e5dd6`, `38d4f5a5`, `f104bd85`, `2b9dfe28`).
4. Group C Steps 2–5 — the multi-PR provider/credential rewrite, revised order
   and detail in the scoping doc. Step 2's catalog-crate half is done (the
   `frona-model-catalog` crate is vendored in and `inference/metadata`
   rebuilt on it); its remaining half — the six shared-brand adapter files
   and the `ModelProviderConfig` schema merge — and Steps 3–5 are still
   open, per the scoping doc's "Step 2 progress" note.
5. The `AppError` redesign for `d4186276`/`c5e95988` — independent of Group C,
   can happen in parallel.
6. Group B's remaining four Podman/Kache dev-container commits (`341280b7`,
   `047c9920`, `2fcf37c5`, `ac847fc9`) — whenever someone has a Podman/`dv`
   environment to actually build-test them in; not blocked by anything else,
   but shouldn't land on read-through confidence alone.
