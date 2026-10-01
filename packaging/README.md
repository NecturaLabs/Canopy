# Packaging

Canopy ships as one `canopy` binary (a `Canopy.app` bundle on macOS). Everything a desktop needs
besides the binary lives here and in `assets/`.

| File | Purpose |
|---|---|
| `install.sh` | One-line installer for Linux and macOS (POSIX `sh`). Published as `canopy/install.sh` in NecturaLabs/app-updates. |
| `install.ps1` | One-line installer for Windows (Windows PowerShell 5.1 and PowerShell 7). Published as `canopy/install.ps1`. |
| `test-install.sh` | End-to-end test of `install.sh` against a fake feed on 127.0.0.1 in a temporary `HOME` (CI, `tooling` job). |
| `linux/io.necturalabs.Canopy.desktop` | freedesktop.org desktop entry. The app ID, the icon name and `StartupWMClass` are all `io.necturalabs.Canopy`, which must match the Wayland app ID / X11 `WM_CLASS` the binary sets. The installer rewrites `Exec=` to the absolute path of the installed binary. |
| `linux/io.necturalabs.Canopy.metainfo.xml` | AppStream metadata for software centres. Add a `<release>` entry for every version. |
| `macos/Info.plist` | `Info.plist` of `Canopy.app` (bundle id `io.necturalabs.Canopy`); the release fills in `@VERSION@` and `@BUNDLE_VERSION@`. |
| `../assets/icon.svg`, `../assets/icon-{64,256,512}.png` | Application icon. Linux archives carry the SVG and the 256 px PNG; the macOS `.icns` is generated from the 512 px PNG during the release. |

## The one-line installers

```sh
curl -fsSL https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.sh | sh
```

```powershell
irm https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.ps1 | iex
```

These URLs are stable: everything Canopy publishes in the shared, public NecturaLabs/app-updates
repository sits under its `canopy/` folder (scripts, `latest.json`, `beta.json`) or in releases
tagged `canopy-v<version>`. Nothing, in the scripts, the docs or the app, uses the repository-wide
`releases/latest`, which may belong to another app.

### What the installers do

1. Detect the platform: OS and architecture (an x86_64 shell under Rosetta still gets the Apple
   silicon build), and on Linux glibc 2.35 or newer (musl is refused).
2. Read `canopy/latest.json` (`--beta` also reads `beta.json` and takes the newer version). With
   `--version X` they use the manifest when it names X, else the `SHA256SUMS` of release
   `canopy-vX`.
3. Accept only `https://github.com/NecturaLabs/app-updates/releases/download/canopy-v<version>/canopy-<version>-<target>.<ext>`
   and a 64-digit SHA-256. Download over HTTPS only (`curl --proto =https --tlsv1.2`, also for
   redirects; GNU wget with `--https-only` as the fallback; `Invoke-WebRequest` with TLS 1.2 on
   Windows), verify the checksum and stop on any mismatch before touching the installation.
4. Unpack into a staging folder inside the install folder, then swap the old content out and the
   new in; on any failure the old version is put back. A first install that fails leaves nothing.
5. Register the app with the desktop (below), print where everything went, and report whether
   `git` and `git-lfs` are on `PATH` with an install hint for the system. They never install git,
   never run what they downloaded, and never use `sudo` or administrator rights.

Re-running upgrades or reinstalls (idempotent). A lock folder stops two installers running at
once. `--uninstall` (Windows: `-Uninstall`, or Apps & features) removes exactly what the
installer created, checking each item is still its own (a symlink pointing into the install
folder, a desktop entry carrying `X-Canopy-Install-Dir`, icons identical to the installed ones,
the shortcut targeting the installed exe). `--purge` also deletes Canopy's settings, logs and
caches after asking (or with `--yes`); it lists the folders first. Tokens in the system keychain
are left to Canopy's Settings → Accounts.

### Layout

