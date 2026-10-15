#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd -- "$repo_root"

if ! command -v cargo >/dev/null 2>&1; then
    printf 'Cargo is required to build unduplicator.\n' >&2
    exit 1
fi
if ! command -v exiftool >/dev/null 2>&1; then
    printf 'ExifTool is required to run unduplicator. See README.md for setup.\n' >&2
    exit 1
fi

printf 'Testing unduplicator...\n'
cargo test --locked --features test-failpoints
printf 'Building the production release...\n'
cargo build --locked --release
release_binary="${CARGO_TARGET_DIR:-$repo_root/target}/release/unduplicator"

if [[ -n "${UNDUPLICATOR_INSTALL_DIR:-}" ]]; then
    install_dir="$UNDUPLICATOR_INSTALL_DIR"
else
    install_dir="${HOME:?HOME is required for the default install location}/.local/bin"
fi
mkdir -p -- "$install_dir"
install_dir="$(cd -- "$install_dir" && pwd -P)"
target="$install_dir/unduplicator"
candidate="$(mktemp "$install_dir/.unduplicator.XXXXXX")"
trap 'rm -f -- "$candidate"' EXIT
install -m 755 -- "$release_binary" "$candidate"

version="$("$candidate" --version)"
if [[ -e "$target" || -L "$target" ]]; then
    action='Updated'
else
    action='Installed'
fi
mv -f -- "$candidate" "$target"
printf '%s %s at %s\n' "$action" "$version" "$target"

found="$(command -v unduplicator || true)"
if [[ "$found" != "$target" ]]; then
    printf 'To use this installation, add this line to your shell startup file:\n  export PATH=%q:$PATH\n' "$install_dir"
fi
