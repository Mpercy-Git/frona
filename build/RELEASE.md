# Release Process

## Versioning Scheme

Frona uses **CalVer** in the format `YYYY.M.PATCH`:

- `YYYY` — full year of the release
- `M` — month of the release (no zero-padding: `5`, not `05`)
- `PATCH` — sequential in-month patch counter, resets to `0` on the first release of a new month

So `2026.5.0` is the first May 2026 release, `2026.5.1` is the first follow-up patch in May, and `2026.6.0` opens June 2026.

Frona ships as an application (Docker image to GHCR), not a library — there is no API-compatibility contract for SemVer to encode, so the date of the release is the more useful signal.

## Quick Start

```bash
mise run release                          # cut today's release; auto-rolls month or bumps PATCH
mise run release patch                    # 2026.5.0 → 2026.5.1 (strict: errors on month change)
mise run release alpha                    # start or advance an alpha pre-release
mise run release beta                     # advance to beta
mise run release rc                       # advance to rc
mise run release stable                   # 2026.5.0-RC1 → 2026.5.0
mise run release 2026.5.0-RC1             # explicit version
```

## CLI Usage

```
build/release.sh [command] [--dry-run] [--skip-docker] [--skip-tests]
```

### Commands

| Command | Description |
|---------|-------------|
| *(no arg)* / `today` | Cut today's release; bumps `PATCH` within the current `YYYY.M`, or resets to `YYYY.M.0` when the month rolls over |
| `patch` | Increment `PATCH` within the current `YYYY.M`. Strict: errors if the calendar month has rolled over since the last release — use the no-arg form instead |
| `alpha` / `beta` / `rc` | Start or advance a pre-release. Rolls to today's `YYYY.M` if the current version is already stable |
| `stable` | Promote current pre-release to stable |
| `<version>` | Set an explicit version (e.g., `2026.5.0-RC1`) |

### Flags

| Flag | Description |
|------|-------------|
| `--dry-run` | Preview changes without modifying anything |
| `--skip-docker` | Version bump + git tag only, no Docker build |
| `--skip-tests` | Skip `cargo test` before releasing |

## Pre-release Format

Pre-release versions use FreeBSD-style uppercase tags without dot separators:

- `2026.5.0-ALPHA1`, `2026.5.0-BETA2`, `2026.5.0-RC1`

Commands are lowercase for ergonomics (`mise run release alpha`).

## Pre-release Workflow

```bash
# Start a pre-release series (rolls to today's YYYY.M, bumps PATCH if same month)
mise run release alpha                    # 2026.5.0 → 2026.5.1-ALPHA1

# Iterate within a pre-release tag
mise run release alpha                    # 2026.5.1-ALPHA1 → 2026.5.1-ALPHA2

# Advance to the next stage
mise run release beta                     # 2026.5.1-ALPHA2 → 2026.5.1-BETA1
mise run release rc                       # 2026.5.1-BETA1 → 2026.5.1-RC1

# Promote to stable
mise run release stable                   # 2026.5.1-RC1 → 2026.5.1
```

## Docker Tagging

- **Stable** `2026.5.0` → `ghcr.io/fronalabs/frona:v2026.5.0` + `:latest`
- **Pre-release** `2026.5.0-ALPHA1` → `ghcr.io/fronalabs/frona:v2026.5.0-ALPHA1` only (no `:latest`)

## Version Sources

The script updates these files in sync:

- `Cargo.toml` — `version` under `[workspace.package]`
- `web/package.json` — `"version"` field
- `web/package-lock.json` — root `"version"` + `packages[""]` version

## Safety Checks

1. Working tree must be clean (no uncommitted changes)
2. Stable releases must be from the `main` branch
3. Git tag must not already exist (also blocks accidental same-day re-runs)
4. Tests must pass (unless `--skip-tests`)

## Git Operations

- Commit message: `release: v{version}`
- Annotated tag: `v{version}`
- Auto-pushes commit and tag to `origin`
