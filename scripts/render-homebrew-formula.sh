#!/bin/sh
set -eu

version="${1:?version required, without leading v}"
repository="${2:?GitHub repository required, e.g. owner/repo}"
dist_dir="${3:-dist}"
output="${4:-dist/web-harness.rb}"

checksum() {
  file="$1"
  test -f "$file"
  awk '{print $1}' "$file"
}

mac_arm="$(checksum "$dist_dir/web-harness-$version-aarch64-apple-darwin.tar.gz.sha256")"
mac_x64="$(checksum "$dist_dir/web-harness-$version-x86_64-apple-darwin.tar.gz.sha256")"
linux_x64="$(checksum "$dist_dir/web-harness-$version-x86_64-unknown-linux-gnu.tar.gz.sha256")"

sed \
  -e "s|@REPOSITORY@|$repository|g" \
  -e "s|@VERSION@|$version|g" \
  -e "s|@MAC_ARM_SHA256@|$mac_arm|g" \
  -e "s|@MAC_X64_SHA256@|$mac_x64|g" \
  -e "s|@LINUX_X64_SHA256@|$linux_x64|g" \
  packaging/homebrew/web-harness.rb.template > "$output"

printf '%s\n' "$output"
