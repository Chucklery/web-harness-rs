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
cp LICENSE NOTICE README.md "$stage/"
tar -C "$stage" -czf "dist/$archive" .
shasum -a 256 "dist/$archive" | sed 's#  dist/#  #' > "dist/$archive.sha256"
printf '%s\n' "dist/$archive"
