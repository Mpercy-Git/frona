# Upstream sync plan — fronalabs/frona

Generated 2026-09-25. Compares `upstream/main` (`fronalabs/frona`) against this
fork's `main` (`Mpercy-Git/frona`).

## How this was built

This fork's git history shares no common ancestor with upstream's (`git
merge-base origin/main upstream/main` returns nothing — the two repos were
connected as a GitHub fork without a shared history, so upstream commits are
ported by re-implementing their diffs, not by merge/cherry-pick). Prior
porting sessions record which upstream commit each fork commit re-implements
in the commit body (`Ported from upstream <hash>` / `Backport of upstream
<hash>`). Collecting those references across fork history puts the last
confirmed sync point at **2026-09-04** (upstream commits `e3ad1c4`,
`05976267`, `ec3772b7`, `652c3e0`, all ported into fork PRs #104/#111/#112/#113).

Everything upstream committed after that — **46 commits, 2026-09-05 through
2026-09-22** — has not been analyzed or ported. `git log
origin/main..upstream/main` still lists all 1075 upstream commits as
"missing" because history is disjoint; the actual unreviewed backlog is the
46 below, not 1075.

## The backlog, grouped

### A. Managed credentials, vault, and provider-adapter overhaul (26 commits, Sep 5–13) — **large, high risk**
`a444b12a..f13693f2`. 146 files, +23.5k/-3.8k lines. This is a full rework of
how Frona stores and resolves API credentials and providers:
- New `frona-catalog` crate + `derive` macro generating model parameter metadata (`fcd3d16f`, `b31f3382`).
- Encrypted managed-credential storage, rotation, and interactive login flows for ChatGPT, GitHub Copilot, and OpenRouter accounts (`86f9f346`, `5836a602`, `a1c35f0a`, `9f74cd9b`, `62026d73`, `610d1e3c`, `67a0995c`).
- New provider adapters: Azure OpenAI, Amazon Bedrock, ChatGPT subscription, GitHub Copilot (`13aadef9`, `35d82d28`, `645a383e`, `3268c03e`), plus isolating Cohere/Together/Hyperbolic/Hugging Face/Perplexity into their own adapters (`0f9d74bf`, `76f52a0b`, `92efd2bf`, `ff4f9c8f`, `3f510307`).
- Wiring: named provider connections + model groups resolve through the new system (`7c1392f9`), credentials surface through vaults (`915dc5bb`), app-state construction now initializes these services (`9a352152`), validation/admin APIs (`74f28d06`), config lists credential env var names (`d3bf621e`), and a fix so commands resolve current credentials at start (`f13693f2`).

**Why this is the risky one:** it lands squarely on code this fork already
rearchitected. The fork's vault layer has its own migration history (grant→
principal rename), and the README's "OpenRouter routing, caching & cost
accounting" and "Cost analyst agent" sections are fork-specific work built on
top of the *pre-Sep-5* provider/routing code upstream is now restructuring
underneath. A mechanical port will conflict with:
- `crates/frona-server/src/credential/` and `.../db/repo/vault.rs` (fork's existing vault work)
- whatever now carries `provider_routing`/`cache_control`/`max_price`/`only`/`data_collection`/`zdr` (fork's OpenRouter work, README lines 51–58)
- the fork's own cost/billing model (README "Cost analyst agent" section)

This needs a dedicated session that reads both the upstream diff and the
fork's current provider/vault/cost code side by side, and re-implements
(not copies) each piece the way prior "adapted where the fork diverges"
commits did. Recommend porting in upstream's own order (catalog → credential
storage → adapters → wiring) rather than as one batch, and running the
fork's OpenRouter/cost tests after each slice.

### B. Bug fixes & structured chat errors (3 commits, Sep 14) — **small, safe, do first**
`058b8ba4`, `d4186276`, `c5e95988`. 38 files, +1267/-121.
- Memory compaction falls back to the chat model instead of failing.
- Structured message-processing errors persist and render in the chat UI instead of being swallowed.

No obvious overlap with fork-specific code. Good candidate for a quick,
low-risk port independent of group A.

### C. Settings UI for the new provider/vault system (6 commits, Sep 15–16) — **depends on A**
`137de6e2`, `5aee1ead`, `a908db4c`, `99db1d24`, `ee6ab0b1`, `7e745ac4`. 24
files, +2946/-1123. Adds UI for named provider connections, model groups from
live provider directories, account logins from vault settings, and a
separate accordion for custom request parameters. Can't be usefully ported
before group A lands, since it's the frontend for that backend.

### D. Provider-catalog build/test packaging (4 commits, Sep 17–18) — **medium, depends on A**
`385e5dd6`, `38d4f5a5`, `b3052bd2`, `f104bd85`. Bundles provider catalogs in
the runtime image, adds an isolated-release-artifact test for provider
workflows, and a low-disk Rust integration test runner. Also gated on group A
existing first.

### E. Test isolation fixes (2 commits, Sep 19) — **small, safe**
`557dd9b6`, `2b9dfe28`. Isolates config tests from environment overrides;
aligns a Bedrock workflow test with live discovery errors. Low risk, but
`2b9dfe28` is Bedrock-specific so only useful once/if group A's Bedrock
adapter is ported.

### F. Dev build/container infra (4 commits, Sep 20–22) — **optional, evaluate separately**
`341280b7`, `047c9920`, `2fcf37c5`, `ac847fc9`. Mount-safe build storage,
sharing compiler artifacts through "Kache", parallel-stage Podman image
builds, stopping dev watchers/containers together. Pure dev-tooling; worth
adopting only if the fork's own `Makefile`/`mise.toml`/container dev setup
hasn't diverged enough to make these no-ops or conflicts. Not urgent.

Also noted: `f69cf35f` "release: v2026.9.0" (Sep 4, just before the window)
is a version bump, nothing to port.

## Recommended order

1. **Group B** now — small, isolated, no conflicts expected.
2. **Group E**'s config-test isolation piece (`557dd9b6`) now — same reasoning as B.
3. **Group A** as its own multi-session effort, sliced by upstream's own
   sub-areas (catalog → credentials → adapters → wiring), each slice
   reconciled against the fork's vault/OpenRouter/cost-accounting code and
   verified against the fork's existing provider tests before moving to the
   next slice.
4. **Group C, D**, and the Bedrock half of **E** once group A is in, since
   they're the UI/packaging/tests for it.
5. **Group F** opportunistically, only after checking it still applies to
   the fork's current dev tooling.

## Next check

Re-run this comparison against `upstream/main` after group A lands, and
periodically regardless (upstream was active nearly every day in the
Sep 5–22 window) — re-collect `Ported from upstream <hash>` / `Backport of
upstream <hash>` references from fork history to find the new sync point.
