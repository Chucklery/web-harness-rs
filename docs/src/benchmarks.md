# Performance and 8 GB Gates

The project treats low-memory Macs as a first-class target. Performance claims require measured evidence rather than inference from CI.

## Architecture gates

| Metric | Target |
| --- | ---: |
| Host idle CPU | approximately 0% |
| Host idle RSS | below 60–80 MiB |
| Tunnel + Host idle RSS | below 120–150 MiB |
| Host cold start | below 500 ms |
| local tool dispatch p50 | below 5 ms |
| local tool dispatch p95 | below 20 ms |
| per-Job memory log buffer | at most 256–512 KiB |

## Machine-readable benchmark

Build and collect a JSON report:

~~~bash
cargo build --release --features release-tools
./target/release/web-harness benchmark --workspace . --iterations 10000
~~~

If the official tunnel client is already running, include its PID to measure combined idle RSS:

~~~bash
./target/release/web-harness benchmark   --workspace .   --iterations 10000   --tunnel-pid 12345
~~~

The report then includes tunnel_rss_kib, tunnel_plus_host_rss_kib, and evaluation of the < 150 MiB Tunnel + Host gate.

Or write it to the benchmark evidence directory:

~~~bash
./scripts/benchmark-8gb.sh 10000 benchmarks/latest.json
~~~

The report includes OS/architecture, physical memory when available, local operation latency, MCP-ready startup time, Host RSS/CPU samples, gate targets, and per-gate evaluation.

Unmeasured gates are represented as null; they are never silently treated as passed.

## Current evidence

The repository includes one physical 8 GiB Intel Mac release-mode snapshot. It satisfies the measured Host idle RSS and cold-start targets on that machine, but it is not a complete production gate because Tunnel + Host RSS and Apple Silicon 8 GiB evidence remain outstanding.


## Release-gate aggregation

Aggregate one or more benchmark evidence files:

~~~bash
cargo run --release --features release-tools -- release-gate   --evidence benchmarks/intel.json   --evidence benchmarks/apple-silicon.json
~~~

The output uses only pass, fail, and not_evaluated. A complete pass requires physical approximately-8-GiB evidence from both Intel and Apple Silicon macOS machines, plus measured Tunnel + Host RSS below 150 MiB. Missing evidence never becomes an implicit pass.

`benchmark` and `release-gate` are maintainer/release instrumentation and are intentionally excluded from the default production binary. Enable them explicitly with the `release-tools` Cargo feature.
