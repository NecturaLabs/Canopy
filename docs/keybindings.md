# Keybindings

Canopy is keyboard-first, in the spirit of Omarchy: a launcher-style command palette, a cheatsheet
one key away, vim motions and single-key, lazygit-style actions for Git. Every binding lives in
one table, `BINDINGS` in `crates/canopy-app/src/keymap.rs`, which both dispatches the keys and
draws the cheatsheet (`?`, or Ctrl+/) and Settings → Keyboard, so those two always match the
code. This page is a reference copy of that table: change the table first, then this page (the
`add-keybinding` skill is the recipe).

## Why the keys are what they are

Omarchy's own bindings (`/usr/share/omarchy/default/hypr/bindings/`, its tmux and terminal
configs) follow a few rules, and Canopy's follow the same ones:

- **Each layer owns its modifiers and a lower layer never uses a higher one's.** Super is the
  window manager's: every window, workspace, launcher and menu action. Alt and Ctrl+Alt belong to
  cycling windows and monitors (Alt+Tab, Ctrl+Alt+Tab, Ctrl+Alt+Delete) and to tmux's no-prefix
  chords; tmux's prefix is Ctrl+Space (and Ctrl+B), which only reaches a terminal, never a
  Canopy window. Print, F9 (push-to-talk dictation), the media keys and the
  Caps Lock compose key belong to the system. Whatever runs inside gets Ctrl and bare keys.
- **The key names the thing, the modifier names the variant.** Shift reverses or moves
  (Tab/Shift+Tab, focus/swap, move to workspace); Alt is the fine or quiet form; Ctrl is the
  broader one. The same letter keeps its family across the variants.
- **Mnemonic letters, positional digits, Tab to cycle, arrows to move, Esc to dismiss, comma for
  settings, Space for the launcher, help one key away.** Every binding has a description.
- **One owner per chord**, and nothing is rebound silently.

Canopy is the app inside, so it uses the Ctrl layer, mirroring Omarchy's Super chords with Ctrl:

| Omarchy (Super) | Canopy (Ctrl) |
|---|---|
| Space, the launcher | Ctrl+Space, the command palette (also Ctrl+K) |
| W, close window | Ctrl+W, close tab |
| Tab / Shift+Tab, next / previous workspace | Ctrl+Tab / Ctrl+Shift+Tab, next / previous tab |
| 1 … 0, workspace | Ctrl+1 … 9, tab |
| − / =, resize | Ctrl+− / =, zoom |
| K, keybindings | ? or Ctrl+/, the cheatsheet |
| comma, notifications | Ctrl+Comma settings |

Rules, each guarded by a test in `keymap.rs`:

- **Super is never bound.** It belongs to the window manager, and nothing fires while it is held.
- **Alt is never bound.** A press carrying Alt matches no chord, except a character typed with
  AltGr (`[`, `?`). That keeps Alt+Tab, Ctrl+Alt+Tab, Ctrl+Alt+Delete and tmux's chords
  Omarchy's, and the keys Omarchy owns (F9, Print, media keys) are not used either.
- **Shift turns a family around:** `p` pull and `P` push, `s` stash and `S` pop, `b` and `B` take
  both, `g` and `G`, Ctrl+Tab and Ctrl+Shift+Tab.
- Capital letters shown as single-key shortcuts mean Shift plus that letter; `P`, for example,
  means Shift+P.
- **Bare keys never fire while a text field has focus**, nor while a select's open list is taking
  letters (type-ahead). Only the "Anywhere" chords, which all
  carry Ctrl, or are F5 or Esc, work while typing. Their Git twins are Fork's Ctrl+Shift chords,
  chosen so they never take a text-editing chord (Ctrl+A, C, V, X, Z, Y). Pull and push are the
  one family whose letters differ between the layers (`p` and `P` against Ctrl+Shift+L and
  Ctrl+Shift+P), on purpose: the Ctrl twin of `p` would be Ctrl+P, the palette's alias, and
  Fork's pair keeps push, the one that leaves the machine, apart from pull.
- **One owner per chord.** A test fails if two bindings in overlapping scopes share one.
- Ctrl is Cmd on macOS (the Control key is accepted there too). A chord written as a character
  (`?`, `/`, `[`, `]`, `=`) matches whatever keys the layout needs to type it.

Chords marked "also" are alternatives for the same command.

## Changing a key

Settings → Keyboard lists every row of the table with its keys. **Change** listens for the next key
press and gives the row that one key, replacing the row's others (Esc cancels); **Reset** returns
the row to the table, and **Reset all** every row. The tab jumps (Ctrl+1…9) are positions and stay.
A change is saved in the settings (`keybindings`: the row, written `Command.Section`, to the chord's
text, `ctrl+shift+symbol+Key`), travels with Settings export and import, and applies on the next
frame. The rules above hold for what you choose: a key another row uses is refused, naming that row
(nothing is rebound silently), and so are Super, F9, Tab without Ctrl, a Ctrl key in a single-key
row and a key without Ctrl in an "Anywhere" row (it would fire while typing). Alt is never read. A
saved file that breaks a rule, or gives two rows one key, has those entries ignored. A row with a
double press (`gg`) fires on one press of its new key.

## Anywhere

Works also while typing.

| Keys | Action |
|---|---|
| Ctrl+K, also Ctrl+P, Ctrl+Space | Open the command palette |
| Ctrl+/ | Show keyboard shortcuts |
| Ctrl+O, also Ctrl+T | Open a repository in a new tab |
| Ctrl+Shift+N | Clone a repository |
| Ctrl+Shift+I | Initialize a new repository |
| Ctrl+W | Close the tab (not while typing) |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Ctrl+1 … Ctrl+9 | Go to tab 1–9 |
| F5, also Ctrl+R | Refresh |
| Ctrl+Shift+F | Fetch |
| Ctrl+Shift+L | Pull |
| Ctrl+Shift+P | Push |
| Ctrl+Shift+A | Approve the selected request (Requests view) |
| Ctrl+Shift+M | Merge the selected request (Requests view; asks first) |
| Ctrl+Shift+H | Stash changes |
| Ctrl+Shift+S | Stage all changes |
| Ctrl+B | Create a branch |
| Ctrl+Enter | Commit staged changes |
| Ctrl+F | Show or hide History search (closing clears the query); in Requests, focus the request filter |
| Ctrl+Comma | Open settings |
| Ctrl+= (also Ctrl++) / Ctrl+− | Zoom in / out |
| Ctrl+0 | Reset zoom |
| Ctrl+Q | Quit Canopy |
| Esc | Close the dialog, palette, cheatsheet or History search (in the request filter: clear it, then leave it) |

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
| / | Show or hide History search; in Requests, focus the request filter |
| Shift+R | Refresh |
| Shift+F10 | Open the menu of the focused row |

## History

| Keys | Action |
|---|---|
| Shift+T | Tag the selected commit |
| Shift+F | Search the history for changed text |
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
| H | Take Current (left) |
| L | Take Incoming (right) |
| B | Take both, Current first |
| Shift+B | Take both, Incoming first |
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
