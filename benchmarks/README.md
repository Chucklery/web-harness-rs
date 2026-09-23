# Benchmark evidence

Benchmark JSON files in this directory are evidence snapshots, not universal performance claims.

The architecture acceptance targets include:

- Host idle RSS below 60–80 MiB
- Tunnel + Host idle RSS below 120–150 MiB
- Host cold start below 500 ms
- local tool dispatch p95 below 20 ms
- bounded Job output memory

Run a release-mode benchmark with:

~~~bash
./scripts/benchmark-8gb.sh 10000 benchmarks/latest.json
~~~

The report records machine OS/architecture/physical memory, local operation latency samples, MCP-ready startup time, process RSS/CPU samples, target gates, and explicit null values for gates that were not measured.

## Checked-in evidence

`2026-09-18-intel-mac-8gb.json` and `2026-09-23-intel-mac-8gb.json` were collected from this physical x86_64 Mac reporting exactly 8 GiB RAM using release binaries. The newer snapshot uses 10,000 iterations, measures search successfully, and records Host RSS of 1,060 KiB and MCP-ready startup of 3 ms. Its patch p95 is 568 µs. It does not include a running official tunnel client, so Tunnel + Host RSS remains unevaluated; Apple Silicon evidence is also still missing.

The 2026-09-18 snapshot is an earlier partial baseline:

- Host idle RSS gate: measured
- cold-start gate: measured
- read/patch/exec latency: measured
- search latency: not measured because rg was unavailable in the runner PATH
- Tunnel + Host RSS: not measured
- Apple Silicon 8 GiB: not measured

The 2026-09-23 snapshot also measures search and records sub-microsecond timings rounded up to 1 µs rather than truncated to zero. Its exec timing includes process spawn and exit, so it is not a measurement of MCP dispatch overhead alone.

Do not use a single machine snapshot as a cross-machine performance guarantee.
