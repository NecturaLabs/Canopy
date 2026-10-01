#!/usr/bin/env bash
# Usage:
#   tools/release-package.sh check-version            # tag ($GITHUB_REF_NAME) matches Cargo.toml
#   tools/release-package.sh package <target> <tar.gz|zip>
#   tools/release-package.sh notes                    # this version's CHANGELOG.md section
#
# The release workflow's steps, kept in one script so the self-hosted and GitHub-hosted builds
# package identically and the steps can be tried locally.
set -euo pipefail

cd "$(dirname "$0")/.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
tag=${GITHUB_REF_NAME:-v$version}

case "${1:-}" in
  check-version)
    if [[ "v$version" != "$tag" ]]; then
      echo "Tag $tag does not match workspace version $version" >&2
      exit 1
    fi
    ;;
  package)
    target=${2:?target}
    archive=${3:?archive}
    name="canopy-${tag#v}-$target"
    rm -rf dist && mkdir -p "dist/$name"
    bin="target/$target/release/canopy"
    [[ -f "$bin.exe" ]] && bin="$bin.exe"
    cp "$bin" README.md LICENSE "dist/$name/"
    if [[ "$target" == *linux* ]]; then
      cp packaging/linux/io.necturalabs.Canopy.desktop \
         packaging/linux/io.necturalabs.Canopy.metainfo.xml "dist/$name/"
      cp assets/icon.svg "dist/$name/io.necturalabs.Canopy.svg"
      cp assets/icon-256.png "dist/$name/io.necturalabs.Canopy.png"
    fi
    cd dist
    if [[ "$archive" == zip ]]; then
      7z a -tzip "$name.zip" "$name" > /dev/null
    else
      tar -czf "$name.tar.gz" "$name"
    fi
    rm -rf "$name"
    ;;
  notes)
    notes=$(mktemp)
    trap 'rm -f "$notes"' EXIT
    awk -v v="${tag#v}" '
      $0 ~ "^## \\[" v "\\]" { found = 1; next }
      found && /^## \[/ { exit }
      found && /^\[.*\]: / { exit }
      found { print }
    ' CHANGELOG.md > "$notes"
    if [[ ! -s $notes ]]; then
      echo "No CHANGELOG.md section for ${tag#v}" >&2
      exit 1
    fi
    cat "$notes"
    ;;
  *)
    sed -n '2,6p' "$0" >&2
    exit 2
    ;;
esac
