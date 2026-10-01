#!/usr/bin/env bash
# Usage: tools/install-desktop.sh [--uninstall]
#
# Builds the release binary and installs Canopy for the current user (Linux only):
#   ~/.local/bin/canopy
#   ~/.local/share/applications/io.necturalabs.Canopy.desktop
#   ~/.local/share/metainfo/io.necturalabs.Canopy.metainfo.xml
#   ~/.local/share/icons/hicolor/{64x64,256x256,512x512}/apps/io.necturalabs.Canopy.png
#   ~/.local/share/icons/hicolor/scalable/apps/io.necturalabs.Canopy.svg
# --uninstall removes those files. Make sure ~/.local/bin is on PATH.
set -euo pipefail

APP_ID=io.necturalabs.Canopy
root=$(cd "$(dirname "$0")/.." && pwd)
data=${XDG_DATA_HOME:-$HOME/.local/share}
bin_dir=$HOME/.local/bin
icons=$data/icons/hicolor

files=(
  "$bin_dir/canopy"
  "$data/applications/$APP_ID.desktop"
  "$data/metainfo/$APP_ID.metainfo.xml"
  "$icons/64x64/apps/$APP_ID.png"
  "$icons/256x256/apps/$APP_ID.png"
  "$icons/512x512/apps/$APP_ID.png"
  "$icons/scalable/apps/$APP_ID.svg"
)

refresh_caches() {
  if command -v update-desktop-database >/dev/null; then
    update-desktop-database -q "$data/applications" || true
  fi
  if command -v gtk-update-icon-cache >/dev/null; then
    gtk-update-icon-cache -q -t -f "$icons" || true
  fi
}

if [[ $(uname -s) != Linux ]]; then
  echo "install-desktop.sh installs a freedesktop.org launcher and only runs on Linux" >&2
  exit 1
fi

case ${1:-} in
  --uninstall)
    rm -f -- "${files[@]}"
    refresh_caches
    echo "Removed Canopy from $bin_dir and $data"
    exit 0
    ;;
  "") ;;
  *) echo "usage: $0 [--uninstall]" >&2; exit 2 ;;
esac

cargo build --release -p canopy-app --manifest-path "$root/Cargo.toml"

install -Dm755 "$root/target/release/canopy" "$bin_dir/canopy"
install -Dm644 "$root/packaging/linux/$APP_ID.desktop" "$data/applications/$APP_ID.desktop"
install -Dm644 "$root/packaging/linux/$APP_ID.metainfo.xml" "$data/metainfo/$APP_ID.metainfo.xml"
for size in 64 256 512; do
  install -Dm644 "$root/assets/icon-$size.png" "$icons/${size}x${size}/apps/$APP_ID.png"
done
install -Dm644 "$root/assets/icon.svg" "$icons/scalable/apps/$APP_ID.svg"
refresh_caches

echo "Installed Canopy to $bin_dir/canopy"
case ":$PATH:" in *":$bin_dir:"*) ;; *) echo "note: $bin_dir is not on PATH" ;; esac
