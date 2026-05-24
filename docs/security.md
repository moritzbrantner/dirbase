# Security

`dirbase` serves local JSON files as live REST, GraphQL, SQL, and overview UI endpoints. Treat the data directory or database file as the source of truth: anyone with write access to mutation routes can change those files.

## Runtime Posture

- The default bind address is `127.0.0.1:4444`, so a default server is reachable only from the local machine.
- Use `--readonly` for demos, shared environments, and any place where the API should not mutate source files.
- Use `--auth-token <token>` whenever binding beyond loopback, for example with `--bind 0.0.0.0:4444`.
- Use `--cors-origin <origin>` only for an explicit browser client that must call the API cross-origin.
- `GET /healthz`, `GET /readyz`, and `GET /metrics` are public by default for compatibility. Use `--protect-ops` with `--auth-token` to require bearer auth for `/readyz` and `/metrics`.
- `dirbase` is not a multi-tenant authorization layer. It has a single optional bearer token and does not provide per-resource or per-row access control.

## Browser Responses

HTTP responses include baseline browser hardening headers:

- `X-Content-Type-Options: nosniff`
- `Referrer-Policy: no-referrer`
- `X-Frame-Options: sameorigin`
- `Cross-Origin-Resource-Policy: same-origin`
- `Permissions-Policy: camera=(), microphone=(), geolocation=()`

Content Security Policy is intentionally not enabled globally because the embedded overview, HTML forms, and GraphiQL need route-specific review before a strict policy can be applied.

## Release Integrity

The npm publish workflow is `.github/workflows/rust-to-bun.yml`. Configure npm trusted publishing for repository `moritzbrantner/dirbase` and that workflow when enabling OIDC-based provenance.

The package includes `bin/SHA256SUMS` during publish. Consumers can compare each installed platform binary under `bin/<platform>/` against the matching checksum entry.

Related upstream documentation:

- Bun publish: https://bun.sh/docs/cli/publish
- npm trusted publishing: https://docs.npmjs.com/trusted-publishers/
- npm provenance: https://docs.npmjs.com/generating-provenance-statements
