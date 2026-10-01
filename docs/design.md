# Design system

Canopy's visual language: a calm, dark workspace where colour is reserved for meaning (branches,
state, actions). All values here are implemented as tokens in
`crates/canopy-app/src/ui/theme.rs`, which is the only place colours, spacing and radii are
defined. When a token or component changes, update this document in the same change.

## Principles

1. **Content first.** The commit graph, file lists and diffs carry the colour. Chrome stays
   neutral: panels differ from the background by one or two steps of lightness, not by hue.
2. **Colour has a job.** Accent marks focus and primary actions; lane colours identify lines of
   history; success, warning and danger mean exactly that. Nothing is coloured for decoration.
3. **Dense but calm.** Fork-level information density with generous line height, hairline borders
   instead of heavy separators, and aligned columns.
4. **Instant feedback.** Every action responds within a frame (hover, press, selection); work that
   takes longer shows a spinner and ends in a toast.
5. **Keyboard parity.** Everything reachable by mouse is reachable from the keyboard or the command
   palette.

## Colour tokens

### Midnight (dark, default)

| Token | Hex | Use |
|---|---|---|
| `bg` | `#0D1016` | Window background, history table |
| `panel` | `#12161E` | Sidebar, detail pane, toolbar, status bar |
| `elevated` | `#171C26` | Popups, dialogs, palette, menus, cards |
| `hover` | `#1E2431` | Hovered rows and buttons |
| `border` | `#262D3B` | 1px hairlines, input outlines |
| `text` | `#E6E9EF` | Primary text |
| `text-dim` | `#9AA3B5` | Secondary text: authors, dates, paths |
| `text-faint` | `#636B7D` | Placeholders, disabled text, line numbers |
| `accent` | `#8B5CF6` | Focus rings, selection, primary buttons, HEAD |
| `accent-2` | `#22D3EE` | Gradient end, links, progress |
| `success` | `#34D399` | Added lines, ahead counts, success toasts |
| `warning` | `#FBBF24` | Conflicts, in-progress banners |
| `danger` | `#F87171` | Removed lines, destructive buttons, error toasts |
| `added-bg` | `#0F2A21` | Added line background in diffs |
| `removed-bg` | `#2E1518` | Removed line background in diffs |

### Daylight (light)

| Token | Hex | Use |
|---|---|---|
| `bg` | `#F7F8FA` | Window background, history table |
| `panel` | `#FFFFFF` | Sidebar, detail pane, toolbar, status bar |
| `elevated` | `#FFFFFF` | Popups, dialogs, palette (separated by border and shadow) |
| `hover` | `#EEF1F6` | Hovered rows and buttons |
| `border` | `#DDE2EA` | 1px hairlines, input outlines |
| `text` | `#161A22` | Primary text |
| `text-dim` | `#566074` | Secondary text |
| `text-faint` | `#8A93A5` | Placeholders, disabled text, line numbers |
| `accent` | `#7C3AED` | Focus rings, selection, primary buttons, HEAD |
| `accent-2` | `#0891B2` | Gradient end, links, progress |
| `success` | `#059669` | Added lines, ahead counts, success toasts |
| `warning` | `#D97706` | Conflicts, in-progress banners |
| `danger` | `#DC2626` | Removed lines, destructive buttons, error toasts |
| `added-bg` | `#E3F7EC` | Added line background in diffs |
| `removed-bg` | `#FDE7E7` | Removed line background in diffs |

The accent gradient runs from `accent` to `accent-2` (violet to cyan in Midnight). It is used
sparingly: the primary button, the active tab indicator, the welcome screen mark and progress
bars.

Selected rows use `accent` at low opacity over the row background (about 18% in Midnight, 12% in
Daylight) with a 2px `accent` bar on the left edge.

### Lane colours

Ten colours, assigned to lines of history in order and reused cyclically. The same values are used
in both themes; in Daylight, edges are drawn at full opacity on the light background and the node
fill uses the panel colour.

| Index | Hex | | Index | Hex |
|---|---|---|---|---|
| 0 | `#8B5CF6` violet | | 5 | `#60A5FA` blue |
| 1 | `#22D3EE` cyan | | 6 | `#F87171` red |
| 2 | `#F472B6` pink | | 7 | `#A3E635` lime |
| 3 | `#34D399` green | | 8 | `#FB923C` orange |
| 4 | `#FBBF24` amber | | 9 | `#C084FC` purple |

Avatar backgrounds are picked from the same ten colours by a stable hash of the author's email.

## Typography

Inter for interface text, JetBrains Mono for SHAs, diffs, paths in diffs and anything that must
align. Both are bundled (SIL OFL 1.1).

| Style | Font | Size (pt) | Use |
|---|---|---|---|
| Display | Inter SemiBold | 22 | Welcome screen title |
| Title | Inter SemiBold | 16 | Dialog titles, commit summary in details |
| Heading | Inter SemiBold | 12, uppercase, +0.5 tracking | Sidebar section headers |
| Body | Inter Regular | 13 | Default text, table rows |
| Body strong | Inter Medium | 13 | Branch names, selected items, buttons |
| Small | Inter Regular | 11.5 | Dates, counts, status bar, badges |
| Mono | JetBrains Mono Regular | 12.5 | Diffs, SHAs, commit message editor |

Line height is 1.4 for body text; history rows are 28px tall, diff lines 20px.

## Spacing

A 4px base grid. Tokens: `xs` 4, `sm` 8, `md` 12, `lg` 16, `xl` 24. Panels use `md` inner
padding; dialogs `xl`; rows `sm` horizontally. Related controls are `sm` apart, groups `lg`.

