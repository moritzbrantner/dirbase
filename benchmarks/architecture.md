# Architecture performance evidence

This benchmark surface exists to catch architectural performance regressions before they are hidden behind aggregate HTTP throughput numbers.

The primary rule is **local work should stay local**. Dirbase owns mutable resource state, and a small mutation should not require copying or recomputing unrelated resources. Derived views such as schema metadata, indexes, query materializations, and serialized responses should be updated or created only when the operation actually needs them.

## Evidence boundaries

The tools have separate responsibilities:

- **Moonlight** checks deterministic behavior across a baseline and candidate. It compares the architecture contract, not timings.
- **runtime-profiler** captures repeatable wall-time/process evidence for declared workloads.
- `scripts/architecture_benchmark.py` records Dirbase-specific server evidence such as startup time, request distributions, and sampled server RSS. This avoids treating a wrapper process's RSS as Dirbase memory.
- `scripts/summarize_architecture_baseline.py` pairs repeated Runtime Profiler bundles with the corresponding Dirbase-server evidence and produces a descriptive baseline artifact.

Performance numbers are descriptive evidence. This slice intentionally does not introduce a release threshold. Budgets should be calibrated from repeated comparable captures after the scenarios are stable.

The benchmark server starts with the generated fixture as its working directory. That keeps an optional repository-level `dirbase.conf` from changing benchmark semantics or contaminating a baseline/candidate comparison.

## Scenarios

### Hot read: source-size amplification

`hot-read-small.json` serves an 8,000-row resource and `hot-read-window.json` serves a 48,000-row resource. Both repeatedly request the same filtered, sorted eight-row page.

The sixfold source-size change leaves the requested result window unchanged. Comparing the two captures makes whole-resource cloning, repeated parsing, repeated schema work, and pre-window materialization visible as source-size amplification rather than hiding those costs inside aggregate request throughput.
Read validity is now an immutable-snapshot admission concern rather than a per-endpoint stage. When a declared schema is present, `load_resource` validates a snapshot once for the current resource-specific declared-schema generation and records that authority on the cache entry. REST, GraphQL, and SQL readers reuse the admitted snapshot until a mutation installs an unvalidated revision or that resource's declared table changes. The `dirbase_resource_cache_*` and `dirbase_resource_validation_*` counters expose this admission funnel directly so repeated reads can prove they do not rescan every row merely to establish validity again.

`hot-read-declared-small.json` and `hot-read-declared-window.json` repeat the same 8-row query against 8,000 and 48,000-row resources with an explicit declared schema. This pair isolates validation-stage amplification separately from the existing schema-free hot-read source-size pair.

### Localized write: target-size amplification

`localized-write-small.json` patches one row in an 8,000-row resource. `localized-write-large.json` performs the same patch workload on a 48,000-row target with no unrelated resources.

This pair isolates costs that scale with the mutated resource itself: whole-resource clones, serialization, validation, cache-index construction, and changed-table inference. It is intentionally separate from unrelated-state amplification so ownership/copy improvements can be measured without conflating them with global recomputation.

### Localized write: unrelated-state amplification

`localized-write-small.json` patches one row in an 8,000-row resource. `localized-write-ballast.json` performs the same patch workload against the same 8,000-row target while four unrelated 20,000-row resources are present.

The semantic operation is unchanged. A large increase relative to the small fixture is evidence that mutation cost depends on unrelated state. That is the architectural signal to reduce global schema reinference or other whole-world recomputation.

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

The comparison deliberately uses the candidate-owned driver for both binaries. This lets the first adoption compare against a baseline that predates the harness while ensuring the same workload definition drives both targets. The repository also contains `moonlight.eval.toml` for the baseline-compatible project test surface.

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

The five architecture scenarios are:

- `profiles/runtime-profiler/hot-read-small.json`
- `profiles/runtime-profiler/hot-read-window.json`
- `profiles/runtime-profiler/localized-write-small.json`
- `profiles/runtime-profiler/localized-write-large.json`
- `profiles/runtime-profiler/localized-write-ballast.json`

Each measured run writes a final Dirbase-specific JSON record under `.artifacts/architecture/`. Runtime Profiler remains the authoritative cross-revision runtime bundle; the JSON record is supporting evidence for interpreting what the server itself did during that scenario.

## Baseline calibration

The `Architecture Evidence` workflow calibrates the current revision on every relevant push to `main`, on the weekly schedule, and on manual dispatch. It runs at least three complete rounds per scenario by default. Each round keeps an immutable Runtime Profiler bundle and moves the corresponding Dirbase-server record into a matching round directory.

The baseline aggregator refuses to combine evidence when any of these identities drift:

- source revision;
- Runtime Profiler scenario digest;
- Runtime Profiler environment fingerprint;
- paired profiler/server round identity.

For each scenario, the baseline records wall-time samples plus distributions for server startup time, workload time, request median/p95, and sampled server RSS. It also calculates paired distributions for three architectural amplification signals: read source size, write target size, and write unrelated state.

The workflow emits:

- `.artifacts/architecture-baseline.json` as the machine-readable `dirbase/architecture-baseline/v1` artifact;
- `.artifacts/architecture-baseline.md` as the review summary;
- all source Runtime Profiler bundles and paired Dirbase-server evidence used to build the baseline.

The initial baseline is deliberately marked `descriptive` with `thresholds_enabled: false`. Promote concrete budgets only after repeated main-branch calibration shows sufficiently stable variance for a metric. Until then, compare optimization branches against the exact baseline scenario digests and environment fingerprint rather than treating an arbitrary percentage as a release rule.

## Architectural interpretation

When using these results to reengineer Dirbase, prefer changes that reduce multiplicative costs rather than micro-optimizing individual functions. In particular, investigate:

- full `serde_json::Value` clones on mutation or response paths;
- schema inference that rescans resources unaffected by a delta;
- indexes or derived views rebuilt when they could be incrementally updated or invalidated narrowly;
- query stages that materialize full collections before applying a small output window;
- serialization boundaries that copy data simply to satisfy ownership layering.

Do not optimize by weakening persistence, query semantics, schema correctness, or concurrency safety. Moonlight's semantic contract remains the baseline/candidate guard while runtime evidence is improved.
