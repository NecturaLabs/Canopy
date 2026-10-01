# Keybindings

Canopy is keyboard-first, in the spirit of Omarchy: a launcher-style command palette, a cheatsheet
one key away, vim motions and single-key, lazygit-style actions for Git. Every binding lives in
one table, `BINDINGS` in `crates/canopy-app/src/keymap.rs`, which both dispatches the keys and
draws the cheatsheet (`?`, or Ctrl+/) and Settings → Keyboard, so those two always match the
code. This page is a reference copy of that table: change the table first, then this page (the
`add-keybinding` skill is the recipe).

Rules:

- **Super is never bound.** It belongs to the window manager.
- **Bare keys never fire while a text field has focus.** Only the "Anywhere" chords, which all
  carry Ctrl, or are F5 or Esc, work while typing.
- **One owner per chord.** A test fails if two bindings in overlapping scopes share one.
- Ctrl is Cmd on macOS (the Control key is accepted there too). A chord written as a character
  (`?`, `/`, `[`, `]`, `=`) matches whatever keys the layout needs to type it.

Chords marked "also" are alternatives for the same command.

## Anywhere

Works also while typing.

| Keys | Action |
|---|---|
| Ctrl+K, also Ctrl+P, Ctrl+Space | Open the command palette |
| Ctrl+/ | Show keyboard shortcuts |
| Ctrl+O, also Ctrl+T | Open a repository in a new tab |
| Ctrl+Shift+N | Clone a repository |
| Ctrl+W | Close the tab (not while typing) |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Ctrl+1 … Ctrl+9 | Go to tab 1–9 |
| F5, also Ctrl+R | Refresh |
| Ctrl+Shift+F | Fetch |
| Ctrl+Shift+L | Pull |
| Ctrl+Shift+P | Push |
| Ctrl+Shift+H | Stash changes |
| Ctrl+Shift+S | Stage all changes |
| Ctrl+B | Create a branch |
| Ctrl+Enter | Commit staged changes |
| Ctrl+F | Filter history |
| Ctrl+, | Open settings |
| Ctrl+= (also Ctrl++) / Ctrl+− | Zoom in / out |
| Ctrl+0 | Reset zoom |
| Ctrl+Q | Quit Canopy |
| Esc | Close the dialog, palette or cheatsheet |

## Lists and views

Only when no text field has focus.

| Keys | Action |
|---|---|
| ? | Show keyboard shortcuts |
| 1 / 2 / 3 | Go to History / Changes / pull or merge requests |
| J or ↓ | Next commit, file or request (repeats while held) |
| K or ↑ | Previous commit, file or request (repeats while held) |
| PgDn or Ctrl+D | Down a page |
| PgUp or Ctrl+U | Up a page |
| gg (press G twice), also Home | Go to the top |
| Shift+G, also End | Go to the bottom |
| / | Filter history |
| Shift+R | Refresh |

## History

| Keys | Action |
|---|---|
| Shift+T | Tag the selected commit |
| Y | Copy the selected commit's SHA |
| O | Open the commit (or request) on GitHub or GitLab |

## Changes

| Keys | Action |
|---|---|
| C | Write the commit message |
| Space | Stage or unstage the selected file |
| A | Stage all changes |
| U | Unstage all changes |
| D | Discard the selected file (asks first) |

## Resolving a conflicted file

| Keys | Action |
|---|---|
| ] / [ | Next / previous conflict |
| H | Take ours (left) |
| L | Take theirs (right) |
| B | Take both, ours first |
| Shift+B | Take both, theirs first |
| Shift+A | Take the common ancestor |
| E | Edit the conflict by hand |
| Z | Undo the choice for this conflict |

## Repository actions

| Keys | Action |
|---|---|
| F | Fetch |
| P | Pull |
| Shift+P | Push |
| N | Create a branch |
| M | Merge or rebase |
| S | Stash changes |
| Shift+S | Pop the latest stash |

The palette (Ctrl+K) also lists commands that have no key, such as initialising a repository,
adding a remote or switching to the next theme, with the shortcut shown next to the ones that do.
