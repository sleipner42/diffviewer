#!/bin/sh
set -eu

repo="sleipner42/diffviewer"
install_dir="${DIFFVIEWER_INSTALL_DIR:-$HOME/.local/bin}"
base_url="https://github.com/$repo/releases/latest/download"

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin) asset="diffviewer-macos-universal.tar.gz" ;;
  Linux)
    case "$arch" in
      x86_64|amd64) asset="diffviewer-linux-x86_64.tar.gz" ;;
      aarch64|arm64) asset="diffviewer-linux-aarch64.tar.gz" ;;
      *) echo "no prebuilt diffviewer for Linux/$arch; build from source with cargo" >&2; exit 1 ;;
    esac
    ;;
  *) echo "diffviewer ships macOS and Linux builds only" >&2; exit 1 ;;
esac

if command -v sha256sum >/dev/null 2>&1; then
  checksum="sha256sum -c"
else
  checksum="shasum -a 256 -c"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "downloading $base_url/$asset"
curl -fsSL "$base_url/$asset" -o "$tmp/$asset"
curl -fsSL "$base_url/$asset.sha256" -o "$tmp/$asset.sha256"
(cd "$tmp" && $checksum "$asset.sha256" >/dev/null)
tar -xzf "$tmp/$asset" -C "$tmp"

mkdir -p "$install_dir"
install -m 755 "$tmp/diffviewer" "$install_dir/diffviewer"
install -m 755 "$tmp/dw" "$install_dir/dw"

version="$("$install_dir/diffviewer" --version)"
echo "installed $version to $install_dir"

case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) echo "add it to your PATH: export PATH=\"$install_dir:\$PATH\"" ;;
esac

resolved="$(command -v diffviewer 2>/dev/null || true)"
if [ -n "$resolved" ] && [ "$resolved" != "$install_dir/diffviewer" ]; then
  echo "note: $resolved comes first on your PATH and shadows the new install"
fi