| | Install folder (`CANOPY_INSTALL_DIR` overrides) | Outside it |
|---|---|---|
| Linux | `~/.local/opt/canopy/`: `bin/canopy`, `share/applications/io.necturalabs.Canopy.desktop`, `share/icons/hicolor/{256x256,scalable}/apps/`, `share/metainfo/`, `LICENSE`, `README.md`, `legal/` (privacy policy, terms, `third-party-notices.txt`), `VERSION`, `uninstall.sh`, `.canopy-install` (receipt) | `~/.local/bin/canopy` symlink (`--no-path` skips it; a warning when `~/.local/bin` is not on `PATH`); copies of the desktop entry and icons in `${XDG_DATA_HOME:-~/.local/share}/applications` and `icons/hicolor`; `update-desktop-database` runs if present, and an existing GTK icon cache is refreshed |
| macOS | `~/Applications/Canopy.app` (the variable names the `.app`) | `~/.local/bin/canopy` symlink to `Contents/MacOS/canopy` (`--no-path` skips it) |
| Windows | `%LOCALAPPDATA%\Programs\Canopy\`: `canopy.exe`, `README.md`, `LICENSE`, `legal\`, `VERSION`, `uninstall.ps1`, `.canopy-install` | Start Menu shortcut `Canopy.lnk`; `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\io.necturalabs.Canopy`; the folder appended to the user `PATH` (`-NoPath` skips it; removed on uninstall only if the installer added it) |

`uninstall.sh` and `uninstall.ps1` are copies of the installer; run under that name they
uninstall the folder they sit in. Canopy's own data folders are separate and untouched unless
`--purge`; their list is in one marked block near the top of each script (`purge_targets`,
`Get-PurgeTargets`), covering the old `eframe::storage_dir` folders and the vendor-specific ones.

### Testing

`packaging/test-install.sh` (Linux) builds fake releases with a dummy `canopy`, serves them with
`python3 -m http.server` on 127.0.0.1 and runs the installer in a temporary `HOME` through the
test-only `CANOPY_INSTALL_BASE_URL`, which the scripts accept only as
`http://127.0.0.1:<port>` or `http://localhost:<port>`. It covers install, reinstall, upgrade,
checksum and tampering refusal, a manifest URL outside Canopy's releases, `--version`, `--beta`,
a missing platform, `--no-path`, a foreign `~/.local/bin/canopy`, uninstall (via `uninstall.sh`
and `--uninstall --purge`), a piped `sh -s` install, a foreign non-empty folder and a path with a
space. `CANOPY_TEST_SHELL=bash` runs it under another shell (dash by default). On Windows, CI
parses `install.ps1` with Windows PowerShell 5.1 and PowerShell 7, checks its argument handling
and runs PSScriptAnalyzer where installed; the macOS path of `install.sh` is not exercised in CI.

## Release archives

`.github/workflows/release.yml` builds `canopy-<version>-<target>.tar.gz` (Linux, macOS) or
`.zip` (Windows), each holding one folder `canopy-<version>-<target>/` with `README.md`,
`LICENSE`, `legal/` and:

- Linux (`x86_64-unknown-linux-gnu`; `aarch64-unknown-linux-gnu` cross-compiled, experimental):
  `canopy`, the desktop entry, the metainfo, `io.necturalabs.Canopy.svg` and `.png`.
- macOS (`aarch64-apple-darwin`; `x86_64-apple-darwin` cross-compiled, experimental):
  `Canopy.app`, ad-hoc signed, not notarised. The installer downloads with curl, so Gatekeeper
  does not quarantine it; an archive downloaded in a browser needs right-click → Open the first
  time.
- Windows (`x86_64-pc-windows-msvc`): `canopy.exe`.

An experimental build may fail without holding back the release; the installer then reports
that the platform has no build. The release also attaches `install.sh`, `install.ps1` and
`SHA256SUMS`, and a stable release commits the two scripts to `canopy/` in app-updates next to
`latest.json`. Pre-releases never replace them.

## Other channels

- **From source**: `tools/install-desktop.sh` builds a release binary and installs the binary,
  desktop entry, metainfo and icons under `~/.local`. It writes `~/.local/bin/canopy` as a file,
  which the one-line installer then leaves alone (with a warning).

## Validation

```sh
desktop-file-validate packaging/linux/io.necturalabs.Canopy.desktop
appstreamcli validate --no-net packaging/linux/io.necturalabs.Canopy.metainfo.xml
plutil -lint packaging/macos/Info.plist   # macOS
```

## Runtime dependencies

`git` 2.30 or newer on `PATH`; on Linux, a Wayland or X11 session with OpenGL 3.3 or OpenGL ES 3.0,
`libxkbcommon` and `libGL`/`libEGL`, and an XDG desktop portal for the folder picker. libgit2 is
compiled in without HTTPS or SSH, so there is no OpenSSL or libssh2 dependency. Canopy started
from Finder sees only the system `PATH` (`/usr/bin`), so on macOS it finds the Xcode Command Line
Tools' git there.

## Planned

Tracked in `docs/backlog.md`: Flatpak (Flathub manifest with `--filesystem=home` and host `git`
via `flatpak-spawn` or a bundled git), AUR (`canopy-bin` and a source package), `.deb` with
`cargo-deb`, `.rpm` with `cargo-generate-rpm`, AppImage, a signed and notarised macOS app, and a
signed Windows binary.
