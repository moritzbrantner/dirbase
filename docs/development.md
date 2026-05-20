# Development

This repository contains a Rust server, a Bun-built React overview UI, and a Bun package wrapper for npm distribution.

## Setup

Install Rust from `rust-toolchain.toml` and Bun 1.3.12 from `.bun-version`.

```bash
bun install --cwd ui --frozen-lockfile
bun install --cwd js --frozen-lockfile
```

Cargo commands do not require Bun for normal server builds because `ui/dist/overview.css` and `ui/dist/overview.js` are checked in.

## Daily Development

```bash
bun run dev
```

This starts `cargo run -- ./examples/school --bind 127.0.0.1:4444`.

For focused work, prefer the native command for the area you are changing:

```bash
cargo test -- --test-threads=1
bun run --cwd ui test
bun run --cwd js test
```

## Standard Project Commands

```bash
bun run test
bun run lint
bun run format
bun run format:check
bun run build
bun run verify
bun run hygiene
```

`bun run verify` runs formatting checks, Rust Clippy, UI typecheck, a Rust build, and `scripts/run_repo_tests.sh`.

## UI Bundle

The Rust server embeds the checked-in UI bundle in `ui/dist/`. Rebuild it only when UI source changes:

```bash
bun run build:ui
```

`ui/src/tailwind.generated.css`, `ui/dist/overview.css`, and `ui/dist/overview.js` are generated outputs from that command.

## Full Verification

Use this before opening a PR that touches multiple areas:

```bash
git status --short
bun run format:check
bun run lint
bun run test
bun run build
bun run verify
```

`scripts/run_repo_tests.sh` is the canonical CI-style local test script. It runs Rust tests, validates the test matrix, typechecks UI, runs UI unit tests, UI coverage, Playwright E2E, and JS wrapper tests.

If Playwright browsers are missing locally, install Chromium with:

```bash
cd ui
bunx playwright install --with-deps chromium
```

Do not use `cd ui && bun test`; that invokes Bun's test runner instead of Vitest.

## Repo Hygiene

Run the reporting-only hygiene check when generated files may have been created:

```bash
bun run hygiene
```

It reports dirty status, untracked files, upstream configuration, ahead/behind state, generated directories that are accidentally tracked, and local-only ignore coverage.

Ignored local outputs include `target/`, `data/`, `requests.log`, `node_modules/`, `js/dist/`, `js/bin/`, UI coverage and Playwright reports, and benchmark work/results outputs. `ui/dist/overview.css` and `ui/dist/overview.js` are intentionally tracked.

## Release And Publish

There is no local root release command. The npm package publish flow is handled by `.github/workflows/rust-to-bun.yml`.

- Tags matching `bun-v*` or manual `workflow_dispatch` trigger the workflow.
- The workflow builds platform Rust binaries, bundles the JS launcher, downloads the binaries into `js/bin`, and runs `bun publish`.
- Publishing requires the `NPM_TOKEN` GitHub Actions secret.

## Troubleshooting

- If `bun run --cwd ui test:e2e` fails because Chromium is unavailable, run `cd ui && bunx playwright install --with-deps chromium`.
- If Cargo rebuilds the UI unexpectedly, check whether `DIRBASE_REBUILD_UI=1` is set.
- If `bun run hygiene` reports missing upstream, set one with `git push --set-upstream origin <branch>` after confirming the branch should be published.