## Radii

| Token | Value | Use |
|---|---|---|
| `radius-sm` | 6px | Buttons, inputs, ref pills, badges |
| `radius` | 8px | Menus, list selection, toasts |
| `radius-lg` | 12px | Dialogs, command palette, cards on the welcome screen |

## Elevation

Three levels, by background token plus shadow:

- **Base** (`bg`, `panel`): no shadow; panels are separated by 1px `border` hairlines.
- **Raised** (`elevated`): menus, tooltips, toasts. Shadow offset (0, 4), blur 16, black at 35%
  (Midnight) or 10% (Daylight).
- **Overlay** (`elevated` over a scrim): dialogs and the palette. Shadow offset (0, 12), blur 32,
  black at 45% / 14%; the scrim is black at 40% (Midnight) or 20% (Daylight).

## Motion

| Interaction | Duration | Curve |
|---|---|---|
| Hover fade (background, icon colour) | 120ms | ease-out |
| Selection highlight move | 180ms | ease-out |
| Toast enter (slide up 8px and fade in) and exit | 250ms | ease-out / ease-in |
| Dialog and palette open | 180ms fade and 0.98 to 1.0 scale | ease-out |

Running jobs show a spinner in the status bar and on the triggering toolbar button. No animation
loops while idle, so an idle Canopy does not repaint. Respect reduced motion where the platform
reports it: switch transitions to instant.

## Iconography

Phosphor icons (`egui-phosphor`), regular weight at 16px for toolbar and list icons, fill weight
for state markers (HEAD, conflict, current branch). Icons inherit `text-dim` and take `text` on
hover and `accent` when active. Toolbar buttons pair an icon with a short label; icon-only buttons
always have a tooltip naming the action and its shortcut.

## Layout

```text
┌──────────────────────────────────────────────────────────────────────────┐
│ Tabs:  [● canopy] [tidepool] [+]                                         │
├──────────────────────────────────────────────────────────────────────────┤
│ Toolbar: Fetch  Pull  Push │ Branch  Stash │ History|Changes     Palette │
├──────────────┬──────────────────────────────────────┬────────────────────┤
│ Sidebar      │ Centre                               │ Detail             │
│  BRANCHES    │  History: graph │ message │ author │ │  commit metadata   │
│  REMOTES     │           date │ SHA                 │  changed files     │
│  TAGS        │  or Changes: unstaged / staged lists │  diff              │
│  STASHES     │  and commit composer                 │                    │
├──────────────┴──────────────────────────────────────┴────────────────────┤
│ Status bar: branch  ↑2 ↓0   MERGING   ⟳ Fetching origin…                 │
└──────────────────────────────────────────────────────────────────────────┘
```

- Sidebar default width 240px (resizable, 180 to 400px); detail pane default 40% of the remaining
  width.
- In Changes, the centre holds the file lists and composer and the detail pane holds the diff.
- The in-progress banner (merge, rebase, cherry-pick) spans the centre column above the table, in
  `warning` with continue and abort buttons.
- Minimum window size 960x600.

## Components

### Ref pill

Rounded rectangle (`radius-sm`, height 18px, horizontal padding 6px) with a Small Medium label and a
leading icon. Local branches use the lane colour of their commit at 20% opacity with a 1px border
in the same colour; remote branches use an outlined style with a cloud icon; tags use `text-dim`
outline with a tag icon. `HEAD` and the current branch are filled with `accent` and white text.
When a local branch and its upstream point to the same commit they merge into one pill showing
both icons. Pills that do not fit collapse to a `+N` badge with a tooltip.

### Avatar

A circle, 20px in tables and 32px in details, filled with a lane colour chosen by hashing the
author's email, showing up to two initials in Small SemiBold white. Optional remote avatar images
are a planned, opt-in setting.

### Toast

`elevated` card, `radius`, raised shadow, max width 420px, stacked bottom-right with `sm` gaps. A
3px left stripe and icon in `success`, `warning`, `danger` or `accent-2` (info). Errors show git's
stderr in Mono, scrollable, with a copy button, and stay until dismissed; other toasts dismiss
after 4 seconds, pausing while hovered.

### Command palette

Centred at the top third of the window, 560px wide, `radius-lg`, overlay elevation. A search field
at the top, then up to ten results: icon, command name with matched characters in `accent`, and
the shortcut right-aligned in `text-faint`. Up/Down move, Enter runs, Esc closes. Branch names
appear as "Checkout <branch>" entries.

### Dialog

Overlay card, `radius-lg`, `xl` padding, 440px wide by default. Title, body, then right-aligned
buttons: secondary (outlined) then primary (accent gradient). Destructive confirmations use a
`danger` primary button whose label names the action ("Delete branch", "Discard 3 files"), never
"OK"; Enter does not confirm a destructive dialog unless the button has focus.

### Graph node and edges

- Lane spacing 14px, node radius 4.5px, edge width 2px, all in the lane colour.
- Edges within a lane are straight vertical lines; lane changes are cubic Bezier curves spanning
  one row, with control points at half the row height so curves leave and enter vertically.
- Regular commits are filled circles with a 1.5px ring in the background colour to separate them
  from crossing edges.
- Merge commits (two or more parents) are hollow: background-filled with a 2px ring in the lane
  colour.
- The `HEAD` commit has an additional soft glow ring (`accent` at 35%, radius +4px).
- Rows not matching the active filter are dimmed to 35% opacity rather than removed, so the graph
  stays connected.
