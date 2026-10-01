## Summary

<!-- What does this change do, and why? One short paragraph is plenty. -->

## Related issues

<!-- e.g. Closes #123, Refs #456. Delete if not applicable. -->

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor / internal change
- [ ] Documentation
- [ ] Build, CI or release tooling

## Areas touched

- [ ] Backend (`crates/`)
- [ ] Web UI (`web/`)
- [ ] Container / build (`build/`, `Makefile`, `mise.toml`)
- [ ] Docs / examples

## Checks

Run locally before requesting review — these mirror the CI jobs:

- [ ] `mise run fmt:check` — Rust formatting is clean
- [ ] `mise run lint:backend` — clippy passes with `-D warnings`
- [ ] `mise run test` — workspace tests pass
- [ ] `cd web && npx tsc --noEmit && npm run lint && npx vitest run` — only if `web/` changed
- [ ] Tests added or updated to cover the change (or explain below why not)

## Configuration and data

- [ ] No new or renamed config keys / environment variables
      <!-- Otherwise list them here and say what happens when they're unset. -->
- [ ] No database schema change
      <!-- Otherwise describe the migration and how existing deployments are handled. -->
- [ ] No breaking change to behaviour or public API
      <!-- Otherwise describe what breaks and what users need to do. -->

## Fork context

This repository is a fork of [`fronalabs/frona`](https://github.com/fronalabs/frona).

- [ ] This change is fork-specific
- [ ] This change would also make sense upstream

## Verification

<!-- How did you check this works? Commands run, manual steps, screenshots for
     UI changes, or logs for backend behaviour. -->

## Notes for reviewers

<!-- Anything worth flagging: trade-offs taken, areas you'd like a closer look
     at, or follow-up work deliberately left out of this PR. Delete if none. -->
