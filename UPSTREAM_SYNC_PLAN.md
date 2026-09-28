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

Eight of nine landed. Two (`22210fe9`, `98cc7742`) were already covered; six
more landed in an earlier session (see "Ported this session" above); the
final two landed 2026-09-28, corrected from this section's earlier
"reclassify as its own effort" call:

| Commit | Date | What |
|---|---|---|
| ~~`d4186276`~~ | 09-14 | Persist structured message processing errors (chat) — fork commit `16cd9c2` |
| ~~`c5e95988`~~ | 09-14 | Display structured failures in chat replies (web) — fork commit `2c8f892` |

**Correction (2026-09-28):** the blocker this section described — this fork's
`InferenceError::AllFallbacksFailed(Vec<(String, String)>)` vs. upstream's
`Vec<InferenceError>` — no longer held by the time this was actually
attempted. By this date this fork's `InferenceError` already carried
`AllFallbacksFailed(Vec<InferenceError>)` and `ModelFailed { provider, model,
retry_count, source }` (apparently picked up as part of Group C's provider
rework, independent of upstream's own redesign), and `ModelConfig` already had
the exact `catalog_provider`/`provider_handle: Handle`/`model_id`/`provider`/
`request_settings` shape upstream's unit test assumed. Re-scoping found the
port was tractable as-is: `AppError::Inference(String)` → `Inference(InferenceError)`
was still a real prerequisite step (3 call sites, not 4 — `inference/retry.rs`
never constructed it), but `chat/message/error.rs` ported essentially verbatim
once that one variant-type change landed. See `16cd9c2`'s and `2c8f892`'s
commit messages for the full port detail, including one real gap the upstream
diff didn't cover: a fork-only test (`tests/api/security.rs`) constructing
`AppError::Inference` directly, caught by `cargo check --tests` and fixed
alongside.

### Group B — build/test/CI infrastructure (do when convenient, low urgency)

Ten commits from 09-17 through 09-22. Two landed this session (see "Ported
this session" above: `557dd9b6`, `b3052bd2`). The other eight, by why they
haven't:

**Blocked on Group C — not portable until the provider-adapter rewrite lands:**

| Commit | Date | Why it's blocked |
|---|---|---|
| `385e5dd6` | 09-17 | Dockerfile stage builds and runs `frona-model-catalog` — the crate now exists in this fork (Step 2, see above), but the commit itself is a Docker build stage this sandbox has no Podman/build validation for, same gap as the four Group B commits held back below. |
| `38d4f5a5` | 09-17 | `validate-provider-artifact.mjs` also invokes `frona-model-catalog` — same crate-now-exists-but-unvalidatable-Docker-change situation as `385e5dd6`. |
| ~~`f104bd85`~~ | 09-18 | Was blocked on `inference::credential::store::CredentialMethod`/`inference::provider::platform::ProviderPlatform`, both of which exist now — unblocked and in progress as of 2026-09-28 (see below). |
| `2b9dfe28` | 09-19 | Still blocked, for a sharper reason than "Bedrock isn't a fork provider yet": Bedrock *is* now a recognized brand in `provider/platform.rs` (recipe, protocols, auth methods, attribute validation all wired), but its actual adapter `build()` deliberately returns `Err(InferenceError::ConfigError("Provider 'bedrock' is not yet available in this build"))` (`platform.rs:772`) — scaffolded, not implemented. This commit's test also assumes a `crates/frona-server/tests/provider_workflow.rs` integration-test file this fork doesn't have at all. Porting it requires first building a real Bedrock adapter (AWS SDK integration, live foundation-model discovery) — a standalone effort, not covered by this plan's existing scope. |

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

**Status as of 2026-09-28: Steps 2–4 done.** PR #131 ("Group C:
managed-credential vault + provider-adapter rewrite (Steps 2-3, unified)")
landed the full `inference/provider/{mod,registry,group,platform,service,
validation}.rs` + `provider/adapter/*.rs` structure (Step 2, including the
six shared-brand adapters and the `ModelProviderConfig` schema merge with
`billing` preserved), `credential/managed/*` (Step 3, no naming collision
confirmed), and the five one-line `byteplus`/`zai`/`venice`/`minimax`/
`llamafile` recipe entries plus the `generic` case dropped in favor of
`dynamic_recipe()` (Step 4) — all verified present in
`crates/frona-server/src/inference/provider/platform.rs` and
`credential/managed/`. This document and `GROUP_C_PROVIDER_SCOPING.md`
weren't updated when that PR merged; they still read as if only the
catalog-crate half of Step 2 had landed. Corrected here.

**One caveat found on closer inspection (2026-09-28):** Step 3's "new-capability
adapters" line names Bedrock alongside ChatGPT-subscription and GitHub Copilot,
but only the latter two actually got a working adapter
(`provider/adapter/{chatgpt,copilot}.rs` exist; there is no `bedrock.rs`).
Bedrock is a recognized brand in `platform.rs` — recipe, protocols
(`ApiSurface::AmazonBedrockConverse`), auth methods, and attribute validation
(`aws_profile`/`aws_region`) are all wired — but its adapter `build()` is a
deliberate stub: `Err(InferenceError::ConfigError("Provider 'bedrock' is not
yet available in this build"))` (`platform.rs:772`). So "Bedrock ... entirely
new capability" (this document's Group C intro, below) is still true in
practice; only the scaffolding landed. See `2b9dfe28`'s corrected entry in the
Group B table above — that commit, and real Bedrock support generally, needs
its own effort (AWS SDK integration, live foundation-model discovery), not
covered by anything currently planned here.

**Step 5 (settings UI) is now done**, all four commits ported:

- `99db1d24` ("manage account logins from vault settings" — the "Managed"
  vault-provider option in `vault-section.tsx`, wired to
  `VaultConnectionConfig::Managed{}`) — ported before PR #131 merged (fork
  commit `f30bac1`).
- `a908db4c` ("configure named provider connections in settings" — fork
  commit `fb47936`) — `web/src/lib/provider-admin.ts` and `provider-drafts.ts`
  are new; `providers-section.tsx` is catalog-driven (add/edit/validate/
  accept/login flows against `/api/config/providers/{handle}/...`) rather
  than editing the old flat `providers: Record<string, ModelProviderConfig>`
  shape directly; `config-types.ts`/`api-client.ts` carry the
  `persisted_revision`-aware `updateConfig`/`getConfigDocument` the commit
  depends on. Adapted from upstream where the fork's actual Rust types
  required it: `GET /api/config` didn't return a `persisted_revision` at all
  (needed for the provider edit/delete routes' mandatory
  `expected_persisted_revision`) — added via `ConfigService::active_revision()`
  plus a small change to `api/routes/config.rs::get_config`, matching the
  shape `PUT /api/config` already returned; `PUT /api/config` requires the
  `{patch, expected_persisted_revision}` envelope unconditionally (no
  bare-patch fallback, unlike upstream's own backend); this fork's
  `ModelProviderConfig` already has `billing`, `credential_id`, `provider`,
  `adapter`, `aws_*`, and a flattened `attributes` bag, all preserved and
  round-tripped through the new UI's `BillingFields`/generic field renderer.
- `ee6ab0b1` + `7e745ac4` ("configure model groups from live provider
  directories" + "edit custom request parameters in a separate accordion" —
  fork commit `68e0cfe`, one commit for both since the second is a direct
  continuation of the first's new file) — `models-section.tsx`/
  `model-selector.tsx` rewritten onto the live per-connection directories
  (`GET`/`POST /api/config/providers/{handle}/models`); new
  `model-authoring.ts` (schema-lite validation, patch diffing) and
  `use-model-directories.ts` (per-connection directory cache); new
  `model-settings.tsx` for the schema-driven typed/extra-params split,
  including the second commit's "Custom request parameters" accordion.
  Preserved fork-only features with no upstream equivalent: OpenRouter's
  `route`/`provider_routing`/`prompt_caching` fields (confirmed via
  `inference/protocol/parameters.rs` that the backend's live
  `ModelSettingInfo` directory can't describe these — `OpenRouterParams`
  wraps its base params via `#[serde(flatten)]`, which the `ParameterMetadata`
  derive doesn't support — so they'd have silently disappeared without manual
  preservation, the same class of gap `billing` was in `a908db4c`), and this
  fork's permissive (not lowercase-only) group-rename validation, since
  `models: HashMap<String, ModelGroupConfig>` has no `Handle`-style casing
  constraint server-side.

All three ported commits' verification: `npx tsc --noEmit` clean, `npx eslint`
clean, full `npx vitest run` passing throughout (503 → 546 tests as each
commit added its own), `cargo check --workspace` clean. `cargo test` was
deliberately not run for these — see Group B's disk-allowance note below;
`cargo check`/`clippy -D warnings`/`fmt --check` is the verification ceiling
used here for Rust-touching frontend work in this sandbox.

This was materially larger than any prior upstream-port PR in this fork's
history (PR #112/#113 together were ~5,400 lines across 15 commits; Group C
alone is ~20 commits with one single commit at +15,858/-3,721), landed across
several PRs as its own multi-PR effort rather than one, per the original plan.
With Step 5 done, Group C's full scope (Steps 1–5) is closed.

## Suggested order

1. ~~Group A~~ — done. All nine commits landed, including `d4186276`/`c5e95988`
   (fork commits `16cd9c2`/`2c8f892`, 2026-09-28), which this document
   previously reclassified as needing a design decision first — see the
   correction in Group A's section above. **Group A is complete.**
2. ~~Group B, the two portable commits~~ — done (`557dd9b6`, `b3052bd2`).
3. ~~Group C Step 1 (scoping spike)~~ — done, see
   [`GROUP_C_PROVIDER_SCOPING.md`](GROUP_C_PROVIDER_SCOPING.md). Unblocks
   Group C Steps 2–5 below and Group B's four Group-C-dependent commits
   (`385e5dd6`, `38d4f5a5`, `f104bd85`, `2b9dfe28`).
4. ~~Group C Steps 2–5~~ — done. Steps 2–4 landed in PR #131; Step 5's four
   commits landed as `f30bac1`, `fb47936`, and `68e0cfe` (see "Status as of
   2026-09-28" above). **Group C is complete.**
5. ~~`f104bd85`~~ — done, fork commit `afda223` (2026-09-28): unblocked once
   Group C landed the `credential::managed`/`ProviderPlatform` types it
   imports; ported verbatim, all 6 tests pass.
6. Group B's three remaining Group-C-dependent commits (`385e5dd6`,
   `38d4f5a5`, `2b9dfe28`) — still blocked. The first two need Podman/Docker
   build validation this sandbox can't do (no daemon: `docker info` fails to
   connect). `2b9dfe28` needs a real Bedrock adapter built first — see the
   caveat in the Group C status note above; Bedrock's `build()` is still a
   deliberate stub, so this is its own effort, not just an unblock.
7. Group B's remaining four Podman/Kache dev-container commits (`341280b7`,
   `047c9920`, `2fcf37c5`, `ac847fc9`) — whenever someone has a Podman/`dv`
   environment to actually build-test them in; not blocked by anything else,
   but shouldn't land on read-through confidence alone.
