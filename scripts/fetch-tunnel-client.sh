#!/bin/sh
set -eu

target="${1:?target triple required}"
destination="${2:?destination directory required}"

version="0.0.14"
base_url="https://github.com/openai/tunnel-client/releases/download/v${version}"

case "$target" in
  x86_64-apple-darwin)
    platform="darwin-amd64"
    expected_sha256="75e10be774184fb42189e347b16eb6bc9fb0780135d8af714d34e30ce068dc53"
    ;;
  aarch64-apple-darwin)
    platform="darwin-arm64"
    expected_sha256="b540493c5bdbcdbb755700c8e2e16597e28b1569e425007e0f73111047bd6a64"
    ;;
  x86_64-unknown-linux-gnu)
    platform="linux-amd64"
    expected_sha256="15bd17e805cad39d412199115bb9e10a978dd35258a114cdf25dd2ae6681c7d3"
    ;;
  *)
    echo "unsupported tunnel-client target: $target" >&2
    exit 2
    ;;
esac

archive="tunnel-client-v${version}-${platform}.zip"
url="${base_url}/${archive}"
temp_dir="$(mktemp -d)"
trap 'rm -rf "$temp_dir"' EXIT

curl --fail --location --silent --show-error --output "$temp_dir/$archive" "$url"
actual_sha256="$(shasum -a 256 "$temp_dir/$archive" | awk '{print $1}')"
if [ "$actual_sha256" != "$expected_sha256" ]; then
  echo "tunnel-client checksum mismatch for $archive" >&2
  echo "expected: $expected_sha256" >&2
  echo "actual:   $actual_sha256" >&2
  exit 3
fi

mkdir -p "$destination"
unzip -q "$temp_dir/$archive" -d "$destination"
printf '%s\n' "v$version" > "$destination/UPSTREAM_VERSION"
printf '%s\n' "$url" > "$destination/UPSTREAM_URL"

chmod 755 "$destination/tunnel-client"
if [ -f "$destination/cloudflared" ]; then
  chmod 755 "$destination/cloudflared"
fi

printf '%s\n' "$destination/tunnel-client"
