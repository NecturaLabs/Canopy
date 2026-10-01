#!/usr/bin/env bash
# Usage: tools/update-manifest.sh <dist-dir> <version> <release-page> <download-base> > latest.json
#
# Writes the update manifest Canopy's in-app update check reads (format: NecturaLabs/app-updates
# README). <dist-dir> holds the release archives and SHA256SUMS; <download-base> is the URL the
# archives are downloadable under. Release notes come from this version's CHANGELOG.md section.
set -euo pipefail

dist=${1:?dist dir}
version=${2:?version}
page=${3:?release page}
base=${4:?download base}
root=$(cd "$(dirname "$0")/.." && pwd)

notes=$(GITHUB_REF_NAME="v$version" "$root/tools/release-package.sh" notes)

python3 - "$dist" "$version" "$page" "$base" "$notes" <<'PY'
import datetime, json, os, sys
dist, version, page, base, notes = sys.argv[1:6]
sums = {}
with open(os.path.join(dist, "SHA256SUMS")) as f:
    for line in f:
        digest, name = line.split(maxsplit=1)
        sums[name.strip().lstrip("*")] = digest
targets = {
    "x86_64-unknown-linux-gnu": "linux-x86_64",
    "aarch64-unknown-linux-gnu": "linux-aarch64",
    "x86_64-pc-windows-msvc": "windows-x86_64",
    "aarch64-apple-darwin": "darwin-aarch64",
    "x86_64-apple-darwin": "darwin-x86_64",
}
platforms = {}
for name, digest in sorted(sums.items()):
    for target, key in targets.items():
        if target in name:
            platforms[key] = {"url": f"{base.rstrip('/')}/{name}", "sha256": digest}
manifest = {
    "version": version,
    "notes": notes.strip(),
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "release_page": page,
    "platforms": platforms,
}
print(json.dumps(manifest, indent=2))
PY
