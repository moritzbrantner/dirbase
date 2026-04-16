# PokeAPI benchmark: folder-server vs json-server

This benchmark uses the CSV snapshot under `benchmarks/pokeapi/data/v2/csv` as a realistic dataset instead of the small synthetic `posts` fixture.

## Generated data

The generator converts every CSV into:

- `benchmarks/.work/pokeapi-json/folder/<resource>.json` for `folder-server`
- `benchmarks/.work/pokeapi-json/db.json` for `json-server`
- `benchmarks/.work/pokeapi-json/metadata.json` with row counts and inferred column types

Run the generator directly:

```bash
python3 scripts/build_pokeapi_json.py
```

Force a rebuild:

```bash
python3 scripts/build_pokeapi_json.py --force
```

## Benchmark scenarios

The PokeAPI benchmark exercises a broader mix of reads than the synthetic benchmark:

- item lookups on `pokemon`, `moves`, and `encounters`
- equality filters on medium and large resources
- numeric range filters
- substring filters on names and flavor text
- multi-column sorting
- pagination on large resources
- combined filter + sort + pagination on `encounters` and `pokemon_moves`

## Run

```bash
scripts/benchmark_pokeapi.sh
```

Single-command validation run:

```bash
python3 scripts/run_tests_and_benchmark.py
```

Optional knobs:

```bash
DURATION=15 CONNECTIONS=100 RUNS=5 WARMUP_DURATION=3 WARMUP_CONNECTIONS=1 scripts/benchmark_pokeapi.sh
```

You can also rebuild the derived JSON before running:

```bash
FORCE_REBUILD_DATA=1 scripts/benchmark_pokeapi.sh
```

## Output

Raw `autocannon` output, summary JSON, and a markdown report are written to `benchmarks/results/`.

- `benchmarks/results/pokeapi-*-with-warmup-run<run>-<timestamp>.json`
- `benchmarks/results/pokeapi-*-without-warmup-run<run>-<timestamp>.json`
- `benchmarks/results/pokeapi-summary-<timestamp>.json`
- `benchmarks/results/pokeapi-report-<timestamp>.md`
