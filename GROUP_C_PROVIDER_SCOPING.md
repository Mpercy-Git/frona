# Group C Step 1 — provider/adapter scoping

Status as of **2026-09-26**. This is the scoping spike `UPSTREAM_SYNC_PLAN.md`
calls for before any of Group C's ~20-commit provider/credential rewrite gets
ported: a decision on whether to adopt upstream's `provider/adapter/*` module
layout, and a concrete mapping of this fork's own provider additions onto it.
No code changes here — this is the design decision Steps 2–5 execute against.

Upstream tree inspected: `fronalabs/frona` @ `ac847fc9` (2026-09-22), via the
local clone at `/home/user/fronalabs/frona`.

## Decision: adopt upstream's structure

Adopt `provider/{mod,registry,group,platform,service,validation}.rs` +
`provider/adapter/*.rs`, replacing this fork's single flat
`inference/provider.rs` (1,571 lines) and the provider-dispatch match block in
`inference/registry.rs`. Reasoning, from actually reading both trees rather
than guessing:

- **Both trees are already built on the same foundation.** This fork and
  upstream both dispatch most providers through `rig_core::providers::*`
  client builders wrapped in a `RigProvider<C>` (this fork) /
  `RigProvider<C>` (upstream, same name, same idea). Adopting upstream's
  structure is not a foreign import — it is largely the same rig-based
  approach this fork already uses, reorganized.
- **Upstream generalized exactly the pattern this fork hand-rolled per
  provider.** This fork's `generic` provider (`inference/registry.rs:332`) is
  "unknown brand, no rig adapter, point rig's `openai::CompletionsClient` at
  an arbitrary `base_url`". Upstream's `platform.rs::dynamic_recipe()` is the
  *same* mechanism made generic: any config declaring `adapter: openai` (an
  `AdapterId` enum) plus a `base_url` gets `FactoryKind::GenericOpenAi`
  automatically — no bespoke match arm needed. Fighting this fork's own
  hand-rolled version of what upstream now does generically is wasted effort.
- **Every one of this fork's "custom" providers turns out to already fit
  upstream's `openai_compatible()` recipe helper.** See the mapping below —
  none of them need new adapter *code*, only new recipe *entries* (a few
  lines each, the same shape as upstream's existing groq/openrouter/deepseek/
  mistral/perplexity/togetherai/xai/hyperbolic/moonshotai/mira/galadriel
  entries).

## Provider-by-provider mapping

This fork dispatches 23 provider names through one `init_provider()` match in
`inference/registry.rs`. Upstream's `platform.rs::built_in_brands()` compiles
in 21 recipes. Cross-referencing them:

| This fork's provider | Upstream brand? | What upstream already has | Action needed |
|---|---|---|---|
| `anthropic` | ✅ `anthropic` | `AdapterId::Anthropic`, dedicated recipe | None — direct rename/adopt |
| `ollama` | ✅ `ollama` | `AdapterId::Ollama`, `localhost:11434` default | None |
| `groq` | ✅ `groq` | `Recipe::openai_compatible(FactoryKind::Groq)` | None |
| `openrouter` | ✅ `openrouter` | Same, plus this fork's own request hook (rename `provider_routing`→`provider`) and prompt-caching model decorator need porting *into* the adopted `RigProvider` builder call, not re-architected | Port fork's 2 openrouter customizations onto the new call site |
| `deepseek` | ✅ `deepseek` | `Recipe::openai_compatible(FactoryKind::DeepSeek)` | None |
| `gemini` | ✅ `google` (compat alias) | `AdapterId::Gemini`; upstream's `compatibility_brand()` maps handle `"gemini"` → brand `"google"` | None — same alias this fork would need anyway |
| `cohere` | ✅ `cohere` | `AdapterId::Cohere` | None |
| `mistral` | ✅ `mistral` | `Recipe::openai_compatible(FactoryKind::Mistral)` | None |
| `perplexity` | ✅ `perplexity` | Has its own `adapter/perplexity.rs` (49 lines) — check whether it does more than the generic path before assuming a bare recipe suffices | Verify during Step 2 |
| `together` | ✅ `togetherai` (compat alias) | Has `adapter/together.rs` (104 lines) | Verify during Step 2 |
| `xai` | ✅ `xai` | `Recipe::openai_compatible(FactoryKind::Xai)` | None |
| `hyperbolic` | ✅ `hyperbolic` | Has `adapter/hyperbolic.rs` (102 lines) | Verify during Step 2 |
| `moonshot` | ✅ `moonshotai` (compat alias) | `Recipe::openai_compatible(FactoryKind::Moonshot)` | None |
| `mira` | ✅ `mira` | `Recipe::openai_compatible(FactoryKind::Mira)` | None |
| `galadriel` | ✅ `galadriel` | `Recipe::openai_compatible(FactoryKind::Galadriel)` — upstream hit the same "rig dropped its adapter" problem and solved it the same way this fork did | None — converged independently |
| `huggingface` | ✅ `huggingface` | `AdapterId::Huggingface`, dedicated recipe | None |
| **`azure`** | ✅ `azure` | `adapter/azure.rs` — same `AzureOpenAIAuth::ApiKey(key)` fix this fork made independently (see below) | **Reconcile, don't take verbatim** — see Azure detail |
| **`generic`** | *(not a brand — the mechanism itself)* | `dynamic_recipe(AdapterId::Openai)` **is** this fork's `generic` provider, generalized | Drop this fork's special case; use upstream's dynamic-adapter path as-is |
| **`byteplus`** | ❌ no equivalent anywhere in upstream | — | New `Recipe::openai_compatible(FactoryKind::Byteplus)` entry, `.endpoint(BYTEPLUS_API_BASE_URL)` |
| **`zai`** | ❌ no equivalent | rig_core's `zai::Client` is itself an OpenAI-completions passthrough (`rig-core-0.42.0/src/providers/zai.rs`) | New `Recipe::openai_compatible(FactoryKind::Zai)` entry |
| **`venice`** | ❌ no equivalent | Same — Venice's completions run through rig's shared `GenericCompletionModel` (`.../providers/venice/completion.rs`) | New recipe entry |
| **`minimax`** | ❌ no equivalent | Same OpenAI-compatible passthrough in rig_core | New recipe entry |
| **`llamafile`** | ❌ no equivalent | Same — rig's llamafile client is an OpenAI-compatible local endpoint | New recipe entry |

