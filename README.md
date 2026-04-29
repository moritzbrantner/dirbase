# dirbase

`dirbase` is a small Rust API server for a directory-based JSON datastore. It takes the `json-server` idea and applies it to an **entire folder** of JSON files instead of a single JSON document.

## Install options

| Path | Best for | Command |
| --- | --- | --- |
| Direct binary | Fastest first run | Download the latest release from <https://github.com/moritzbrantner/dirbase/releases> and run `dirbase ./data` |
| `cargo install` | Rust users who want a local CLI | `cargo install --git https://github.com/moritzbrantner/dirbase.git` |
| Bun wrapper | Optional JavaScript distribution flow | `bunx --bun dirbase ./data --bind 127.0.0.1:4444` |

Default recommendation: use a release binary when you want the shortest path to a working server. `cargo build`, `cargo run`, `cargo test`, and `cargo install` use the checked-in `ui/dist/*` assets by default and do not require Bun.

## First run

### 1) Create a small dataset

```bash
mkdir -p data
cat > data/users.json <<'JSON'
[
  {"id": 1, "name": "Ada"},
  {"id": 2, "name": "Linus"}
]
JSON

cat > data/posts.json <<'JSON'
[
  {"id": 1, "title": "Hello", "user_id": 1}
]
JSON
```

### 2) Start the server

```bash
dirbase ./data

# Or, without installing globally:
cargo run -- ./data
```

Startup prints a short summary that includes the detected source mode, resource count, schema status, and the browser URL to open.

### 3) Verify it works

```bash
curl http://127.0.0.1:4444/users
curl http://127.0.0.1:4444/users/1
curl -X POST http://127.0.0.1:4444/users \
  -H 'content-type: application/json' \
  -d '{"name":"Grace"}'
```

Then open the printed browser URL to use the built-in overview, explorer, relation map, and schema editor.

## Idea

`json-server` usually serves one file (for example `db.json`) as REST resources. This project applies the same idea to a whole directory:

- each `*.json` file in a folder becomes a resource,
- the file name becomes the route name,
- writes are persisted back to the same file.

You can point `dirbase` directly at either a folder or a single JSON database file by passing the path positionally. In file mode, each top-level key in the file is served as a resource, like `json-server`.

So if the folder contains:

- `users.json`
- `posts.json`

you get:

- `GET /users`
- `GET /posts`
- `GET /users/:id`, `POST /users`, `PUT /users/:id`, `PATCH /users/:id`, `DELETE /users/:id`, etc.

## Behavior

- `GET /` lists all available resources as JSON for API clients, and renders a visual HTML overview for browsers.
- While the server is running, file additions, edits, and deletions in the selected folder are watched and the available endpoints update automatically.
- `GET /{resource}` returns the whole JSON document from `{resource}.json` (array or object).
- `GET /{resource}?field=value&other=...` filters array resources (default operator is `eq`).
- Advanced filters use `field:operator=value` and support: `eq`, `ne`, `lt`, `lte`, `gt`, `gte`, `in`, `contains`, `startsWith`, and `endsWith` (for example `views:gt=100`, `author.name:eq=typicode`, `title:contains=hello`).
- Null filters support `field:isNull=true` and `field:isNotNull=true`.
- Sorting supports `sort` and `_sort` keywords; use `-column` for descending and comma-separated multi-sort (for example `_sort=author.name,-views`).
- Pagination supports `page`/`_page` and `per_page`/`_per_page`. Array responses become an object containing `{ first, prev, next, last, pages, items, data }`.
- Embedding supports `embed` and `_embed` keywords to replace foreign key fields with the related object from another table when schema foreign keys are defined (for example `embed=author_id`).
- `GET /events` streams `overview_changed`, `resource_changed`, and `schema_changed` SSE notifications for live UIs.
- `GET /schema` returns the current schema metadata inferred from JSON tables and merged with any declared schema.
- `POST /schema` saves the full effective schema as `schema.json` next to the served folder or database file.
- `PUT /schema` validates and saves a declared schema overlay to `schema.json`.
- `POST /schema/infer` re-infers schema metadata from the current data and writes that inferred snapshot to `schema.json`.
- `GET /graphql` serves GraphiQL for browser requests and executes GraphQL queries for API requests.
- `POST /graphql` accepts standard GraphQL JSON bodies with `query`, `variables`, and `operationName`.
- Item routes (`/{resource}/{id}`) use the table primary key from schema metadata when available, and fall back to `id`.
- `POST /{resource}` appends a new object to the array and auto-generates a numeric primary key when none is provided.
- `PUT`, `PATCH`, and `DELETE` mutate the corresponding array item and persist changes to disk.
- For object resources, `PUT /{resource}` replaces the full object and `PATCH /{resource}` merges fields.
- Passing a positional path makes `dirbase` inspect the filesystem and choose file or folder mode automatically.
- If the positional path does not exist yet, it is treated as a folder unless it ends in `.json`.
- If `./dirbase.conf` exists in the current working directory, `dirbase` loads it automatically using the same CLI-style arguments as the command line; explicit CLI arguments take precedence.
- `--port <port>` overrides just the listen port while keeping the current bind address host.
- `--log` enables request logging and `--logname <path>` selects the log output file (default `requests.log`).
- `--readonly` disables mutation routes and only serves `GET` endpoints.
- `--auth-token <token>` enables bearer-token auth for application routes.
- `--cors-origin <origin>` enables explicit CORS for a single allowed origin.
- `--max-body-bytes`, `--max-per-page`, `--max-sql-scan-rows`, and `--max-sql-selected-rows` configure request and query limits.
- `GET /healthz`, `GET /readyz`, and `GET /metrics` expose operational status and counters.
- Schema metadata is inferred automatically for array-of-object resources. Object tables prefer `id` as the primary key and also detect `<table>_id` or `<singular_table>_id` when those columns are unique and present on every row. Foreign keys are inferred conservatively from `*_id` columns.
- Declared schema overlays are enabled automatically when `{folder}/schema.json` or `{folder}/schema.dbml` exists.
- Use `--schema <path>` to load a custom `.dbml` or `.json` schema file.
- Schema auto-discovery prefers `schema.json` over `schema.dbml`.
- Declared schema overlays are permissive: undeclared resources and undeclared columns are still allowed, while declared columns, primary keys, and foreign keys override inferred metadata.
- GraphQL remains read-only, but now also exposes query-capable collection fields such as `usersQuery(filter:, sort:, page:, perPage:)`.

