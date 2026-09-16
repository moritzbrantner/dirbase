# Architecture performance evidence

This benchmark surface exists to catch architectural performance regressions before they are hidden behind aggregate HTTP throughput numbers.

The primary rule is **local work should stay local**. Dirbase owns mutable resource state, and a small mutation should not require copying or recomputing unrelated resources. Derived views such as schema metadata, indexes, query materializations, and serialized responses should be updated or created only when the operation actually needs them.

## Evidence boundaries

The tools have separate responsibilities:

- **Moonlight** checks deterministic behavior across a baseline and candidate. It compares the architecture contract, not timings.
- **runtime-profiler** captures repeatable wall-time/process evidence for declared workloads.
- `scripts/architecture_benchmark.py` records Dirbase-specific server evidence such as startup time, request distributions, and sampled server RSS. This avoids treating a wrapper process's RSS as Dirbase memory.

Performance numbers are descriptive evidence. This slice intentionally does not introduce a release threshold. Budgets should be calibrated from repeated comparable captures after the scenarios are stable.

## Scenarios

### Hot read window

`hot-read-window.json` serves a 48,000-row resource and repeatedly requests a filtered, sorted eight-row page.

This scenario is intended to expose unnecessary whole-resource cloning, repeated parsing, repeated schema work, and materialization that grows with the source resource rather than the requested result window.

### Localized write: small

`localized-write-small.json` patches one row in an 8,000-row resource.

This is the reference workload for the cost of a local mutation.

### Localized write: unrelated ballast

`localized-write-ballast.json` performs the same patch workload against the same 8,000-row target resource while four unrelated 20,000-row resources are present.

The semantic operation is unchanged. A large increase relative to the small fixture is evidence that mutation cost depends on unrelated state. That is the architectural signal to reduce full-resource copies, global schema reinference, or other whole-world recomputation.

The benchmark also hashes unrelated resource files before and after the mutation workload. A local mutation fails the contract if it changes unrelated bytes.

## Deterministic contract

Run the local semantic contract with:

```bash
bash scripts/architecture-contract.sh
```

It verifies a narrow paginated/filter read, repeated PATCH deltas, persisted final state, stable row count, and unchanged unrelated resources. Its stdout is deterministic so Moonlight can compare it across revisions.

For a baseline/candidate comparison:

```bash
bash scripts/moonlight-architecture-compare.sh /path/to/baseline /path/to/candidate
```

The repository also contains `moonlight.eval.toml` for project-level baseline/candidate evaluation.

## Runtime Profiler

Build and prepare deterministic fixtures before capture:

```bash
cargo build --release
python3 scripts/architecture_benchmark.py prepare-all
```

Then capture any scenario, for example:

```bash
bash scripts/runtime-profile.sh \
  .artifacts/runtime-profiler/hot-read-window \
  profiles/runtime-profiler/hot-read-window.json
```

The three architecture scenarios are:

- `profiles/runtime-profiler/hot-read-window.json`
- `profiles/runtime-profiler/localized-write-small.json`
- `profiles/runtime-profiler/localized-write-ballast.json`

Each measured run writes a final Dirbase-specific JSON record under `.artifacts/architecture/`. Runtime Profiler remains the authoritative cross-revision runtime bundle; the JSON record is supporting evidence for interpreting what the server itself did during that scenario.

## Architectural interpretation

When using these results to reengineer Dirbase, prefer changes that reduce multiplicative costs rather than micro-optimizing individual functions. In particular, investigate:

- full `serde_json::Value` clones on mutation or response paths;
- schema inference that rescans resources unaffected by a delta;
- indexes or derived views rebuilt when they could be incrementally updated or invalidated narrowly;
- query stages that materialize full collections before applying a small output window;
- serialization boundaries that copy data simply to satisfy ownership layering.

Do not optimize by weakening persistence, query semantics, schema correctness, or concurrency safety. Moonlight's semantic contract remains the baseline/candidate guard while runtime evidence is improved.
