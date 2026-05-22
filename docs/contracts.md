# Public Contracts

This project treats the CLI, HTTP API, GraphQL API, SQL endpoint, overview UI, JavaScript wrapper, and generated UI bundle as compatibility surfaces.

## Stable Behavior Surfaces

Keep these stable unless a change is explicitly designed as a behavior change:

- CLI flags, config-file loading, command-line precedence, startup output, and process exit behavior
- REST route paths, HTTP methods, status codes, response bodies, content types, auth, CORS, XML mode, metrics, and SSE events
- GraphQL schema naming, GraphiQL serving, query execution, relation traversal, and error response shape
- SQL query endpoint behavior, supported SQL subset, row limits, structured error codes, and export output
- Overview UI workflows for exploring data, editing data, editing schema, live updates, and request URL generation
- JavaScript wrapper package exports, binary lookup, and CLI launcher behavior

## Error Responses

API errors use this JSON shape:

```json
{
  "error": "message",
  "code": "optional_static_code"
}
```

Do not add a `code` field to an existing error response unless the behavior change is intentional and covered by contract tests. Existing internal error code constants live in `src/error.rs`.

GraphQL request-level errors use the GraphQL response shape with an `errors` array and `application/graphql-response+json` for explicit GraphQL request errors.

## Test Matrix Rules

Update `docs/testing/test-matrix.json` when changing or adding tests for:

- public Rust routes, middleware, storage, schema, GraphQL, SQL, watcher, or CLI behavior
- exported UI helpers, hooks, or components
- JavaScript wrapper exports
- parsers, validators, concurrency-sensitive helpers, serialization helpers, auth/CORS helpers, or schema compatibility helpers

Run:

```bash
python3 scripts/render_test_matrix.py --check
```

## Generated UI Assets

Cargo builds embed the checked-in UI bundle by default. Only change these generated files when UI source changed intentionally and `bun run build:ui` was run:

- `ui/src/tailwind.generated.css`
- `ui/dist/overview.css`
- `ui/dist/overview.js`

For Rust-only, docs-only, CI-only, and benchmark-only changes, this check must stay clean:

```bash
bash scripts/check_generated_ui_clean.sh
```
