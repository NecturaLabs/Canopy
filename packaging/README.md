# Packaging

Canopy ships as one `canopy` binary (a `Canopy.app` bundle on macOS). Everything a desktop needs
besides the binary lives here and in `assets/`.

| File | Purpose |
|---|---|
| `install.sh` | One-line installer for Linux and macOS (POSIX `sh`). Published as `canopy/install.sh` in NecturaLabs/app-updates. |
| `install.ps1` | One-line installer for Windows (Windows PowerShell 5.1 and PowerShell 7). Published as `canopy/install.ps1`. |
| `test-install.sh` | End-to-end test of `install.sh` against a fake feed on 127.0.0.1 in a temporary `HOME`, on Linux and macOS (run by `tools/check.sh` and CI). |
| `linux/io.necturalabs.Canopy.desktop` | freedesktop.org desktop entry. The app ID, the icon name and `StartupWMClass` are all `io.necturalabs.Canopy`, which must match the Wayland app ID / X11 `WM_CLASS` the binary sets. The installer rewrites `Exec=` to the absolute path of the installed binary. |
| `linux/io.necturalabs.Canopy.metainfo.xml` | AppStream metadata for software centres. The release workflow adds this build's `<release>` entry to the copy in the archive; the file in the repository needs none. |
| `macos/Info.plist` | `Info.plist` of `Canopy.app` (bundle id `io.necturalabs.Canopy`); the release fills in `@BUILD@` (the build number) as both bundle versions. |
| `../assets/icon-{64,256,512}.png` | Application icon. Linux archives carry the 256 px PNG; the macOS `.icns` is generated from the 512 px PNG during the release. |

## Crate versions and release builds

The workspace crates use the fixed Cargo version `0.0.0` and are not published to crates.io.
It is **not** the version shown to users and must not be incremented for releases. Canopy
releases are identified by a `build-N` tag, where `N` is the commit count on `main`.
`tools/release.py` determines the number; the release workflow embeds it in the executable,
archives, update manifests and installer metadata. Cargo's version appears only in private
package metadata and the corresponding entries in `Cargo.lock`. The earlier
`0.2.1-beta.1` value was a legacy pre-numbered release, not an active version stream.

## The one-line installers

```sh
curl -fsSL https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.sh | sh
```

```powershell
irm https://raw.githubusercontent.com/NecturaLabs/app-updates/main/canopy/install.ps1 | iex
```

A stable release commits the installers to app-updates. Before the first one they were copied
there by hand from `v0.2.1-beta.1`, so the addresses answer, but the stable channel
(`latest.json`) has no builds yet: the plain command stops with "no build", and `--beta`
(`sh -s -- --beta`) installs the newest beta build, which ships Linux builds only for now.

These URLs are stable: everything Canopy publishes in the shared, public NecturaLabs/app-updates
repository sits under its `canopy/` folder (scripts, `latest.json`, `beta.json`) or in releases
tagged `canopy-build-<number>`. Nothing, in the scripts, the docs or the app, uses the repository-wide
`releases/latest`, which may belong to another app.

### What the installers do

1. Detect the platform: OS and architecture (an x86_64 shell under Rosetta still gets the Apple
   silicon build), and on Linux glibc 2.35 or newer (musl is refused). `install.sh` checks for
   `curl` or GNU `wget` before downloading anything, and says so when neither is there.
2. Read `canopy/latest.json` (`--beta` also reads `beta.json` and takes the newer build). Builds
   are numbered: the manifest's `version` is the build number, `git rev-list --count` of the
   build's commit on `main`, and builds are ordered by that integer alone (a version such as
   `0.2.1-beta.1`, from before builds were numbered, is older than every build, and a beta
   manifest naming one counts as absent). With `--version N` they use the manifest when it names
   N, else the `SHA256SUMS` of release `canopy-build-N`. Without `--version` they never install a
   build older than the installed one: they stop and suggest `--beta` or `--version N` (so a beta
   build stays until asked otherwise).
