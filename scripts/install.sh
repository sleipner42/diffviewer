#!/bin/sh
set -eu

repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
bin_dir="${CARGO_HOME:-$HOME/.cargo}/bin"

cargo install --path "$repo_dir" --locked
install -m 755 "$repo_dir/scripts/dw" "$bin_dir/dw"

echo "installed diffviewer and dw into $bin_dir"
