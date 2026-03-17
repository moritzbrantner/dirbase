# folder-server

`folder-server` is a small Rust API server inspired by `json-server`, but built for an **entire folder** of JSON files instead of a single JSON document.

## Idea

`json-server` usually serves one file (for example `db.json`) as REST resources. This project applies the same idea to a whole directory:

- each `*.json` file in a folder becomes a resource,
- the file name becomes the route name,
- writes are persisted back to the same file.

You can also point `folder-server` directly at a single JSON database file (for example `db.json`) with `--file`. In that mode, each top-level key in the file is served as a resource, like `json-server`.

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
- Sorting supports `sort` and `_sort` keywords; use `-column` for descending and comma-separated multi-sort (for example `_sort=author.name,-views`).
- Pagination supports `page`/`_page` and `per_page`/`_per_page`. Array responses become an object containing `{ first, prev, next, last, pages, items, data }`.
- Embedding supports `embed` and `_embed` keywords to replace foreign key fields with the related object from another table when schema foreign keys are defined (for example `embed=author_id`).
- Item routes (`/{resource}/{id}`) assume the resource file is a JSON array of objects with an `id` field.
- `POST /{resource}` appends a new object to the array and auto-generates a numeric `id` if none is provided.
- `PUT`, `PATCH`, and `DELETE` mutate the corresponding array item and persist changes to disk.
- For object resources, `PUT /{resource}` replaces the full object and `PATCH /{resource}` merges fields.
- `--log` enables request logging and `--logname <path>` selects the log output file (default `requests.log`).
- `--readonly` disables mutation routes and only serves `GET` endpoints.
- GraphQL is not supported; use REST endpoints (`/{resource}`, `/{resource}/{id}`) and `/sql` for query-style access.
- Schema validation is enabled automatically when `{folder}/schema.dbml` exists.
- Use `--schema <path>` to load a DBML schema from a custom location.
- When a schema is active, resources must map to DBML tables and row values must match declared column types.

## Quick start

### 1) Create data files

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
  {"id": 1, "title": "Hello", "userId": 1}
]
JSON
```

### 2) Run the server

```bash
cargo run -- --folder ./data --bind 127.0.0.1:4444

# Read-only mode (only GET routes)
cargo run -- --folder ./data --bind 127.0.0.1:4444 --readonly

# Explicit schema file (if not using ./data/schema.dbml)
cargo run -- --folder ./data --schema ./schema.dbml

# Serve a single json-server-style database file
cargo run -- --file ./db.json --bind 127.0.0.1:4444
```

### 3) Try the API

```bash
curl http://127.0.0.1:4444/
open http://127.0.0.1:4444/
curl http://127.0.0.1:4444/users
curl http://127.0.0.1:4444/users/1
curl -X POST http://127.0.0.1:4444/users \
  -H 'content-type: application/json' \
  -d '{"name":"Grace"}'
```

## npm package pipeline (Rust + esbuild)

This repository now contains a Node package wrapper in [`js/`](./js) that bundles a tiny CLI launcher with **esbuild** and ships the compiled Rust binary.

### Build the npm package locally

```bash
cd js
npm install
npm run build
npm pack
```

`npm run build` performs three steps:

1. bundle `src/index.ts` and `src/cli.ts` using esbuild,
2. compile Rust in release mode,
3. copy the resulting `folder-server` binary into `js/bin/` so the npm package can execute it.

### Publish pipeline

A GitHub Actions workflow is provided at `.github/workflows/rust-to-npm.yml`.

- Trigger: pushing tags matching `npm-v*` (or manual `workflow_dispatch`).
- Toolchain: Rust stable + Node 20.
- Steps: `npm ci` → `npm run build` → `npm publish`.
- Secret required: `NPM_TOKEN`.

Once published, users can run:

```bash
npx folder-server --folder ./data --bind 127.0.0.1:4444
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
cargo test
```

## Benchmarking against typicode/json-server

A reproducible benchmark script is available at [`scripts/benchmark_vs_json_server.sh`](./scripts/benchmark_vs_json_server.sh).

- Usage and methodology: [`benchmarks/README.md`](./benchmarks/README.md)
- Latest recorded comparison: [`benchmarks/comparison.md`](./benchmarks/comparison.md)
