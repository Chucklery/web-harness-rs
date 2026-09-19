#!/bin/sh
set -eu

iterations="${1:-10000}"
output="${2:-benchmarks/latest.json}"

mkdir -p "$(dirname "$output")"
cargo build --release --features release-tools
./target/release/web-harness benchmark --workspace . --iterations "$iterations" > "$output"

printf 'benchmark written to %s\n' "$output"
