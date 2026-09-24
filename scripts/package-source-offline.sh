#!/usr/bin/env bash
set -euo pipefail
repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_path="${1:-$repository_root/dist/versus-source-offline.tar.gz}"
inventory_path="${2:-$repository_root/dist/DEPENDENCY-LICENSES.md}"
cd "$repository_root"
for required in Cargo.toml Cargo.lock package.json package-lock.json npm-cache vendor src-ui src-tauri "$inventory_path"; do
  [[ -e "$required" ]] || { echo "Required offline source bundle input is missing: $required" >&2; exit 1; }
done
staging_directory="$(mktemp -d)"
cleanup() { rm -rf "$staging_directory"; }
trap cleanup EXIT
mkdir -p "$(dirname "$output_path")" "$staging_directory/versus-source-offline"
git archive --format=tar HEAD | tar -xf - -C "$staging_directory/versus-source-offline"
cp "$inventory_path" "$staging_directory/versus-source-offline/DEPENDENCY-LICENSES.md"
tar -C "$staging_directory" -czf "$output_path" versus-source-offline
echo "Created $output_path"
