# ADR 0016: Launcher lists show what is installed, then what is well known and missing, then Custom command

**Status:** accepted
**Date:** 2026-10-14

## Context

Settings → Integrations lets the user choose the terminal, the editor and the file manager that
Canopy's "Open terminal here", "Open in editor" and "Show in file manager" start. Until now the
lists held the system's own program, the programs found on this computer and Custom command…,
so a user could not tell whether Canopy knew a program they had not installed yet, and a choice
saved for a program that was later uninstalled stayed a plain custom command that failed with an
error when used.

The owner decided (2026-10-13): each list is built from facts about this computer, in this
order, and a missing program cannot be picked.

1. The installed programs, each once ([#243](https://github.com/NecturaLabs/Canopy/issues/243)).
2. Well-known programs that are not installed, after the installed ones, dimmed and tagged "Not
   installed".
3. Custom command…, last, for any other tool.

## Decision

- **One table.** `KNOWN` in `crates/canopy-app/src/system_apps.rs` holds, per program, its kind,
  its name and what each platform calls it: the command names on `PATH` and the desktop entry
  ids (Linux), the application bundle (macOS) and the App Paths key (Windows). It replaces the
  per-platform allowlists `MAC_APPS` and `WINDOWS_APPS` and the terminal list `LINUX_TERMINALS`
  (now `linux_terminals()`, in the table's order after `x-terminal-emulator`). `GUI_EDITORS`
  stays: it tells a graphical editor from a terminal editor in `$VISUAL`/`$EDITOR`, which is not a
  list of rows. An install page is not recorded: nothing would read it, and offering to open it
  is not required (#45 covers missing required tools).
- **Where "Not installed" rows are shown** (per platform and kind). A row says a program is not
  installed only where the platform can say so reliably:

  | Platform | Editors | Terminals | File managers | How a program counts as installed |
  |---|---|---|---|---|
  | Linux | rows | rows | rows | an installed desktop entry with one of its ids (so Flatpak and Snap exports in `XDG_DATA_DIRS` count), or one of its commands on `PATH` |
  | macOS | rows | rows | rows | its `.app` folder in `/Applications`, `/Applications/Utilities`, `/System/Applications`, `/System/Applications/Utilities`, `/System/Library/CoreServices` or `~/Applications` |
  | Windows | none | none | none | App Paths only; it misses programs that are only on `PATH` (`wt.exe` is an alias) and Explorer is always there, so it cannot say a program is missing |

  A kind or platform without rows lists the installed programs and Custom command…. Windows
  gets rows once the Start menu or `PATH` is read for the curated names.
- **A missing program cannot be picked.** The select kit has an unavailable state
  (`Select::unavailable`): the label is dimmed, a tag says why, the arrow keys, Home, End and
  typing pass over it, a click or Enter on it does nothing, and a screen reader hears it as
  disabled with the description "Not installed".
- **A saved choice that is gone shows as missing and runs nothing.** A saved command that starts
  a well-known program that is not installed shows as that program, selected, dimmed and tagged.
  Opening runs the system's program instead and a notification says which program is missing and
  what ran. Any other saved command (a program nobody knows, or an installed one) runs as typed;
  a failure keeps today's error message with the operating system's reason.
- **Installed means found, not just categorised.** A well-known program on `PATH` that no desktop
  entry lists is an installed program too (its command is the program's name), so it is never
  shown as "Not installed". One program found through its entry and `PATH` is one row (#243: a
  program's id is its executable, and a launcher entry such as `flatpak run` is told apart by the
  entry).
- **Editors that need a terminal** (vim, Neovim, Helix, nano) are not in the table. A desktop
  entry for one is listed only where a terminal launcher can start it ("Neovim in terminal"),
  as before.

## The curated lists and their sources

Chosen for what users of a Git client open files, folders and repositories with; each list is
consulted on 2026-10-14 and can be extended with a row in `KNOWN`.

- **Editors (graphical).** Stack Overflow Developer Survey 2025, "Development environments"
  (<https://survey.stackoverflow.co/2025/technology>), share of respondents: Visual Studio Code
  75.9 %, Notepad++ 27.4 %, Cursor 17.9 %, Sublime Text 10.5 %, Zed 7.3 %, VSCodium 6.2 %,
  Windsurf 4.9 %. IDEs that need their own launcher setup (the JetBrains family, Visual Studio,
  Xcode) are left out; Vim, Neovim and Nano (24.3 %, 14.0 %, 12.2 %) are terminal editors. Added
  for the desktops that ship them as their default text editor: Kate (KDE), Text Editor and
  gedit (GNOME), Mousepad (Xfce), Geany; for macOS the long-standing editors BBEdit,
  CotEditor, Nova, TextMate, MacVim and TextEdit (the previous allowlist).
- **Terminals.** The desktop environments' own terminals: Console, GNOME Terminal and Ptyxis
  (GNOME), Konsole (KDE), Xfce Terminal (Xfce), Terminal (macOS), Windows Terminal; popularity
  data: Debian popularity contest by source package
  (<https://popcon.debian.org/source/by_inst>, read 2026-10-14 up to rank 975: gnome-terminal
  rank 393, xterm rank 921; Konsole and the others below were not among those ranks). The
  others are the widely used GPU and tiling terminals of their platforms, chosen by judgment:
  Alacritty, kitty, Foot, WezTerm, Ghostty, Tilix, Terminator, iTerm, Warp.
- **File managers.** The desktop environments' own: Files (Nautilus, GNOME), Dolphin (KDE),
  Thunar (Xfce), Nemo (Cinnamon), Caja (MATE), PCManFM (LXDE), COSMIC Files, Finder (macOS),
  Explorer (Windows); popularity: Debian popcon (nautilus rank 398, dolphin rank 920); macOS
  Path Finder, ForkLift and Commander One, Windows Total Commander and Files (the previous
  allowlists).

## Consequences

- Linux and macOS users see what Canopy can use and what it cannot find; Windows users see no
  change in what the lists show.
- A program missing from `KNOWN` can still be chosen through its desktop entry or Custom
  command…; the table is a convenience, not a gate.
- Opening with a stale choice no longer fails with an operating-system error for the well-known
  programs: it opens with the default and says so.
- The detection runs on its own thread, as before, and reads the same places (desktop entries,
  `PATH`, the macOS application folders, the Windows registry): no new access and no new host.