3. Accept only `https://github.com/NecturaLabs/app-updates/releases/download/canopy-build-<number>/canopy-build-<number>-<target>.<ext>`
   and a 64-digit SHA-256, verify the checksum and stop on any mismatch before touching the
   installation. Downloads are HTTPS only, TLS 1.2 or newer:
   - `curl -q --proto =https --proto-redir =https --tlsv1.2`: no `~/.curlrc`, and a redirect to
     anything but HTTPS is refused.
   - GNU `wget --https-only --secure-protocol=TLSv1_2`, with `--no-config` where the wget has it
     (1.17 and later). wget has no switch for the scheme of a redirect and follows the server's
     redirects as they come; `--https-only` only governs recursive downloads. The first request
     always goes to GitHub over verified TLS, and GitHub's redirects go to HTTPS; the archive is
     checked against the SHA-256 from the manifest either way.
   - The checksum proves the archive is the one the feed names, not who published it: with
     `--version` the checksum file comes from the same GitHub release as the archive, and nothing
     is signed. Trust rests on GitHub's TLS and on who can write to `NecturaLabs/app-updates`.
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

Earlier builds also registered a scalable SVG. Upgrades remove it only when it is a regular file,
not a symlink, whose current SHA-256 is one of the two icons those builds shipped, whether the old
receipt lists it or the copy is orphaned. An edited or custom SVG is left alone; the receipt does
not by itself prove that its current bytes are still Canopy's.

`--uninstall` (Windows: `-Uninstall`, or Apps & features) takes the same lock and removes exactly
what the installer created, checking each item is still its own (a symlink pointing into the
install folder, a desktop entry carrying `X-Canopy-Install-Dir`, icons identical to the installed
ones, the shortcut targeting the installed exe). In the folder it removes only the installer's
own entries (Linux: `bin`, `share`, `legal`, `LICENSE`, `README.md`, `VERSION`, `uninstall.sh`,
`.canopy-install`; Windows: `canopy.exe`, `README.md`, `LICENSE`, `legal`, `VERSION`,
`uninstall.ps1`, `.canopy-install`; both: leftover `.staging.*`, `.previous.*` and `.lock`), then
the folder if that left it empty; otherwise it keeps the folder and lists what is in it. `--purge`
also deletes Canopy's settings, logs and caches after asking (or with `--yes`), following the
rules below. Saved tokens are `tokens.json` in Canopy's data folder, so `--purge` deletes them with
it (Canopy does not use the system keychain yet).

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

- a folder with a **vendor-specific name** (`io.necturalabs.Canopy`, `NecturaLabs\Canopy`, and the
  leftover app-id folder of earlier builds: `${XDG_DATA_HOME:-~/.local/share}/io.necturalabs.canopy`
  on Linux, `%APPDATA%\io.necturalabs.Canopy` on Windows) when it exists;
- a folder with a **generic name** (anything called just `canopy` or `Canopy`) only when it holds
  the marker file `.canopy-data`; for the folder eframe used (`canopy`, which is also Linux's data
  folder), also when it holds Canopy's own files and nothing else: `tokens.json`, an `app.ron` with
  Canopy's settings key `canopy-settings-v1`, `logs/canopy-*.jsonl` or `crashes/crash-*.txt`, and no
  entry named other than `tokens.json`, `settings.json*`, `app.ron*`, `logs`, `crashes` and
  `avatars`. A folder with a file of someone else's beside Canopy's is listed and left. Any other generically named folder is listed and left too, since
  another program may own it.

The app creates these folders itself and puts `.canopy-data` (empty, 0600) in each one that is
empty or holds only its own files (`crates/canopy-app/src/storage.rs`, ADR 0018); a folder with a
file of someone else's is used but left unmarked, and `--purge` leaves it. Builds before that wrote only `eframe::storage_dir("canopy")` (Linux `$XDG_DATA_HOME/canopy`, macOS
`~/Library/Application Support/canopy`, Windows `%APPDATA%\canopy\data`) and the folder named
after the app id (`app.ron`); the app moves their contents on its first start, and the installers
still recognise the old `canopy` folder by its contents, for a user who never started the new build.

**Contract for the app's folders (#40).** The installers purge these folders; the app creates the
marker file `.canopy-data` in each one it creates, including every one with a generic name, or
`--purge` will leave it:

