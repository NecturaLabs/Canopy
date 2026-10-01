---
name: add-keybinding
description: Recipe for adding, changing or removing a keyboard shortcut in Canopy through the single table in crates/canopy-app/src/keymap.rs. Covers choosing a chord by the Omarchy, lazygit, vim and Fork conventions, the Command, Binding and Section types, the rules (never Super, bare keys never fire while typing, one owner per chord), handling the command in app/input.rs, the tests that guard the table, the cheatsheet and docs/keybindings.md. Use whenever a request adds or rebinds a key, or when a command should be reachable from the keyboard.
license: Proprietary (NecturaLabs); see LICENSE
---

# Add a keybinding

Every shortcut lives in one table, `BINDINGS` in `crates/canopy-app/src/keymap.rs`. The same table
dispatches keys, renders the cheatsheet (`ui/cheatsheet.rs`, `?` or Ctrl+/), fills Settings →
Keyboard, and supplies the hints in menus, tooltips and the command palette (`keymap::hint`). A
shortcut written anywhere else (an ad hoc `key_pressed` in a view, a table in the README) is a
defect: it bypasses the rules below and drifts.

## The rules

These come from the owner (AGENTS.md) and are enforced by tests and by `keyboard()` in `app/input.rs`:

1. **Never bind Super.** It belongs to the window manager (Omarchy's modifier). `dispatch` also
   fires nothing while Super is held, so a window-manager chord cannot reach a bare key.
2. **Bare keys never fire while a text field has focus.** Only `Section::Anywhere` bindings work
   while typing, so they must carry Ctrl (Cmd on macOS; the `ctrl` field of `Chord`), or be F5 or
   Escape. Every other section is a bare-key section (`Section::is_bare`): no Ctrl, except vim's
   half-page motions in lists. A focused button, checkbox or list also keeps Space, Enter and the
   arrows (`keymap::widget_key`), and an open dialog, settings screen or palette disables
   everything except Dismiss.
3. **One owner per chord.** `no_chord_is_bound_twice_in_overlapping_scopes` rejects any chord that
   appears twice anywhere in the table, even in sections that are never active together. Reusing
   a bare key in another section therefore means choosing another key.
4. **No Alt chords.** Alt is not matched; `symbol` chords tolerate AltGr only so that characters
   such as `[` and `?` work on every layout.

## 1. Pick the chord

Order of precedence for the conventions (the comment above `BINDINGS`):

- **Omarchy**: one modifier for the app's global actions (Ctrl, never Super) and help one key
  away: `?` opens the cheatsheet from any bare context, Ctrl+/ from anywhere, Ctrl+K the palette.
- **lazygit**: single keys for Git actions outside text fields, lowercase for the common one and
  Shift for its stronger sibling: `f` fetch, `p` pull, `P` push, `s` stash, `S` pop, `a` stage all,
  `u` unstage all, `d` discard (which asks first), `c` write the commit message.
- **vim**: motions `j`/`k`, `gg` (a `double` binding) and `G`, Ctrl+D/U and PgDn/PgUp (`held`, so
  they repeat), `/` to filter, `[`/`]` between conflicts. Arrows and Home/End are listed beside
  them, not instead.
- **Fork**: Ctrl+Shift+letter for the same Git actions while typing (Ctrl+Shift+F fetch, +L pull,
  +P push, +H stash, +S stage all).

A Git action therefore usually gets both: a `Section::Anywhere` Ctrl+Shift chord and a bare key in
`Section::Repository`, History, or Changes. Do not take a chord a text field uses for editing
(Ctrl+A, C, V, X, Z, Y) in `Anywhere`: it fires while the user types. Check the table for a free
chord before choosing, and prefer the mnemonic letter.

## 2. Add the table row

In `keymap.rs`:

- Add a variant to `enum Command`.
- Add the row to `BINDINGS`, in the group of its section (the order inside the table is the order
  the cheatsheet shows, and the first matching binding wins). Constructors: `b(section, command,
  &[chords], "description")`; `held(...)` for a binding that repeats while the key is held; struct
  update for the rest (`Binding { double: true, keys_label: "gg  ·  Home", ..b(...) }`;
  `keys_label` only when the label cannot be derived, and written with "Ctrl", which macOS shows
  as Cmd). Chord constructors: `bare`, `shifted`, `symbol` (a character such as `?` `/` `[` that
  needs Shift or AltGr on some layouts), `ctrl`, `ctrl_shift`, `ctrl_symbol`.
- The `description` is the cheatsheet text: sentence case, no trailing full stop, naming what
  happens ("Discard the selected file (asks first)").
- Sections are `Anywhere`, `Navigate` ("Lists and views"), `History`, `Changes`, `Resolve` and
  `Repository`. A new section is rarely right: it must be added to `Section::ALL`, `title` and
  `is_bare`, and the cheatsheet and Settings iterate `Section::ALL`.

## 3. Handle the command (`crates/canopy-app/src/app/input.rs`)

- Add the arm in `CanopyApp::command`; the `match` is exhaustive, so the compiler points at it.
  Guard it by what it needs (`has_tab`, `view == Some(View::Changes)`) and turn it into an
  `Action`, as the neighbouring arms do. A destructive command opens the confirmation (`Action::
  Confirm`) like its menu twin; the shortcut never skips it. Work on a repository is an
  `Action::Op(...)` that goes to the worker (the `add-git-operation` skill), not a call here.
- Give the command a palette entry in `palette_entries` with `Some(Command::X)`, so its hint
  shows, and use `keymap::hint(Command::X)` in tooltips and menus instead of typing the keys.
- Do not add key handling in a view. If a view needs a key, it gets a `Command` and a row.

## 4. Tests (`mod tests` in `keymap.rs`)

Run `cargo test -p canopy-app keymap`. The existing tests guard the rules: no chord bound twice;
bare sections have no Ctrl and `Anywhere` chords need Ctrl, F5 or Escape; `Chord::label` reads
naturally; `matching_respects_shift`; Super never reaches bare keys; `gg` needs two presses; the
cheatsheet lists each command once; `every_command_with_a_hint`. Extend them for the new row:

- an assertion in `every_command_with_a_hint` for the new command's `hint(...)` (the global chord,
  then the bare key, joined by " · ");
- a `labels_read_naturally` case when the chord has a new shape;
- a behaviour test with the `press`/`release`/`fire` helpers for anything beyond a plain press (a
  `double` binding, a repeat, a `symbol` chord).

Never loosen one of the guard tests to fit a chord; change the chord.

## 5. Docs and look

- `docs/keybindings.md` is the reference derived from the table (the module doc of `keymap.rs`
  points at it). Update it in the same change; if the file does not exist yet, say so in your
  report rather than restating the table elsewhere. The README's shortcut table is a copy and
  goes stale: link to the reference instead of extending it.
- `CHANGELOG.md` under `[Unreleased]`, and `docs/features.md` if the feature row changes.
- Screenshot the cheatsheet and Settings → Keyboard (`screenshot` skill): `tools/screenshot.sh
  "$S/showcase" "$S/keys.png" --overlay cheatsheet`, and `--overlay settings`; confirm the new row
  is in the right section with the right label, in dark and light.

## Done when

The row is in `BINDINGS` and nowhere else; `cargo test -p canopy-app keymap` passes; the command
works from the keyboard in the running app with and without a text field focused (a bare key must
do nothing while typing); `tools/check.sh` passes; and `canopy-review` has reviewed the diff.
