# ccc Benchmark

Use `scripts/benchmark_ccc_search.py` to compare three repo-search workflows on the same fixed task
set:

- `ccc`: semantic search only
- `rg`: regex search only
- `hybrid`: `ccc` candidate retrieval followed by `rg` reranking inside the candidate set

The benchmark currently covers ten repository-specific tasks such as locating release validation
logic, package version declarations, PTY spawn code, paste-burst docs, and clawbot reaction
cleanup.

## Scoring

Each method can earn up to 4 points per task:

- `1` point if the method's first returned result already contains the needed answer
- `1` point if the method uses the fewest search-input tokens among methods that complete the task
- `1` point if the method completes the task
- `1` point if the method finishes in at most two search queries

Token counting measures search input only:

- semantic query text
- regex patterns
- path filters or file scopes

Shell boilerplate and file-reading follow-up are not counted.

## Usage

Run all methods:

```bash
python3 scripts/benchmark_ccc_search.py
```

Refresh the `ccc` index first:

```bash
python3 scripts/benchmark_ccc_search.py --refresh-ccc
```

Limit to one method or a subset of tasks:

```bash
python3 scripts/benchmark_ccc_search.py --method hybrid --task T5 --task T6
```

Write machine-readable output:

```bash
python3 scripts/benchmark_ccc_search.py --json-output /tmp/ccc-benchmark.json
```

## Notes

- This benchmark measures fixed search workflows, not the theoretical best possible query a human
  could invent after unlimited retries.
- The `hybrid` method deliberately tests whether `rg` can improve ranking after `ccc` retrieves a
  plausible candidate set.
- Pure filename tasks may still rely on path hints during hybrid reranking because content-only
  reranking is not enough when the answer is the file itself.
