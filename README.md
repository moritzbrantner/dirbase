# folder-server

`folder-server` is a small Rust API server inspired by `json-server`, but built for an **entire folder** of JSON files instead of a single JSON document.

## Idea

`json-server` usually serves one file (for example `db.json`) as REST resources. This project applies the same idea to a whole directory:

- each `*.json` file in a folder becomes a resource,
- the file name becomes the route name,
- writes are persisted back to the same file.

So if the folder contains:

- `users.json`
- `posts.json`

you get:

- `GET /users`
- `GET /posts`
- `GET /users/:id`, `POST /users`, `PUT /users/:id`, `PATCH /users/:id`, `DELETE /users/:id`, etc.

## Behavior

- `GET /` lists all available resources discovered from `*.json` files.
- While the server is running, file additions, edits, and deletions in the selected folder are watched and the available endpoints update automatically.
- `GET /{resource}` returns the whole JSON document from `{resource}.json` (array or object).
- `GET /{resource}?field=value&other=...` filters array resources by one or more exact-match query parameters.
- `GET /{resource}?sort=column` sorts array resources ascending by a column; pass multiple columns with commas (for example `sort=role,name`) or repeated `sort` params.
- Item routes (`/{resource}/{id}`) assume the resource file is a JSON array of objects with an `id` field.
- `POST /{resource}` appends a new object to the array and auto-generates a numeric `id` if none is provided.
- `PUT`, `PATCH`, and `DELETE` mutate the corresponding array item and persist changes to disk.
- For object resources, `PUT /{resource}` replaces the full object and `PATCH /{resource}` merges fields.
- `--log` enables request logging and `--logname <path>` selects the log output file (default `requests.log`).
- `--readonly` disables mutation routes and only serves `GET` endpoints.
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
cargo run -- --folder ./data --bind 127.0.0.1:3000

# Read-only mode (only GET routes)
cargo run -- --folder ./data --bind 127.0.0.1:3000 --readonly

# Explicit schema file (if not using ./data/schema.dbml)
cargo run -- --folder ./data --schema ./schema.dbml
```

### 3) Try the API

```bash
curl http://127.0.0.1:3000/
curl http://127.0.0.1:3000/users
curl http://127.0.0.1:3000/users/1
curl -X POST http://127.0.0.1:3000/users \
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
npx folder-server --folder ./data --bind 127.0.0.1:3000
```

## Notes

- Resource names are restricted to letters, numbers, `_`, and `-`.
- Item-level endpoints (`/{resource}/{id}`) expect array-based resources (`[{"id": ...}, ...]`).
- Object resources support `GET /{resource}`, `PUT /{resource}`, and `PATCH /{resource}`.
- Invalid JSON in a file returns a 500 with an error payload.
