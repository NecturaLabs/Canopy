# Packaging

Canopy ships as a single `canopy` binary. Everything a desktop needs besides the binary lives
here and in `assets/`.

| File | Purpose |
|---|---|
| `linux/io.necturalabs.Canopy.desktop` | freedesktop.org desktop entry. The app ID, the icon name and `StartupWMClass` are all `io.necturalabs.Canopy`, which must match the Wayland app ID / X11 `WM_CLASS` the binary sets. |
| `linux/io.necturalabs.Canopy.metainfo.xml` | AppStream metadata for software centres. Add a `<release>` entry for every version. |
| `../assets/icon.svg`, `../assets/icon-{64,256,512}.png` | Application icon, installed under `hicolor/<size>/apps/io.necturalabs.Canopy.png`. |

## Current channels

- **From source**: `tools/install-desktop.sh` builds a release binary and installs the binary,
  desktop entry, metainfo and icons under `~/.local`.
- **GitHub releases**: `.github/workflows/release.yml` builds on every `v*` tag. The Linux archive
  (`canopy-<version>-x86_64-unknown-linux-gnu.tar.gz`) contains the binary, the desktop entry, the
  metainfo, the icon and the README; macOS (Apple silicon) and Windows builds are a bare binary in
  an archive.

## Validation

```sh
desktop-file-validate packaging/linux/io.necturalabs.Canopy.desktop
appstreamcli validate --no-net packaging/linux/io.necturalabs.Canopy.metainfo.xml
```

## Runtime dependencies

`git` 2.30 or newer on `PATH`; on Linux, a Wayland or X11 session with OpenGL 3.3 or OpenGL ES 3.0,
`libxkbcommon` and `libGL`/`libEGL`, and an XDG desktop portal for the folder picker. libgit2 is
compiled in without HTTPS or SSH, so there is no OpenSSL or libssh2 dependency.

## Planned

Tracked in `docs/backlog.md`: Flatpak (Flathub manifest with `--filesystem=home` and host `git`
via `flatpak-spawn` or a bundled git), AUR (`canopy-bin` and a source package), `.deb` with
`cargo-deb`, `.rpm` with `cargo-generate-rpm`, AppImage, a signed and notarised macOS `.app`, and a
Windows installer.
