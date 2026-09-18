#!/bin/sh
set -eu

target="${1:?target triple required}"
version="${2:?version required}"
binary="target/${target}/release/web-harness"
archive="web-harness-${version}-${target}.tar.gz"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

test -x "$binary"
mkdir -p dist
cp "$binary" "$stage/web-harness"
mkdir -p "$stage/libexec/web-harness"
./scripts/fetch-tunnel-client.sh "$target" "$stage/libexec/web-harness"
cp LICENSE NOTICE README.md THIRD_PARTY_NOTICES.md "$stage/"
tar -C "$stage" -czf "dist/$archive" .
shasum -a 256 "dist/$archive" | sed 's#  dist/#  #' > "dist/$archive.sha256"
printf '%s\n' "dist/$archive"