## Capabilities by interface

| Interface | Read | Write | Filtering / sorting / pagination | Relations | Notes |
| --- | --- | --- | --- | --- | --- |
| REST | Yes | Yes unless `--readonly` | Yes | `embed` plus item routes | Primary API surface |
| GraphQL | Yes | No | `*Query` fields support filter / sort / pagination | Foreign-key traversal | GraphiQL at `GET /graphql` |
| SQL | Yes | No | `SELECT`, projection, `WHERE`, `ORDER BY`, `LIMIT/OFFSET` | Schema-backed `INNER JOIN` | Query endpoint at `/sql` |
| Overview UI | Yes | Yes unless `--readonly` | Explorer drives REST params | Live-updating relation map | Root HTML at `GET /` |

## Why dirbase?

`dirbase` is for projects where the local JSON files are the source of truth and should become a usable API immediately. Compared with `json-server`, it keeps each resource in its own file, watches the folder for changes, infers schema metadata, supports schema-backed relations, exposes REST, GraphQL, and SQL read paths over the same data, and includes operational endpoints such as health checks and metrics.

| Capability | dirbase | json-server | Mockoon | Prism |
| --- | --- | --- | --- | --- |
| Folder of JSON files as API | Yes | No | Configured mocks | No |
| Single `db.json` mode | Yes | Yes | No | No |
| Persisted CRUD writes | Yes | Yes | Mock-oriented | No |
| REST API | Yes | Yes | Yes | Yes |
| GraphQL API | Yes, read-only | No | No | No |
| SQL endpoint | Yes, read-only | No | No | No |
| File watching | Yes | Limited / version-dependent | No | No |
| Schema inference | Yes | No | No | OpenAPI-driven |
| Best fit | Local JSON datastore API | Simple fake REST API | Rich mock scenarios | Contract-first API mocking |

See [`BENCHMARKS.md`](./BENCHMARKS.md) for comparison notes, benchmark methodology, and reproduction commands.

## CLI examples

```bash
# Keep the default host and only override the port
dirbase ./data --port 5555

# Read-only mode (only GET routes)
dirbase ./data --readonly

# Auth + explicit CORS
dirbase ./data --auth-token secret --cors-origin http://localhost:3000

# Explicit schema file (if not using ./data/schema.dbml)
dirbase ./data --schema ./schema.dbml

# Serve a single json-server-style database file
dirbase ./db.json --bind 127.0.0.1:4444
```

Optional local config in the directory where you run `dirbase`:

```bash
cat > dirbase.conf <<'CONF'
--folder ./data
--port 4444
--readonly
CONF

# Uses dirbase.conf automatically
dirbase

# Explicit CLI args override dirbase.conf
dirbase --bind 127.0.0.1:5555
```

## Schema files

`dirbase` supports both `schema.json` and `schema.dbml`.

- `schema.json` is the editable, preferred format.
- `schema.dbml` is still supported for compatibility.
- If both files exist next to the data source, `schema.json` wins.
- `GET /schema` always shows the merged effective schema.
- `POST /schema` writes the merged effective schema back to `schema.json`.
- `PUT /schema` writes a declared schema overlay to `schema.json`.
- `POST /schema/infer` writes a fresh inferred schema snapshot to `schema.json`.

