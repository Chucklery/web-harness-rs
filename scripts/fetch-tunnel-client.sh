#!/bin/sh
set -eu

target="${1:?target triple required}"
destination="${2:?destination directory required}"

version="0.0.14"
checksums_sha256="3c09650be77841e72747951f415b7be32b01be46ca7f534d873fe162c1fb6887"
primary_base_url="https://persistent.oaistatic.com/tunnel-client/v${version}"
fallback_base_url="https://github.com/openai/tunnel-client/releases/download/v${version}"

case "$target" in
  x86_64-apple-darwin) platform="darwin-amd64" ;;
  aarch64-apple-darwin) platform="darwin-arm64" ;;
  x86_64-unknown-linux-gnu) platform="linux-amd64" ;;
  aarch64-unknown-linux-gnu) platform="linux-arm64" ;;
  x86_64-pc-windows-msvc) platform="windows-amd64" ;;
  aarch64-pc-windows-msvc) platform="windows-arm64" ;;
  *)
    echo "unsupported tunnel-client target: $target" >&2
    exit 2
    ;;
esac

stem="tunnel-client-runtime-v${version}-${platform}"
archive="${stem}.zip"
checksums="SHA256SUMS.txt"
primary_url="${primary_base_url}/${archive}"
fallback_url="${fallback_base_url}/${archive}"
temp_dir="$(mktemp -d)"
extract_dir="$temp_dir/extracted"
trap 'rm -rf "$temp_dir"' EXIT

download() {
  output="$1"
  name="$2"
  if ! curl --fail --location --silent --show-error --retry 3 --output "$output" "${primary_base_url}/${name}"; then
    echo "primary OpenAI CDN download failed for $name; trying GitHub release fallback" >&2
    curl --fail --location --silent --show-error --retry 3 --output "$output" "${fallback_base_url}/${name}"
  fi
}

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

download "$temp_dir/$checksums" "$checksums"
actual_checksums_sha256="$(sha256_file "$temp_dir/$checksums")"
if [ "$actual_checksums_sha256" != "$checksums_sha256" ]; then
  echo "tunnel-client checksum manifest mismatch" >&2
  echo "expected: $checksums_sha256" >&2
  echo "actual:   $actual_checksums_sha256" >&2
  exit 3
fi

expected_sha256="$(awk -v file="$archive" '$2 == file {print $1}' "$temp_dir/$checksums")"
if [ -z "$expected_sha256" ]; then
  echo "missing checksum for $archive in upstream SHA256SUMS.txt" >&2
  exit 3
fi

download "$temp_dir/$archive" "$archive"
actual_sha256="$(sha256_file "$temp_dir/$archive")"
if [ "$actual_sha256" != "$expected_sha256" ]; then
  echo "tunnel-client runtime checksum mismatch for $archive" >&2
  echo "expected: $expected_sha256" >&2
  echo "actual:   $actual_sha256" >&2
  exit 3
fi

mkdir -p "$extract_dir"
unzip -q "$temp_dir/$archive" -d "$extract_dir"
mkdir -p "$destination"

case "$platform" in
  windows-*)
    source_binary="$extract_dir/tunnel-client-runtime.exe"
    output_binary="$destination/tunnel-client.exe"
    ;;
  *)
    source_binary="$extract_dir/tunnel-client-runtime"
    output_binary="$destination/tunnel-client"
    ;;
esac

test -f "$source_binary"
cp "$source_binary" "$output_binary"
chmod 755 "$output_binary"

for evidence in LICENSE NOTICE "${stem}-licenses.txt" "${stem}.spdx.json"; do
  test -f "$extract_dir/$evidence"
  cp "$extract_dir/$evidence" "$destination/$evidence"
done

printf '%s\n' "v$version" > "$destination/UPSTREAM_VERSION"
printf '%s\n' "runtime" > "$destination/UPSTREAM_FLAVOR"
printf '%s\n' "$archive" > "$destination/UPSTREAM_ARTIFACT"
printf '%s\n' "$primary_url" > "$destination/UPSTREAM_URL"
printf '%s\n' "$fallback_url" > "$destination/UPSTREAM_FALLBACK_URL"
printf '%s\n' "$expected_sha256" > "$destination/UPSTREAM_SHA256"

printf '%s\n' "$output_binary"