**Bottom line:** of this fork's 6 previously-flagged "hand-added providers"
(azure, generic, byteplus, zai, venice, minimax, llamafile — 7 counting
llamafile separately), only **Azure** needs actual reconciliation work.
**Generic** needs zero new code — upstream's dynamic-adapter path already
does exactly this. The other **five** (byteplus, zai, venice, minimax,
llamafile) are one `Recipe::openai_compatible(FactoryKind::X)` line each,
identical in shape to eleven recipes upstream already carries. The earlier
audit's framing of this as "every fork-added provider has to be re-homed by
hand" overstated the difficulty — the mechanism upstream built is general
enough that most of this is copy-paste, not redesign.

## Azure: independent convergence, not a collision

Upstream's `adapter/azure.rs::build()` does:

```rust
let client = azure::Client::builder()
    .api_key(AzureOpenAIAuth::ApiKey(key.clone()))
    .azure_endpoint(endpoint.to_owned())
    .api_version(version)
    ...
```

This fork's `registry.rs` "azure" arm does the same
`AzureOpenAIAuth::ApiKey(key)` construction, with the same comment explaining
*why* (`impl From<S: Into<String>> for AzureOpenAIAuth` yields a bearer token,
so the resource-key path has to name the `ApiKey` variant explicitly). Both
sides hit the same rig_core footgun and fixed it identically. This is good
news: adopting upstream's `adapter/azure.rs` is very likely a straight take,
not a merge of two divergent implementations. What differs and needs a
decision:

- Upstream reads the API version from a generic `config.attributes` bag
  (`azure_api_version`); this fork has a dedicated `api_version: Option<String>`
  field on `ModelProviderConfig`. See the config-schema section below.
- Upstream's `validate_deployment()` restricts the model string to
  `[A-Za-z0-9._-]`; check whether this fork's existing Azure deployments (if
  any are configured in the wild) would still validate before adopting this
  wholesale.

## Config schema: `ModelProviderConfig` reconciliation

Upstream's `ModelProviderConfig` (`core/config/types.rs:921`) has grown fields
this fork's flat `core/config.rs:1031` version doesn't have, and vice versa:

**Upstream added** (needed for the adopted structure to work at all):
`credential_id: Option<Uuid>` (managed-credential linkage, Step 3),
`provider: Option<String>` (explicit brand name, decoupled from the config map
key), `adapter: Option<AdapterId>` (compiled adapter for dynamic/unknown
brands), `aws_profile`/`aws_region`/`azure_credential`, and
`#[serde(flatten)] attributes: serde_json::Map<String, Value>` (replaces
dedicated per-provider fields like this fork's `api_version` with a generic
bag — Azure's version goes in there as `azure_api_version`).

**This fork has that upstream dropped:** `billing: Option<ProviderBilling>`
(`core/config.rs:1140` — the metered/subscription/self-hosted classification
the cost-analyst agent and cost accounting depend on). This is a real
fork-only feature with no upstream equivalent — **do not lose it** when
adopting upstream's struct shape. It can ride alongside upstream's new fields
as an additional struct member, or move into upstream's `attributes` bag if a
flattened representation is preferred; either way it must survive Step 2's
config-schema merge, and its own JSON schema description (`a444b12a`'s work,
already ported) should move with it.

## Credential vault: no naming conflict

The earlier concern that this fork's `credential/vault/` and upstream's new
managed-credential work "need a coherent naming story" turns out to be a
non-issue on inspection. Upstream added a **new, separate** directory,
`credential/managed/` (17 files, ~3,684 lines — OAuth login flows and storage
for ChatGPT/Codex, GitHub Copilot, and OpenRouter subscription accounts),
sitting *alongside* `credential/vault/` and `credential/share/`, both of which
upstream kept unchanged in name and shape. This fork already has identically-named
`credential/vault/` (1Password, Bitwarden, KeePass, HashiCorp, local — API-key
storage) and `credential/share/` directories. There is no rename and no
naming collision: Step 3 is a pure addition of `credential/managed/` next to
what already exists.

## Revised Steps 2–5

1. **Step 2** — Port `frona-model-catalog` crate extraction, then
   `provider/{mod,registry,group,platform,service,validation}.rs` and
   `provider/adapter/{azure,cohere,huggingface,hyperbolic,perplexity,together}.rs`
   (the six brands where upstream and this fork already overlap). Reconcile
   Azure per above (likely a near-straight take). Verify perplexity/together/
   hyperbolic's adapter files don't hide brand-specific behavior beyond the
   generic recipe before assuming a bare recipe entry suffices for them.
   Merge `ModelProviderConfig` per the schema section above, preserving
   `billing`.
2. **Step 3** — Port `credential/managed/*` (vault, OAuth logins, storage) and
   `provider/adapter/{bedrock,chatgpt,copilot}.rs` — new capability, additive,
   confirmed no naming conflicts with this fork's existing `credential/vault/`.
3. **Step 4** — Add five one-line `Recipe::openai_compatible(FactoryKind::X)`
   entries for byteplus, zai, venice, minimax, and llamafile (new
   `FactoryKind` variants + `built_in_brands()` entries), matching the shape
   of upstream's existing groq/deepseek/mistral/xai/moonshot/mira/galadriel
   entries exactly. Drop this fork's `generic` special case entirely —
   upstream's `dynamic_recipe(AdapterId::Openai)` path already covers it.
4. **Step 5** — Port the settings-UI commits (`a908db4c`, `99db1d24`,
   `ee6ab0b1`, `7e745ac4`) last, reconciling with this fork's existing
   voice/cost-analyst settings pages and the `billing` field's UI (if any).

## What this changes in `UPSTREAM_SYNC_PLAN.md`

Group C is still a multi-PR effort — this doesn't shrink the credential-vault
or settings-UI work (Steps 2's core plumbing and Steps 3/5 are unchanged in
scope). What it does change: Step 4 (re-homing the fork's own providers),
previously described as needing a from-scratch mapping decision for each of
7 providers, is now five mechanical one-line recipe additions plus dropping
a special case — not a redesign.

## Step 2 progress (2026-09-26): the catalog crate, ported

`frona-model-catalog` is now vendored into this fork (`crates/frona-model-catalog/`,
copied verbatim from upstream) and `inference/metadata/{catalog,loader}.rs`'s
own type definitions were deleted in favor of it. This was scoped tighter
than originally planned here, once actually reading the crate turned up two
things this document didn't anticipate:

- **The fetch/cache/scheduling mechanism moved into the crate too**, along
  with a second source (`modelparams.dev`) and a build-time-baking CLI
  (`frona-model-catalog download`, the thing `385e5dd6`'s Dockerfile stage
  runs) this fork has no equivalent of. Adopting that wholesale would mean
  taking on an architecture change (build-time catalog baking vs. this
  fork's runtime fetch-and-cache) this sandbox has no way to build-test
  (same Podman-validation gap as Group B's held-back commits). **Not
  adopted this round** — this fork's own `fetch_metadata`/`fetch_remote`/
  `save_cache`/`cache_age`/`load_cache_or_defaults` and its own `parse()`
  (same intermediate `CatalogJson`/`ProviderBlock` structs, same
  cost-less-entry filter this fork has and upstream's crate-level `parse()`
  doesn't) stayed in `frona-server`, now just building the crate's
  `ModelEntry`/`ModelCatalogSnapshot` instead of locally-defined ones.
- **The crate's own `ModelEntry::cost_for` doesn't price cache-WRITE tokens
  at all** (`TokenUsage` carries no `cache_creation_input_tokens` field) —
  calling it directly would have silently stopped billing Anthropic's cache-
  write premium. Kept this fork's own cache-write-aware pricing as a
  `CostForUsage::full_cost_for` extension trait method instead of the
  crate's `cost_for`; same reasoning applies to `lookup`/`lookup_prefix`
  (need the byteplus→volcengine alias) and `protocol_default` (this fork's
  `OpenAiApi` enum vs. the crate's raw npm-label strings) — all three
  became renamed extension-trait methods (`lookup_exact`, `lookup_model`,
  `model_protocol_default`) rather than same-named ones, since an inherent
  method the crate already defines always wins silently over a trait method
  of the same name.

Verified: `cargo check --workspace`, `cargo clippy --workspace --locked --
-D warnings`, `cargo fmt --check`, the crate's own 20 tests, 161
`inference::` + 16 `cost::` unit tests (including the
`repricing_agrees_with_live_costing_for_every_provider_shape` round-trip
test), and the `usage_service_e2e` integration test (15 tests) all pass.

Still open from the original Step 2 scope: the six shared-brand adapter
files (azure, cohere, huggingface, hyperbolic, perplexity, together) and the
`ModelProviderConfig` schema merge. The catalog port didn't touch either —
this fork's provider dispatch is still the flat `inference/registry.rs`
match block, now just resolving through the vendored catalog types.