Manual schema files act as overlays on top of inference:

- omitted fields keep their inferred values
- declared `primary_key` overrides inferred primary-key detection
- declared `foreign_keys` override or add relationships by source column
- declared `columns` override inferred type/nullability for those columns only

### JSON example

This example connects `posts.author_ref` to `users.user_id`, even though the names do not follow the default `*_id` convention:

```json
{
  "tables": {
    "users": {
      "primary_key": "user_id"
    },
    "posts": {
      "foreign_keys": {
        "author_ref": {
          "target_table": "users",
          "target_column": "user_id"
        }
      }
    }
  }
}
```

### DBML example

The same relationship can be declared in DBML:

```dbml
Table users {
  user_id int [pk]
  name varchar
}

Table posts {
  id int [pk]
  author_ref int
  title varchar
}

Ref: posts.author_ref > users.user_id
```

With either format:

- `GET /posts?embed=author_ref` replaces the foreign-key value with the matching user row
- `GET /users/1` resolves against `user_id` when that is the declared primary key
- `POST /schema` exports a complete `schema.json` snapshot with inferred columns plus declared PK/FK overrides

## Optional Bun package pipeline (Rust + esbuild)

This repository now contains a Bun-powered package wrapper in [`js/`](./js) that bundles a tiny CLI launcher with **esbuild** and ships the compiled Rust binary.

### Build the package locally

```bash
cd js
bun install
bun run build
bun pm pack
```

`bun run build` performs three steps:

1. bundle `src/index.ts` and `src/cli.ts` using esbuild,
2. compile Rust in release mode,
3. copy the resulting `dirbase` binary into `js/bin/` so the package can execute it.

### Publish pipeline

A GitHub Actions workflow is provided at `.github/workflows/rust-to-bun.yml`.

- Trigger: pushing tags matching `bun-v*` (or manual `workflow_dispatch`).
- Toolchain: Rust stable + Bun 1.3.12.
- Steps: build native binaries on Linux, macOS, and Windows, bundle the JS launcher, then publish one package containing all prebuilt binaries with Bun.
- Secret required: `NPM_TOKEN`.

Once published, users can run:

```bash
bunx --bun dirbase ./data --bind 127.0.0.1:4444
```

## Notes

- Resource names are restricted to letters, numbers, `_`, and `-`.
- Item-level endpoints (`/{resource}/{id}`) expect array-based resources (`[{"id": ...}, ...]`).
- Object resources support `GET /{resource}`, `PUT /{resource}`, and `PATCH /{resource}`.
- Invalid JSON in a file returns a 500 with an error payload.

## Rust toolchain and formatting

This repo pins the Rust toolchain in `rust-toolchain.toml` and includes `rustfmt` + `clippy` as required components.

### One-time local setup

```bash
git config core.hooksPath .githooks
```

This enables the repository's `pre-commit` hook, which runs `cargo fmt --all` automatically before every commit (and stages formatting changes).

### Editor / Codex format-on-save

Workspace settings are provided in `.vscode/settings.json` to enable format-on-save for Rust via rust-analyzer + rustfmt.

If your editor uses different settings, configure it to run `cargo fmt --all` on save.

## Development checks

Before committing or opening a PR, always run linting and tests:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features -- --test-threads=1
bash scripts/run_repo_tests.sh
```

Canonical test commands:

```bash
cargo test -- --test-threads=1

cd ui
bun run test
bun run test:coverage
bun run test:e2e

cd ../js
bun test
```

Do not use `cd ui && bun test` in this repository. That invokes Bun's test runner instead of Vitest, bypasses the `jsdom` setup in [`ui/vitest.config.ts`](./ui/vitest.config.ts), and produces false failures around globals such as `vi.stubGlobal`, `document`, and `EventSource`.

Maintainers only: the checked-in `ui/dist/overview.css` and `ui/dist/overview.js` assets are used by default. Rebuild them explicitly when the overview UI changes:

```bash
cd ui
bun run build

# Or from the repo root
DIRBASE_REBUILD_UI=1 cargo build
```

### Coverage

Coverage is reported separately from the fast PR test commands so it can guide test expansion without making local iteration slower.

```bash
cargo llvm-cov --all-features --summary-only -- --test-threads=1
cargo llvm-cov --all-features --lcov --output-path target/llvm-cov/lcov.info -- --test-threads=1

cd ui
bun run test:coverage
```

## Benchmarking against typicode/json-server

A reproducible benchmark script is available at [`scripts/benchmark_vs_json_server.sh`](./scripts/benchmark_vs_json_server.sh).

- Comparison guide and methodology: [`BENCHMARKS.md`](./BENCHMARKS.md)
- Usage and methodology: [`benchmarks/README.md`](./benchmarks/README.md)
- Historical comparison note: [`benchmarks/comparison.md`](./benchmarks/comparison.md)