| | Folders |
|---|---|
| Linux | `$XDG_CONFIG_HOME/canopy`, `$XDG_DATA_HOME/canopy`, `$XDG_STATE_HOME/canopy`, `$XDG_CACHE_HOME/canopy` (defaults `~/.config`, `~/.local/share`, `~/.local/state`, `~/.cache`; a relative value is ignored, as the XDG spec says) |
| macOS | `~/Library/Application Support/io.necturalabs.Canopy`, `~/Library/Logs/Canopy`, `~/Library/Caches/io.necturalabs.Canopy` |
| Windows | `%APPDATA%\NecturaLabs\Canopy`, `%LOCALAPPDATA%\NecturaLabs\Canopy` (an emptied `NecturaLabs` parent goes too) |

The leftover app-id folders of earlier builds (Linux `$XDG_DATA_HOME/io.necturalabs.canopy`,
Windows `%APPDATA%\io.necturalabs.Canopy`, which the app empties but may not remove when something
else is in them) are purged by their vendor-specific name; on macOS the app-id folder is the config
folder above.

A new folder, or a new name, needs the same change in both scripts and in this table.

### Testing

`packaging/test-install.sh` builds fake releases with a dummy `canopy`, serves them with
`python3 -m http.server` on 127.0.0.1 and runs the installer in a temporary `HOME` through the
test-only `CANOPY_INSTALL_BASE_URL`. The scripts accept that only as `http://127.0.0.1:<port>`
or `http://localhost:<port>` with at most a trailing `/` (so not `http://127.0.0.1:1@elsewhere`),
and then allow plain HTTP to exactly that base. On Linux it runs the Linux flow and then the
macOS flow under a fake `uname`; on macOS it runs the macOS flow natively (`tools/check.sh`
runs it on either; CI does not). It covers install, reinstall, upgrade, checksum and tampering refusal, a
manifest URL outside Canopy's releases, `--version` (and a number spanning two lines, a semantic version, a leading zero), builds ordered as integers (99 below 100), an installation from before builds were numbered, `--beta`
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

Nothing in CI runs `install.ps1` today: its Windows job (parse with Windows PowerShell 5.1 and
PowerShell 7, argument handling, the malformed test feeds with `CANOPY_INSTALL_DIR` under
`RUNNER_TEMP`, an unchanged Start Menu, Apps & features and user `PATH`, a run through `iex` that
leaves nothing in the session, PSScriptAnalyzer) was dropped from `ci.yml`. Run those checks by
hand on Windows before publishing a change to it; the installer fixes made to `install.sh` are not
yet in `install.ps1` ([#39](https://github.com/NecturaLabs/Canopy/issues/39)).

## Release archives

`.github/workflows/release.yml` builds `canopy-build-<number>-<target>.tar.gz` (Linux, macOS) or
`.zip` (Windows), each holding one folder `canopy-build-<number>-<target>/` with `README.md`,
`LICENSE`, `legal/` and:

- Linux (`x86_64-unknown-linux-gnu`; `aarch64-unknown-linux-gnu` cross-compiled, experimental):
  `canopy`, the desktop entry, the metainfo, and `io.necturalabs.Canopy.png`.
- macOS (`aarch64-apple-darwin`; `x86_64-apple-darwin` cross-compiled, experimental):
  `Canopy.app`, ad-hoc signed, not notarised. The installer downloads with curl, so Gatekeeper
  does not quarantine it; an archive downloaded in a browser needs right-click → Open the first
  time.
- Windows (`x86_64-pc-windows-msvc`): `canopy.exe`.

An experimental build may fail without holding back the release; the installer then reports
that the platform has no build. The release also attaches `install.sh`, `install.ps1` and
`SHA256SUMS`, and a stable release commits the two scripts to `canopy/` in app-updates next to
`latest.json`. Beta builds never replace them.

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

Tracked in GitHub Issues ([#95](https://github.com/NecturaLabs/Canopy/issues/95), [#97](https://github.com/NecturaLabs/Canopy/issues/97)): Flatpak (Flathub manifest with `--filesystem=home` and host `git`
via `flatpak-spawn` or a bundled git), AUR (`canopy-bin` and a source package), `.deb` with
`cargo-deb`, `.rpm` with `cargo-generate-rpm`, AppImage, a signed and notarised macOS app, and a
signed Windows binary.
