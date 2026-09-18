#!/bin/sh
set -eu

target="${1:?target triple required}"
destination="${2:?destination directory required}"

version="0.0.14"
primary_base_url="https://persistent.oaistatic.com/tunnel-client/v${version}"
fallback_base_url="https://github.com/openai/tunnel-client/releases/download/v${version}"

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
  aarch64-unknown-linux-gnu)
    platform="linux-arm64"
    expected_sha256="2de3fb879a18edb847e0313592c912f1983685488290a7fdba7ac403e6a4fb0a"
    ;;
  x86_64-pc-windows-msvc)
    platform="windows-amd64"
    expected_sha256="784ab8da7b5a88f0109f1fd8aaf0a1c86067430b896dddf307ef7e3cc49fa1a5"
    ;;
  aarch64-pc-windows-msvc)
    platform="windows-arm64"
    expected_sha256="fa775db8897df543dd4ba66404f69492a2acfbc6a291f10df27aced064a16568"
    ;;
  *)
    echo "unsupported tunnel-client target: $target" >&2
    exit 2
    ;;
esac

archive="tunnel-client-v${version}-${platform}.zip"
primary_url="${primary_base_url}/${archive}"
fallback_url="${fallback_base_url}/${archive}"
temp_dir="$(mktemp -d)"
trap 'rm -rf "$temp_dir"' EXIT

if ! curl --fail --location --silent --show-error --retry 3 --output "$temp_dir/$archive" "$primary_url"; then
  echo "primary OpenAI CDN download failed; trying GitHub release fallback" >&2
  curl --fail --location --silent --show-error --retry 3 --output "$temp_dir/$archive" "$fallback_url"
fi

if command -v shasum >/dev/null 2>&1; then
  actual_sha256="$(shasum -a 256 "$temp_dir/$archive" | awk '{print $1}')"
else
  actual_sha256="$(sha256sum "$temp_dir/$archive" | awk '{print $1}')"
fi
if [ "$actual_sha256" != "$expected_sha256" ]; then
  echo "tunnel-client checksum mismatch for $archive" >&2
  echo "expected: $expected_sha256" >&2
  echo "actual:   $actual_sha256" >&2
  exit 3
fi

mkdir -p "$destination"
unzip -q "$temp_dir/$archive" -d "$destination"
printf '%s\n' "v$version" > "$destination/UPSTREAM_VERSION"
printf '%s\n' "$primary_url" > "$destination/UPSTREAM_URL"
printf '%s\n' "$fallback_url" > "$destination/UPSTREAM_FALLBACK_URL"

if [ -f "$destination/tunnel-client" ]; then
  chmod 755 "$destination/tunnel-client"
fi
if [ -f "$destination/cloudflared" ]; then
  chmod 755 "$destination/cloudflared"
fi

if [ -f "$destination/tunnel-client.exe" ]; then
  printf '%s\n' "$destination/tunnel-client.exe"
else
  printf '%s\n' "$destination/tunnel-client"
fi
