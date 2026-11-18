#!/usr/bin/env bash
# Usage: tools/update-manifest.sh <dist-dir> <build> <release-page> <download-base> <notes-file> > latest.json
#
# Writes the update manifest Canopy's in-app update check reads (format: NecturaLabs/app-updates
# README). <dist-dir> holds the release archives and SHA256SUMS; <build> is the build number the
# manifest's `version` names; <download-base> is the URL the archives are downloadable under;
# <notes-file> holds the release notes (tools/release.py notes).
set -euo pipefail

dist=${1:?dist dir}
version=${2:?build number}
page=${3:?release page}
base=${4:?download base}
notes=$(cat "${5:?notes file}")
case $version in
    '' | 0* | *[!0-9]*) echo "update-manifest.sh: $version is not a build number" >&2; exit 1 ;;
esac

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
# The installer scripts' checksums: an in-app update that runs one (any install not in its own
# folder) or refreshes uninstall.sh from it checks the script against the signed manifest.
installers = {name: sums[name] for name in ("install.sh", "install.ps1") if name in sums}
if len(installers) != 2:
    sys.exit("update-manifest.sh: SHA256SUMS does not list install.sh and install.ps1")
manifest = {
    "version": version,
    "notes": notes.strip(),
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "release_page": page,
    "platforms": platforms,
    "installers": installers,
}
print(json.dumps(manifest, indent=2))
PY
