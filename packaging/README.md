# Packaging

Canopy ships as one `canopy` binary (a `Canopy.app` bundle on macOS). Everything a desktop needs
besides the binary lives here and in `assets/`.

| File | Purpose |
|---|---|
| `install.sh` | One-line installer for Linux and macOS (POSIX `sh`). Published as `canopy/install.sh` in NecturaLabs/app-updates. |
| `install.ps1` | One-line installer for Windows (Windows PowerShell 5.1 and PowerShell 7). Published as `canopy/install.ps1`. |
| `test-install.sh` | End-to-end test of `install.sh` against a fake feed on 127.0.0.1 in a temporary `HOME`, on Linux and macOS (CI: the `tooling` job and the macOS test job). |
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

These work from the first stable release that ships the installers: the release commits them to
app-updates. Until then the addresses answer 404, and `curl -fsSL … | sh` exits 0 with nothing
installed (`curl` prints the error; `sh` reads an empty script).

These URLs are stable: everything Canopy publishes in the shared, public NecturaLabs/app-updates
repository sits under its `canopy/` folder (scripts, `latest.json`, `beta.json`) or in releases
tagged `canopy-v<version>`. Nothing, in the scripts, the docs or the app, uses the repository-wide
`releases/latest`, which may belong to another app.

### What the installers do

1. Detect the platform: OS and architecture (an x86_64 shell under Rosetta still gets the Apple
   silicon build), and on Linux glibc 2.35 or newer (musl is refused). `install.sh` checks for
   `curl` or GNU `wget` before downloading anything, and says so when neither is there.
2. Read `canopy/latest.json` (`--beta` also reads `beta.json` and takes the newer version). With
   `--version X` they use the manifest when it names X, else the `SHA256SUMS` of release
   `canopy-vX`. Without `--version` they never install a version older than the installed one:
   they stop and suggest `--beta` or `--version X` (so a beta stays until asked otherwise).
3. Accept only `https://github.com/NecturaLabs/app-updates/releases/download/canopy-v<version>/canopy-<version>-<target>.<ext>`
   and a 64-digit SHA-256, verify the checksum and stop on any mismatch before touching the
   installation. Downloads are HTTPS only, TLS 1.2 or newer:
   - `curl -q --proto =https --proto-redir =https --tlsv1.2`: no `~/.curlrc`, and a redirect to
     anything but HTTPS is refused.
   - GNU `wget --https-only --secure-protocol=TLSv1_2`, with `--no-config` where the wget has it
     (1.17 and later). wget has no switch for the scheme of a redirect and follows the server's
     redirects as they come; `--https-only` only governs recursive downloads. The first request
     always goes to GitHub over verified TLS, and GitHub's redirects go to HTTPS; the archive is
     checked against the SHA-256 from the manifest either way.
   - PowerShell: `install.ps1` follows redirects itself (`HttpWebRequest` with
     `AllowAutoRedirect` off, at most five) and refuses any hop that is not HTTPS, because Windows
     PowerShell 5.1 would follow one to plain HTTP. On 5.1 it asks for TLS 1.2 and puts the
     process setting back when it ends.
4. Unpack into a staging folder inside the install folder, then swap in two phases: move the old
   entries aside into `.previous.*`, then the new ones in. A failure, or Ctrl+C (the cleanup trap
   on Linux and macOS, `finally` on Windows), undoes exactly the phase reached: it never deletes
   an old entry, removes only names that came from the staging folder, and never moves an old
   entry over an existing name. If something cannot be put back, it says where the rest is. A
   first install that fails leaves nothing, not even the parent folders it created (such as
   `~/.local/opt`); a pre-existing empty folder is left empty and unclaimed. The receipt is
   written only once the swap succeeded.
5. Register the app with the desktop (below), print where everything went, and report whether
   `git` and `git-lfs` are on `PATH` with an install hint for the system. They never install git,
   never run what they downloaded, and never use `sudo` or administrator rights.

