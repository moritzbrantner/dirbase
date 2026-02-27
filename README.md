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
- `GET /{resource}` returns the whole JSON document from `{resource}.json`.
- Item routes (`/{resource}/{id}`) assume the resource file is a JSON array of objects with an `id` field.
- `POST /{resource}` appends a new object to the array and auto-generates a numeric `id` if none is provided.
- `PUT`, `PATCH`, and `DELETE` mutate the corresponding item and persist changes to disk.

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

## Notes

- Resource names are restricted to letters, numbers, `_`, and `-`.
- For item-level endpoints, this tool currently expects array-based resources (`[{"id": ...}, ...]`).
- Invalid JSON in a file returns a 500 with an error payload.
