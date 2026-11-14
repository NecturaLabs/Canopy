# ADR 0006: Canopy updates itself where its installer put it

**Status:** accepted
**Date:** 2026-10-02

## Context and problem statement

The update check ([ADR 0003](0003-network-services-and-consent.md)) only told people a new build
existed; "Download" opened the archive in a browser, and replacing Canopy by hand was left to
them. The one-line installers can upgrade in place, but running them is a trip to a terminal.
Canopy has to be able to install an update itself without becoming a way to run unverified code,
without touching anything of the user's, and without ever leaving a half-installed program.

## Decision drivers

- Integrity: only Canopy's own archive from Canopy's own release, checked against a SHA-256 that
  came from the manifest, before a byte of it is trusted.
- An install that fails or is interrupted leaves the previous version running and intact.
- Settings, tokens, logs, caches and repositories are never touched.
- What the installer registered (command link, desktop entry, icons, receipt, `uninstall.sh`)
  stays consistent, so the uninstaller still removes exactly what exists.
- No new service, and nothing contacted until the user chooses to update.
- Only what can be tested: this work is done and verified on Linux.

## Considered options

1. **Keep "Download" and leave the replacement to the user.** Nothing to get wrong, and nothing
   gained.
2. **Always run the one-line installer from the app.** One code path, but it executes a script
   fetched at update time on every update, even where the app could do the same file operations
   itself, and it installs a second copy when Canopy was not installed by it.
3. **Do it natively where the installer put Canopy, and run the installer elsewhere.** The
   native path is the installer's own protocol written in Rust, with tests; every other install
   gets the installer.

## Decision outcome

Option 3. Where the running executable is `<root>/bin/canopy` of a folder whose `.canopy-install`
receipt names that root (Linux), the update is done in process (`selfupdate/mod.rs`): download over
HTTPS from `releases/download/canopy-build-<n>/` of NecturaLabs/app-updates (the URL the manifest
names must equal that path exactly, redirects only to HTTPS, five at most, size capped), compare
the SHA-256 with the manifest's, unpack into `<root>/.staging.*` accepting only regular files and
folders under the archive's one top folder, swap the install folder's entries in two phases (the
old ones aside into `.previous.*`, then the new ones in; a failed move puts the old ones back),
and bring the receipt, `VERSION`, `uninstall.sh` and the registered desktop entry and icon copies
up to date. `uninstall.sh` is the release's `install.sh` only when it is listed in the release's
`SHA256SUMS`, matches, is headed as Canopy's installer and parses; otherwise the old one stays. The
install folder's `.lock` is taken as the installer takes it, with its holder's process id in a `pid`
file inside; on Linux the lock of a holder that no longer exists is taken over, elsewhere (and for a
lock without a process id) it is left and reported. Leftovers of an interrupted run are cleaned
(a swap killed halfway is undone), except a `.previous.*` folder when no Canopy binary remains
beside it.

Every other installation (a source build, macOS, Windows) downloads the feed's installer script,
checks it is headed as Canopy's installer, and runs it pinned to the new build (`--version N`).
That installs the installer's own copy and leaves the running one alone; the dialog says so before
the user presses Update.

Both end with an offer to restart. The restart starts the new program only after the window has
closed and the settings are saved, so the new run reads what the old one wrote.

Consequences:

- The privacy policy gained an "Installing an update" section and its date moved on, so every
  user is asked to accept it again.
- The Windows install folder and the macOS app bundle are not updated natively. Windows would
  need the running executable renamed aside, and neither platform could be tested here; both run
  the installer.
- Lock takeover judges a holder by its process id in `/proc`: a different process namespace or a
  `hidepid` mount can make a running holder look gone, and a reused id makes a dead one look
  alive (the lock then stays until the user removes it). Both are accepted limits.
- A roll-back that cannot put every old file back marks its `.previous.*` folder (`.not-restored`);
  the start sweep and the next update refuse to clear it, since it is the old version's only copy.
- `tar`, `flate2` and `sha2` are direct dependencies. The archive is read with `tar` but every file
  is written by Canopy itself, never over another file, and links and special files are refused.
- The download base can be changed only at compile time (`CANOPY_UPDATE_DOWNLOADS`), like the
  feed, which is how the updater is tested against a local server.

## Related

- [ADR 0003](0003-network-services-and-consent.md): the closed list of services; the download is
  part of the update service and starts only when the user asks.
- `packaging/README.md`: what the installers do, which the native path repeats.