On Linux the install folder is used by its real path (`pwd -P`, so a symlinked `HOME` works): the
symlink, the desktop entry's `X-Canopy-Install-Dir` marker and the receipt's `root=` line all name
the folder exactly as the installed `uninstall.sh` sees itself.

Re-running upgrades or reinstalls (idempotent). A lock folder (`.lock` in the install folder;
`.Canopy-install.lock` next to the app on macOS) stops two installs or uninstalls running at
once. Nothing outside the folder is written where something else owns the name: the
`~/.local/bin/canopy` symlink, the desktop entry (it must carry this install's
`X-Canopy-Install-Dir`, which sits in the `[Desktop Entry]` group, the only group whose `Exec=` is
rewritten) and the icons (they must be in this install's receipt) are skipped with a warning when
another file or a symlink is there, and nothing is ever copied through a symlink; on Windows the
same goes for a Start Menu shortcut or Apps & features entry that points at another folder.

`--uninstall` (Windows: `-Uninstall`, or Apps & features) takes the same lock and removes exactly
what the installer created, checking each item is still its own (a symlink pointing into the
install folder, a desktop entry carrying `X-Canopy-Install-Dir`, icons identical to the installed
ones, the shortcut targeting the installed exe). In the folder it removes only the installer's
own entries (Linux: `bin`, `share`, `legal`, `LICENSE`, `README.md`, `VERSION`, `uninstall.sh`,
`.canopy-install`; Windows: `canopy.exe`, `README.md`, `LICENSE`, `legal`, `VERSION`,
`uninstall.ps1`, `.canopy-install`; both: leftover `.staging.*`, `.previous.*` and `.lock`), then
the folder if that left it empty; otherwise it keeps the folder and lists what is in it. `--purge`
also deletes Canopy's settings, logs and caches after asking (or with `--yes`), following the
rules below. Tokens in the system keychain are left to Canopy's Settings → Accounts.

### Layout

| | Install folder (`CANOPY_INSTALL_DIR` overrides) | Outside it |
|---|---|---|
| Linux | `~/.local/opt/canopy/`: `bin/canopy`, `share/applications/io.necturalabs.Canopy.desktop`, `share/icons/hicolor/{256x256,scalable}/apps/`, `share/metainfo/`, `LICENSE`, `README.md`, `legal/` (privacy policy, terms, `third-party-notices.txt`), `VERSION`, `uninstall.sh`, `.canopy-install` (receipt: `root=`, `version=` and what was created outside) | `~/.local/bin/canopy` symlink (`--no-path` skips it; a warning when `~/.local/bin` is not on `PATH`); copies of the desktop entry and icons in `${XDG_DATA_HOME:-~/.local/share}/applications` and `icons/hicolor`. Only caches that already exist are refreshed (`mimeinfo.cache` by `update-desktop-database`, the GTK icon cache): the entry declares no MIME types, and running either tool where there is no cache would create one |
| macOS | `~/Applications/Canopy.app` (the variable names the `.app`) | `~/.local/bin/canopy` symlink to `Contents/MacOS/canopy` (`--no-path` skips it) |
| Windows | `%LOCALAPPDATA%\Programs\Canopy\`: `canopy.exe`, `README.md`, `LICENSE`, `legal\`, `VERSION`, `uninstall.ps1`, `.canopy-install`. A relative `CANOPY_INSTALL_DIR` is relative to PowerShell's current location | Start Menu shortcut `Canopy.lnk`; `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\io.necturalabs.Canopy`; the folder appended to the user `PATH` (`-NoPath` skips it; removed on uninstall only if the installer added it) |

`uninstall.sh` and `uninstall.ps1` are copies of the installer; run under that name they
uninstall the folder they sit in.

### Canopy's data folders and `--purge`

Canopy's own data folders are separate from the install folder and untouched unless `--purge`.
Their list is in one marked block near the top of each script (`purge_targets`,
`Get-PurgeTargets`). `--purge` lists what it will delete and what it leaves, asks, and never
follows a symlink (or junction). It deletes:

- a folder with a **vendor-specific name** (`io.necturalabs.Canopy`, `NecturaLabs\Canopy`) when it
  exists;
- a folder with a **generic name** (anything called just `canopy` or `Canopy`) only when it holds
  the marker file `.canopy-data`; for today's eframe folder (below), also when it holds Canopy's
  own files: `tokens.json`, an `app.ron` with Canopy's settings key `canopy-settings-v1`,
  `logs/canopy-*.jsonl` or `crashes/crash-*.txt`. Any other generically named folder is listed and
  left, since another program may own it.

Today the app writes only `eframe::storage_dir("canopy")` (eframe 0.36): Linux
`$XDG_DATA_HOME/canopy` (`~/.local/share/canopy` when unset or relative), macOS
`~/Library/Application Support/canopy`, Windows `%APPDATA%\canopy\data` (`%APPDATA%\canopy` goes
too once that left it empty).

**Contract for the planned move of the app's data (#44).** The installers already purge these
folders; the app must create the marker file `.canopy-data` (empty is fine) in each one it
creates, including every one with a generic name, or `--purge` will leave it:

| | Folders |
|---|---|
| Linux | `$XDG_CONFIG_HOME/canopy`, `$XDG_DATA_HOME/canopy`, `$XDG_STATE_HOME/canopy`, `$XDG_CACHE_HOME/canopy` (defaults `~/.config`, `~/.local/share`, `~/.local/state`, `~/.cache`; a relative value is ignored, as the XDG spec says) |
| macOS | `~/Library/Application Support/io.necturalabs.Canopy`, `~/Library/Logs/Canopy`, `~/Library/Caches/io.necturalabs.Canopy` |
| Windows | `%APPDATA%\NecturaLabs\Canopy`, `%LOCALAPPDATA%\NecturaLabs\Canopy` (an emptied `NecturaLabs` parent goes too) |

A new folder, or a new name, needs the same change in both scripts and in this table.

### Testing

`packaging/test-install.sh` builds fake releases with a dummy `canopy`, serves them with
`python3 -m http.server` on 127.0.0.1 and runs the installer in a temporary `HOME` through the
test-only `CANOPY_INSTALL_BASE_URL`. The scripts accept that only as `http://127.0.0.1:<port>`
or `http://localhost:<port>` with at most a trailing `/` (so not `http://127.0.0.1:1@elsewhere`),
and then allow plain HTTP to exactly that base. On Linux it runs the Linux flow and then the
macOS flow under a fake `uname`; on macOS it runs the macOS flow natively (CI does both; `tools/check.sh`
runs it on either). It covers install, reinstall, upgrade, checksum and tampering refusal, a
manifest URL outside Canopy's releases, `--version` (and a version spanning two lines), `--beta`
and the refusal to downgrade from it, a missing platform, `--no-path`, a foreign
`~/.local/bin/canopy`, a foreign or symlinked desktop entry and icons, malformed test feeds, no
`curl` or `wget`, a move that fails while the old version is moved aside or the new one moved in,
Ctrl+C during the swap (a stub `mv` on `PATH`), a symlinked `HOME`, uninstall (via `uninstall.sh`,
with a file of the user's in the folder, and `--uninstall --purge` with marked, unmarked and
symlinked data folders), a piped `sh -s` install, a foreign non-empty folder, a failed install
into an existing empty folder and into a new one, a path with a space, GNU wget, and copies of the
script cut short at several points. `CANOPY_TEST_SHELL` picks the shell (default dash, else
`sh`), e.g. `bash` or `"busybox sh"`; a busybox built to prefer its own applets (Ubuntu's
`busybox-static`) bypasses the test's stub `mv` and fake `uname`, so use one that runs commands
from `PATH`.

On Windows, CI parses `install.ps1` with Windows PowerShell 5.1 and PowerShell 7, checks its
argument handling and the same malformed test feeds (with `CANOPY_INSTALL_DIR` under
`RUNNER_TEMP`, and a check that the Start Menu, Apps & features and user `PATH` are unchanged),
checks that a run through `iex` leaves no functions, variables, StrictMode or preferences in the
session, and runs PSScriptAnalyzer where installed.

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
