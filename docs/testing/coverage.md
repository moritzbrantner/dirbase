# Test Coverage Matrix

This repository tracks coverage in two complementary ways:

- numeric coverage from `cargo llvm-cov` and Vitest/Istanbul
- semantic coverage in `docs/testing/test-matrix.json`

Numeric coverage shows which lines ran. The matrix shows which product behavior is checked, which parameters are represented, and whether each row covers happy paths, exceptional paths, and concurrency-sensitive paths.

## Updating The Matrix

Update `docs/testing/test-matrix.json` when you add, remove, or materially change:

- public Rust API surfaces, routes, middleware, storage behavior, schema behavior, GraphQL, SQL, watcher, or CLI behavior
- exported UI helpers, hooks, or components
- JS wrapper exports
- tests that prove one of those behaviors

Each matrix entry should identify:

- the area it belongs to
- the module and symbol or interface being covered
- risk level: `critical`, `high`, `medium`, or `low`
- tests that prove the behavior, with exact test names and paths
- relevant parameter values that are covered or missing
- happy, exceptional, and concurrency status

Use `missing_cases` rather than vague notes when a behavior is intentionally not covered yet. That keeps follow-up work directly actionable.

## Classification Rules

Happy-path coverage means the expected successful behavior is asserted with representative inputs.

Exceptional-path coverage means invalid, missing, malformed, unauthorized, readonly, unsupported, or otherwise rejected inputs are asserted.

Concurrency coverage means simultaneous or bursty operations are asserted when the code has shared mutable state, filesystem persistence, locks, cache invalidation, watcher events, EventSource streams, or parallel request behavior.

Use `not_applicable` only when concurrency is not meaningful for that row.

## Private Helpers

Private helpers do not need their own matrix row when they are simple and already covered through a public behavior row.

Add a direct row or list the helper in `covered_symbols` when the helper is:

- a parser or validator
- concurrency or locking related
- security/auth/CORS related
- schema/type compatibility related
- serialization/deserialization related
- hard to observe through an endpoint, component, or exported function

## Running Checks

Validate the matrix and show drift warnings:

```bash
python3 scripts/render_test_matrix.py --check
```

Generate the static report:

```bash
python3 scripts/render_test_matrix.py --write
```

Open `target/test-matrix/index.html` in a browser, or serve it locally:

```bash
python3 scripts/render_test_matrix.py --serve --port 8765
```

The generated files are written to `target/test-matrix/` and are not meant to be committed.

## Numeric Coverage

Rust:

```bash
cargo llvm-cov --all-features --summary-only -- --test-threads=1
cargo llvm-cov --all-features --lcov --output-path target/llvm-cov/lcov.info -- --test-threads=1
```

UI:

```bash
cd ui
bun run test:coverage
```

Repository test suite:

```bash
bash scripts/run_repo_tests.sh
```

The matrix check is intentionally separate from `scripts/run_repo_tests.sh` so local test iteration stays focused. CI runs the matrix check in the coverage workflow.
