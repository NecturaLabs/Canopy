#!/usr/bin/env bash
# Usage: tools/vendor-eframe.sh [--check]
#
# Rebuilds vendor/eframe from the published eframe crate plus vendor/eframe.patch (see
# vendor/README.md). With --check it builds the same tree in a temporary folder and fails when it
# differs from the committed vendor/eframe. Needs curl, tar, git and diff; downloads the crate
# from crates.io's download host and refuses it unless its SHA-256 matches the one pinned here.
set -euo pipefail

# Raising VERSION means: take the new crate's checksum from Cargo.lock (before the patch entry
# replaces it) or `cargo info`, regenerate vendor/eframe.patch against it and rerun this script.
VERSION="0.36.2"
SHA256="55ad41fac6e3149abb4866e93df37dbd33baa95db525c3368c944584e95a5dc2"

root="$(cd "$(dirname "$0")/.." && pwd)"
check=false
case "${1:-}" in
  "") ;;
  --check) check=true ;;
  *) echo "usage: tools/vendor-eframe.sh [--check]" >&2; exit 2 ;;
esac

tmp="$(mktemp -d "${TMPDIR:-/tmp}/canopy-vendor.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "https://static.crates.io/crates/eframe/eframe-$VERSION.crate" -o "$tmp/eframe.crate"
if command -v sha256sum >/dev/null; then
  actual="$(sha256sum "$tmp/eframe.crate" | cut -d' ' -f1)"
else
  actual="$(shasum -a 256 "$tmp/eframe.crate" | cut -d' ' -f1)"
fi
if [ "$actual" != "$SHA256" ]; then
  echo "eframe-$VERSION.crate has SHA-256 $actual, expected $SHA256" >&2
  exit 1
fi

tar -xzf "$tmp/eframe.crate" -C "$tmp"
# Only what cargo needs to build the crate: the manifest, the sources and the icon they embed.
mkdir -p "$tmp/out/data"
cp "$tmp/eframe-$VERSION/Cargo.toml" "$tmp/out/Cargo.toml"
cp -R "$tmp/eframe-$VERSION/src" "$tmp/out/src"
cp "$tmp/eframe-$VERSION/data/icon.png" "$tmp/out/data/icon.png"
# The ceiling keeps git from finding a repository above the temporary folder and applying the
# patch relative to that instead.
(cd "$tmp/out" && GIT_CEILING_DIRECTORIES="$tmp" git apply "$root/vendor/eframe.patch")

if $check; then
  diff -r "$tmp/out" "$root/vendor/eframe"
  echo "vendor/eframe is eframe $VERSION plus vendor/eframe.patch"
else
  rm -rf "$root/vendor/eframe"
  cp -R "$tmp/out" "$root/vendor/eframe"
  echo "vendor/eframe rebuilt from eframe $VERSION plus vendor/eframe.patch"
fi
