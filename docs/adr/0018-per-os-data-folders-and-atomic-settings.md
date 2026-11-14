# ADR 0018: Canopy's files live in per-OS folders, and settings are written atomically

**Status:** accepted
**Date:** 2026-11-14

## Context

Canopy's files were in two folders that `eframe::storage_dir` names: `canopy` (tokens, logs, crash
reports, pictures) and one named after the window's app id (`app.ron`: the settings, the window's
place, the notifications). `app.ron` was written in place by eframe, so a write cut short left a
truncated file; an unreadable one was read as nothing, the settings fell back to defaults without
a word and the next save overwrote what was left (issue #40). The installers' `--purge` already
expects folders with a marker file `.canopy-data` (`packaging/README.md`), which no code created.

## Decision drivers

- A write interrupted at any point leaves the previous settings, complete.
- A damaged file is never silently replaced; the user is told once.
- The UI thread never waits for the disk.
- One layout per operating system that follows its conventions and that `--purge` can find.
- No new dependency and no new host.

## Decision outcome

**Folders** (`storage.rs`): config, data, state and cache, each created owner-only (0700 on Unix)
and claimed with the empty file `.canopy-data` (0600) when it is empty or holds only Canopy's own
files (its settings, tokens, window state, logs, crash reports, pictures and the temporary names of
a move); a folder with a stranger's file is used but left unmarked, and the installers' `--purge`
leaves it, the Linux data folder (the old eframe folder) included. Linux follows the XDG variables (`$XDG_CONFIG_HOME/canopy`, `$XDG_DATA_HOME/canopy`,
`$XDG_STATE_HOME/canopy`, `$XDG_CACHE_HOME/canopy`); macOS uses `Application Support/<app id>`
(config and data), `Logs/Canopy` and `Caches/<app id>`; Windows uses `%APPDATA%\NecturaLabs\Canopy`
and `%LOCALAPPDATA%\NecturaLabs\Canopy`. This is the table in `packaging/README.md`, which the
installers already purge. Settings and window state are config; saved tokens are data (until
#227 removes them); logs and crash reports are state; pictures are cache.

**Migration** (`storage::migrate`, first thing in `main`): the contents of the two old folders move
one file at a time to their new places. A file is never overwritten (the move is a hard link that
refuses an existing destination, then removal of the old name; across file systems or where links
are unavailable, a flushed copy under a per-process temporary name that is linked into place the
same way), never partly present at its new place and leaves the old place only after it arrived,
so a run cut short, or a second run, finishes the job. Links and files Canopy
does not know stay. Emptied old folders go.

**Settings file** (`settings_store.rs`): `settings.json` in the config folder, `{"schema": 1,
"settings": {…}}`. Every settings field has a serde default, so an older file loads with new fields
at their defaults; `schema` versions the envelope and refuses a newer one rather than guessing.
eframe's `app.ron` keeps the window's place and size and the notifications, and is still written
by eframe.

**Atomic writes, off the UI thread**: a frame that saves serialises the settings (a few kilobytes)
and hands the text to a `settings-writer` thread, which writes a temporary file beside the real one
(0600), flushes it, renames it over the real one and flushes the folder. Only the newest queued
text is written, and an identical one is skipped. A failed write is reported once as a toast; the
previous file stays. A test stops a write at each stage and reads the file back.

**Recovery**: an unreadable `settings.json` (not JSON, cut short, a value of the wrong kind, no or a
newer schema, over 16 MiB) is moved to `settings.json.bad` (replacing an older one), the settings
start from defaults and the notification center says so once, with the reason and where the file
is. Builds before this layout kept the settings in `app.ron` under the key `canopy-settings-v1`;
with no `settings.json` yet they are read from there once, and removed from eframe's storage after
the first write.

## Consequences

- Backups and sync tools see settings as a plain file in the config folder, apart from the data
  and the cache.
- `app.ron` is still written in place by eframe. What it holds now (window geometry, the
  notifications) is cheap to lose; a damaged one is read as nothing.
- Windows cannot flush a folder; the rename is as durable as the file system makes it there.
- A new folder or name needs the same change in `install.sh`, `install.ps1` and the table in
  `packaging/README.md`.
- A REST redirect to another host never carries the token (`RedirectAuthHeaders::SameHost`); tests now pin it.
