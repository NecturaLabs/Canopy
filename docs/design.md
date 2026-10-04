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
| `border` | `#262D3B` | 1px hairlines between regions (decorative) |
| `control-border` | `#696F7A` | Outlines that identify a control: text fields, checkboxes, switch tracks (3:1) |
| `text` | `#E6E9EF` | Primary text |
| `text-dim` | `#A5ADBD` | Secondary text: authors, dates, paths |
| `text-faint` | `#979EAD` | Placeholders, disabled text, line numbers |
| `accent` | `#9C71F8` | Focus rings, selection, primary buttons, HEAD |
| `accent-2` | `#22D3EE` | Gradient end, links, progress |
| `success` | `#34D399` | Added lines, ahead counts, success toasts |
| `warning` | `#FBBF24` | Conflicts, in-progress banners |
| `danger` | `#F87171` | Removed lines, destructive buttons, error toasts |
| `added-bg` | `#0F2A21` | Added line background in diffs |
| `removed-bg` | `#2E1518` | Removed line background in diffs |
| `button`, `button2` | `#7C3AED`, `#0E7490` | Primary fill and companion accent, labelled `on-button` (`#FFFFFF`) |

### Daylight (light)

| Token | Hex | Use |
|---|---|---|
| `bg` | `#F7F8FA` | Window background, history table |
| `panel` | `#FFFFFF` | Sidebar, detail pane, toolbar, status bar |
| `elevated` | `#FFFFFF` | Popups, dialogs, palette (separated by border and shadow) |
| `hover` | `#EEF1F6` | Hovered rows and buttons |
| `border` | `#DDE2EA` | 1px hairlines between regions (decorative) |
| `control-border` | `#858A92` | Outlines that identify a control: text fields, checkboxes, switch tracks (3:1) |
| `text` | `#161A22` | Primary text |
| `text-dim` | `#4B5466` | Secondary text |
| `text-faint` | `#586071` | Placeholders, disabled text, line numbers |
| `accent` | `#7C3AED` | Focus rings, selection, primary buttons, HEAD |
| `accent-2` | `#0D6F8A` | Gradient end, links, progress |
| `success` | `#047555` | Added lines, ahead counts, success toasts |
| `warning` | `#A04A08` | Conflicts, in-progress banners |
| `danger` | `#C72222` | Removed lines, destructive buttons, error toasts |
| `added-bg` | `#E3F7EC` | Added line background in diffs |
| `removed-bg` | `#FDE7E7` | Removed line background in diffs |
| `button`, `button2` | `#7C3AED`, `#0E7490` | Primary fill and companion accent, labelled `on-button` (`#FFFFFF`) |

Every theme passes the contrast test in `theme.rs` for every pairing the views draw: `text` 7:1
and `text-dim` and `text-faint` 4.5:1 on every surface (`bg`, `panel`, `elevated`, `hover`, a
selected row over each, `added-bg`, `removed-bg` and the warning tint); links and SHAs
(`accent-2`) and status colours 4.5:1 on plain surfaces and selected rows, `success` on added
lines, `danger` on removed lines and `warning` on its tint; `accent` 4.5:1 on plain surfaces;
`control-border`, the focus ring and lane colours 3:1. The six other themes define the same tokens
(their lanes and syntax colours are in `theme.rs`):

- **Aurora** (dark): cool arctic blues after the Nord palette.
- **Ember** (dark): warm charcoal with orange and gold.
- **Neon** (dark): deep violet night; electric cyan carries selection and focus and hot pink the
  links, so selection never reads as the orange-red of removed lines and destructive actions.
- **Forest** (dark): deep greens lit with sunlit gold; gold carries selection and focus and sky
  blue the links, so neither is mistaken for the green of added lines.
- **Paper** (light): warm off-white with rust and teal.
- **Contrast** (dark): black and white with saturated signal colours, a blue accent, yellow
  links and strong outlines, for low vision and bright rooms.

| Token | Aurora | Ember | Neon | Forest | Paper | Contrast |
|---|---|---|---|---|---|---|
| `bg` | `#1E222A` | `#16110F` | `#0B0716` | `#0E1411` | `#F4EFE6` | `#000000` |
| `panel` | `#242933` | `#1D1714` | `#110B20` | `#131B17` | `#FBF8F2` | `#0A0A0A` |
| `elevated` | `#2B303B` | `#241C18` | `#170F2B` | `#18221D` | `#FFFDF8` | `#121212` |
| `hover` | `#333A47` | `#2E241F` | `#211638` | `#1F2B25` | `#EDE6DA` | `#242424` |
| `border` | `#3B4252` | `#3A2E27` | `#2D1F4A` | `#2A3830` | `#DCD3C4` | `#6B6B6B` |
| `control-border` | `#7E8490` | `#796E67` | `#6F6386` | `#67746D` | `#8B8379` | `#B3B3B3` |
| `text` | `#ECEFF4` | `#F3E9E2` | `#F5EEFF` | `#E4EEE8` | `#2B2620` | `#FFFFFF` |
| `text-dim` | `#C2C8D3` | `#BCAB9F` | `#CDBFE6` | `#B3C5BB` | `#585044` | `#E6E6E6` |
| `text-faint` | `#B1B8C4` | `#AC9D93` | `#BBAEDC` | `#A7B4AD` | `#655B50` | `#CCCCCC` |
| `accent` | `#88C0D0` | `#F97316` | `#00E5FF` | `#E2B84F` | `#A14A08` | `#4CC2FF` |
| `accent-2` | `#CAAFC5` | `#FACC15` | `#FF7ABD` | `#7DD3FC` | `#0E6E67` | `#FFE14D` |
| `success` | `#A5C08F` | `#4ADE80` | `#3DF5A0` | `#4ADE80` | `#166534` | `#3DFF8A` |
| `warning` | `#EBCB8B` | `#FBBF24` | `#FFD60A` | `#FB923C` | `#825500` | `#FFA62B` |
| `danger` | `#EAA5AB` | `#F87171` | `#FF6B4A` | `#F98282` | `#B91C1C` | `#FF6B6B` |
| `added-bg` | `#253229` | `#15281B` | `#0B2A22` | `#12301F` | `#E3F0DC` | `#002B14` |
| `removed-bg` | `#3A262C` | `#331816` | `#2E0B1E` | `#2E1618` | `#F8E1DC` | `#3B0009` |
| `button` | `#88C0D0` | `#C2410C` | `#0E7490` | `#8A5A00` | `#B45309` | `#005FB8` |
| `button2` | `#B48EAD` | `#A16207` | `#7C3AED` | `#166534` | `#0F766E` | `#005FB8` |
| `on-button` | `#1E222A` | `#FFFFFF` | `#FFFFFF` | `#FFFFFF` | `#FFFFFF` | `#FFFFFF` |

In every theme the accent's hue stays clear of `success` and `danger`, so a selected row is never
confused with an added or removed line.

The accent gradient runs from `accent` to `accent-2` (violet to cyan in Midnight). It is used
sparingly: the active tab indicator, the welcome screen mark and progress bars. The primary
button uses a solid `button` fill, chosen so its `on-button` label stays readable.

Keep all eight themes. Each earns its place by carrying the graph, selection and status colours
through the full interface, rather than borrowing Midnight's lanes or changing only the chrome.

Selected rows will all use `Palette::selected()`: `accent` at low opacity over the row background,
set by `selection_alpha` (18% in most dark themes, 12% in light themes and Ember to keep its orange
calm). An `accent` bar marks the left edge (3px in the history list, 4px on diff lines). egui's
selected values use the same fill. Views still carrying their
own tint move to it in the controls pass.

### Lane colours

Ten colours, assigned to lines of history in order and reused cyclically. Each theme has its own
set, each at least 3:1 against the background, a hovered row and a selected row; the dark themes
Midnight, Ember and Forest share the set below, Daylight uses deeper variants, and Aurora, Neon
and Paper define their own in `theme.rs`. In light themes, edges are drawn at full opacity and
the node fill uses the panel colour.

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
align. Both are bundled (SIL OFL 1.1), so metrics are consistent on Linux, macOS and Windows.
Inter Regular, Medium and SemiBold distinguish body text, controls and headings; monospace is
reserved for code and identifiers. Sizes scale with the interface zoom and display density.

One scale, as tokens in `theme.rs` (`*_SIZE`); views never write a size of their own. Icon glyphs
are sized for their optical weight and are not part of the scale.

| Token | Font | Size (pt) | Use |
|---|---|---|---|
| `HERO_SIZE` | Inter SemiBold | 38 | Welcome screen title |
| `DISPLAY_SIZE` | Inter SemiBold | 24 | The About dialog's name |
| `HEADLINE_SIZE` | Inter SemiBold | 20 | Pull request title in details, first-run dialog headings |
| `TITLE_SIZE` | Inter SemiBold | 16 | View and dialog titles, the palette query |
| `SUBTITLE_SIZE` | Inter SemiBold | 14 | Card and group titles, the repository name, commit summary |
| `LEAD_SIZE` | Inter Medium or SemiBold | 13.5 | The text that leads a row, field labels, toasts, primary buttons |
| `BODY_SIZE` | Inter Regular or Medium | 13 | Default text, table rows, branch names |
| `META_SIZE` | Inter Regular | 12 | Metadata beside a row's main text, setting descriptions |
| `SMALL_SIZE` | Inter Regular or Medium | 11.5 | Hints, counts, badges, status bar |
| `MICRO_SIZE` | Inter SemiBold | 10.5, uppercase, +0.8 tracking | Pills, sidebar sections and table headers |
| `MONO_SIZE` | JetBrains Mono Regular | 12.5 | Diffs, commit message editor, error output |
| `MONO_SMALL_SIZE` | JetBrains Mono Regular | 11.5 | Short SHAs, key chords, History dates and line counts |

The shortcuts sheet keeps whole groups in order and balances its two columns by their estimated
rendered heights, including stacked descriptions; narrow windows use a single column.

Other section headings use sentence case at SMALL semibold in `text-dim`, without tracking.

The global Body, Button, Small and Heading styles use the typography tokens in `theme.rs`.
History rows are 30 points tall, diff lines 20 points.

Settings grows with the viewport up to 1120 × 900 logical points, keeping a 24-point outer
margin and a compact section picker when its navigation sidebar would crowd the content.

## Spacing

Controls (buttons, text fields, combo boxes, checkboxes) are 26 points high; text fields use
`FIELD_MARGIN` (8 by 5) so they match. A 4px base grid. Tokens: `xs` 4, `sm` 8, `md` 12, `lg` 16, `xl` 24. Panels use `md` inner
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
  black at 45% / 14%; the scrim is black at 69% in dark themes or 24% in light themes, so the
  dialog leads over a busy graph without hiding the repository beneath it.

## Motion

Motion is feedback, never decoration. The tokens live in `ui/motion.rs`.

| Token | Duration | Use |
|---|---|---|
| instant | 0 | Anything driven by the keyboard or repeated often: row hover, selection, the focus ring, tab switches, palette filtering, scrolling to the selection, lists updated by a background refresh |
| `FAST` | 100ms | Pointer hover and press feedback on controls, switch thumbs, checkboxes |
| `BASE` | 150ms | Popovers, menus and tooltips appearing; expand and collapse; toasts leaving |
| `SLOW` | 200ms | Dialogs arriving (fade and 0.98 to 1.0 scale); toasts arriving (8px slide and fade) |

Enter and hover transitions ease out (cubic). Exits take about three quarters of the enter time
and only fade; closing a menu, tooltip or the palette is instant. Spinners turn once per second,
linearly.

Running jobs show a spinner in the status bar and on the triggering toolbar button. Outside the
welcome screen, whose glow drifts and whose canopy's halos breathe, no animation loops while
idle, so an idle repository view does not repaint. Reduce motion makes every duration zero and
stills the welcome screen. Today it is an on/off switch in Settings → Appearance; `FAST` times
the segmented controls and switches, while the three-way preference (match the system, reduced
or full) and the `SLOW` token are not wired up yet (the backlog's "Motion" entry). The system setting it will follow is read by
`platform::prefers_reduced_motion` from the desktop portal or GNOME settings on Linux,
`com.apple.universalaccess` on macOS and the client-area animation setting on Windows, off the UI
thread.

## Iconography

Phosphor icons (`egui-phosphor`), regular weight at 16px for toolbar and list icons, fill weight
for state markers (HEAD, conflict, current branch). Icons inherit `text-dim` and take `text` on
hover and `accent` when active. Toolbar buttons pair an icon with a short label; icon-only buttons
always have a tooltip naming the action and its shortcut.

A glyph beside text takes the text's size. A glyph drawn on its own takes one of the icon sizes
in `theme.rs`:

| Token | Size | Use |
|---|---|---|
| `ICON_XS` | 11.5px | Disclosure chevrons, close crosses on tabs and recent repositories |
| `ICON_SM` | 14px | Sidebar row glyphs, tab and segmented-control icons |
| `ICON_MD` | 16px | Buttons, palette rows, settings navigation, request status |
| `ICON_LG` | 18px | Toasts, account cards, the welcome tip |
| `ICON_TITLE` | 20px | Beside a dialog's title |
| `ICON_XL` | 24px | The welcome screen's action cards |
| `ICON_HERO` | 32px | The centre of an empty or finished state |

## Layout

```text
┌──────────────────────────────────────────────────────────────────────────┐
│ Tabs:  [canopy] [tidepool] [+]                        Search   Settings  │
├──────────────────────────────────────────────────────────────────────────┤
│ Toolbar: History|Changes|Requests  Fetch Pull Push Branch Merge Stash   │
├──────────────┬───────────────────────────────────────────────────────────┤
│ Sidebar      │ History: graph │ message │ author │ date │ SHA           │
│ BRANCHES     ├───────────────────┬───────────────────────────────────────┤
│ REMOTES      │ Commit metadata   │ Selected file diff                    │
│ TAGS/STASHES │ and changed files │                                       │
├──────────────┴───────────────────┴───────────────────────────────────────┤
│ Status: branch  ↑2 ↓0  changed files                    path / Git       │
└──────────────────────────────────────────────────────────────────────────┘
```

- Sidebar defaults to 250 points (resizable, 190–460). Below 1000 logical points it moves into
  the toolbar's navigation menu, preserving room for repository content. It stays open for
  section toggles, double-click checkout and context menus.
- History details sit below the graph, initially 40% of its available height capped to 300–320
  points where space permits. The resizable pane can shrink to 260 points; its maximum leaves
  history controls and two complete rows. Expand opens the details in the content area, with Back
  to history to restore the split. Both controls sit beside the commit summary and remain
  available while files load or no file is selected. When that readable split cannot fit, History and Commit details
  share the area through two explicit mode controls; a selected commit initially shows its details.
- Commit details use a compact author, SHA and date above the changed files. Commit information
  starts open, opens independently of pane expansion and retains its state; its body scrolls while
  the files
  stay reachable.
- Commit metadata, the Changes file list, and the request list take at most 40% of their
  available width, leaving the rest for the diff or request details. Pane limits follow the
  current viewport. Read-only diff headers share a wrapping row at short heights to keep more
  changed lines visible without reducing text size.
- Draft requests use a dim pencil icon in the list, distinct from the green open request icon;
  the full state and branch pair remain available on hover.
- The request conversation ends with a primary Comment action. It stays visibly placed below the
  editor when empty, with a short explanation of why it is disabled.
- Review summaries name each reviewer's state in text as well as showing its icon and colour.
- History hides date, author and SHA columns as space decreases; full metadata remains in
  commit details and row tooltips. Row content is clipped to the table's scroll viewport.
- History search stays hidden until Ctrl+F or `/` opens it and focuses the field. The same
  shortcut, Esc or the close button hides it and clears the query. The commit count does not
  occupy the table header; an in-progress operation banner remains visible without search.
- The action toolbar is 44 points high with 40-point buttons and body-size captions. Tabs and toolbar actions scroll horizontally when needed; toolbar overflow has a visible
  scrollbar. The commit form scrolls independently above its fixed Commit button. Long status paths truncate with
  a tooltip. Diff controls share a row with the filename when space permits and wrap below it, and hunk actions use named icons in narrow
  panes.
- When there is no remote, the toolbar says "Local only" as dim status text with an explanatory
  tooltip, rather than styling it as an active control.
- On the welcome screen, the status bar says "No repository open" so it remains informative even
  before the first tab is opened.
- Welcome action cards explain Open, Clone and Init in two short lines without requiring hover;
  the shortcut stays in each card's top corner.
- The welcome illustration's layout bounds end just below its trunk. The stack uses the height
  saved there to sit nearer the vertical centre at tall window sizes, keeping the title close to
  the art and reducing the empty space below the actions.
- Git identity uses an explicit primary Save action and a danger Clear action in the same row
  grid as other settings; its description explains the save boundary.
- The view switcher always includes Requests, disabled with an explanation until a GitHub or
  GitLab remote is available.
- Sidebar remote actions (Publish and Add remote) use outlined compact buttons; local-only
  status is a non-interactive dim label above them.
- An unborn branch counts as a branch in the sidebar and shows its name with "no commits yet" in
  both the sidebar and status bar.
- Diff hunks: the action buttons are 26 px tall. Stage (success tint) is the one filled button;
  Discard is a ghost that turns `danger` under the pointer and sits apart on the right, and on
  very wide panes the actions stay within 960 px of the left edge. A new or deleted file is always
  one column without the empty line-number column (the layout switch shows dimmed on Unified), and
  its lines keep the full added/removed tint only in the gutter, with a quiet wash over the code.
  A line wider than the pane fades out at the edge.
- The in-progress banner (merge, rebase, cherry-pick) appears above the history table with
  continue and abort buttons.
- Changes list headers: "Stage all" and "Unstage all" are 26 px tall (`widgets::COMPACT_H`). Discard
  all lives in a ⋯ menu (danger-coloured item, then a confirmation) a gap away from Stage. A zero
  count is neutral, not warning amber, and an empty section is left out while another has content
  (a conflict hides Unstaged and Staged when they are empty).
- The disabled primary button is flat with a hairline outline and faint text in every theme, never
  the primary fill. When the commit button is disabled for a reason its label does not give, a line
  above it says so ("Write a summary to commit", "Resolve the conflicts to commit").
- The commit composer follows the file lists while both fit the column, under a hairline, and
  pins to the bottom with the lists scrolling above it only when they would crowd it (about
  260 points of composer). Row actions (stage, unstage, mark resolved) show on the selected row
  and under the pointer on any row.
- A clean working tree gives the main pane its own state: the illustration, "Working tree
  clean", where the branch stands (up to date, N to push or pull with that button, or local only
  with Publish…) and the last commit. The file column then shows only "Changes 0" and a quiet
  composer, "Nothing to commit" and the Amend choice, which expands to the full composer when
  ticked or when text is typed.
- Minimum window size is 900×560 desktop units (`app::update_window_minimum`); interface zoom does not increase it. Compact layouts are checked at 960×600 and at
  interface zoom up to 160%; dimensions above are logical points after zoom.
- Below `COMPACT_WIDTH` the view switcher names only the active view; the others keep their icon
  and count. In windows under 600 points tall the history inspector starts at 260 points and the
  commit composer stops at 260, so the lists keep their rows. Prose and its fields (pull request
  details) stop at `MEASURE`, 920 points, and the welcome screen centres its content on tall
  windows. Layouts are checked at 900×560, 960×600, 1440×900 and 2560×1440.

## Components

### Ref pill

Rounded rectangle (10-point radius, height 19px, horizontal padding 7px) with a Small Medium label and a
leading icon. Branch, remote, tag and HEAD colours use distinct tinted fills and outlines;
the actual drawn text uses a neutral ink with at least 4.5:1 contrast against its fill.
When a local branch and its upstream point to the same commit they merge into one pill showing
both icons. Pills that do not fit collapse to a `+N` badge with a tooltip.

### Segmented control

`widgets::segmented` (text) and `widgets::icon_segmented` (icons with tooltips) for a choice
between a few exclusive options: settings choices, the merge dialog's merge or rebase, the clone
source and protocol, the request state filter and the diff layout. A recessed `bg` track with a
hairline `border`; the chosen segment is raised (`elevated`, `accent` outline at 60%) with `text`,
the others `text-dim` brightening on hover. Each segment is a radio button for assistive
technology and shows the focus ring.

### Switch and checkbox

On/off settings use `widgets::switch`: a 36×20 track, `bg` with a `control-border` outline when off
and the `button` colour when on, with a 14-point thumb that slides (`FAST`). Checkboxes in forms
use `widgets::checkbox`: a 16-point square, radius 4, filled with `button` and an `on-button` tick
when checked. Both carry their name for assistive technology and show the focus ring.

### Scroll edges

Lists that scroll inside a fixed area (the command palette, the cheatsheet) fade their top and
bottom 12 points into the surface, at 85% strength, where more content lies beyond, so a cut row reads
as "more" (`widgets::scroll_fade`). The palette's list also shows whole rows only. A scrollbar's handle
is never shorter than 32 points, so a long document's stays easy to see and grab.

### Illustrations

Drawn in code in the theme's colours (`ui/art.rs`, `docs/art-prompts.md`): the logo mark, the
welcome canopy and four empty-state spots. Each grows in once when it appears and then stays
still; reduce motion shows it finished.

### Context menu

An opened submenu item uses the same neutral surface as a hovered menu item.
Reset choices put the mode name in semibold type above a dim description, with destructive
Hard reset marked in danger colour and confirmed before it runs.

### Avatar

A circle, 20px in tables and 24px in details, filled with a lane colour chosen by hashing the
displayed identity after case and punctuation are removed. A Git author name and matching host
handle therefore get the same colour without needing an account lookup. At these sizes it shows
one larger initial for legibility; larger circles can show two. Account pictures fill the same
circle, with initials while unavailable or downloads are
disabled. History and commit details share a twelve-hour local cache; a failed refresh retains
the last picture.

### Toast

`elevated` card, `radius`, raised shadow, max width 420px, stacked bottom-right with `sm` gaps. A
3px left stripe and icon in `success`, `warning`, `danger` or `accent-2` (info). Errors show git's
stderr in Mono, scrollable, with a copy button, and stay until dismissed; other toasts dismiss
after 4 seconds, pausing while hovered.

### Command palette

Placed near the top of the window, up to 620 points wide, `radius-lg`, overlay elevation. Its
height stays inside the viewport. A search field at the top, then scrollable results: icon, command name with matched characters in `accent`, and
the shortcut right-aligned in `text-faint`. Up/Down move, Enter runs, Esc closes. Branch names
appear as "Checkout <branch>" entries.

Field placeholders end in an ellipsis when they ask the user to act (for example, "Search
shortcuts…"); example values and field names do not.

The toolbar stays visible for frequent repository actions and status; the palette is the faster
keyboard path and the home for less frequent actions. Neither hides essential actions available
through the other. At narrow widths, compact toolbar controls preserve room for repository data
while their labels and shortcuts remain discoverable in tooltips and the palette.

### Dialog

Overlay card, `radius-lg`, `xl` padding, 460 points wide by default, bounded by the viewport. Title, body, then right-aligned
buttons: secondary (outlined) then primary (the solid `button` fill, the same widget everywhere,
with a focus ring). Destructive confirmations use a
solid `danger` primary button (`widgets::danger_primary_button`, its text white or black by contrast
with the theme's `danger`) whose label names the action ("Delete branch", "Discard 3 files"), never
"OK"; Cancel has the initial focus, so Enter does not confirm a destructive dialog unless the button
has focus. A choice that is riskier than the main one (trusting a whole folder) is a quiet link in
`warning` with a line saying what it covers, never a second full-width button. Informational
destinations (About: What's new, Releases, the legal texts) are text links in `accent`, with one
primary button for the real action.

The merge dialog explains every merge mode beside its radio choice so they can be compared before
selection. The sentence below the choices still names the exact branches affected by the selected
mode.

Action dialogs lead with the effect on the repository and the choice Canopy will pass to Git.
They do not append a universal command preview: an action may run several Git commands, its
arguments can depend on repository state at execution time, and a guessed preview would imply a
guarantee it cannot keep. A future optional command disclosure must use the same redacted argv
that the operation actually executes. Differences from the user's Git configuration are shown
with the setting, value and source, as the Git settings work in the backlog requires.

### Git identity

Settings → Git starts with an "Identity for future commits" group built from the ordinary settings
rows: Name and Email, each with a text field and, as its help text, what Git uses now and where it
comes from ("In use now: Ada (global: ~/.gitconfig)"); a Committer row only when the committer
differs; a "Save to" choice between this repository and all repositories (only with a repository
open); Save identity, which stays disabled until both fields have text; and Clear…, which asks first
in a `danger` confirmation. A `warning` line with the warning icon sits above the rows when Git
cannot use the identity, guessed part of it, or it looks like a placeholder. Fields offer only
values someone chose, never Git's guess.

The Changes view repeats that line (`warning`, small size) with an "Edit identity" button above the
commit button, in a footer anchored to the bottom of the commit area: the fields scroll and the
commit button always stays visible, however the warning wraps. The button's tooltip explains the
warning. The first-launch dialog ("Set your Git identity", name and email, "Save globally", Cancel)
is an ordinary dialog opened once after the consent dialogs. History's column header says "Author"; it shows what each commit recorded, independently of current settings.

### Graph node and edges

- Lane spacing 16px, node radius 4.5px, edge width 2px, all in the lane colour.
- Edges within a lane are straight vertical lines; lane changes are cubic Bezier curves spanning
  one row, with control points at half the row height so curves leave and enter vertically.
- Regular commits are filled circles with a 1.5px ring in the background colour to separate them
  from crossing edges.
- Merge commits (two or more parents) are hollow: background-filled with a 2px ring in the lane
  colour.
- The `HEAD` commit has an additional soft glow ring (`accent` at 35%, radius +4px).
- Filtering shows matching commits without graph lanes; clearing the filter restores the graph.

### Settings and native selection

Settings stay within the viewport with an independently scrolling content area. A section menu
replaces the left navigation below 720 points. Below 620 points of content width, each setting's
label and help sit above its controls. Shortcut descriptions and keycaps stack when they cannot
fit together. Zoom shows a percentage preview while dragging and applies
on release, so the control stays under the pointer; typing a percentage applies on Enter or
when the field loses focus. A row's label column is always half the width, so controls of different
rows line up on the right however short their help text is.

A row's help is one sentence; longer explanation sits behind a "More" link and opens beneath the
row at reading width (560 points). Settings → Keyboard shows one key column of the cheatsheet's
keycaps with the descriptions on a shared edge. A selected theme card has an accent ring, a halo
and a tick; hover is a neutral hairline. Cards fill the row in whole columns and reflow with the
width. Text-field placeholders are italic and faint, and fields in a section share one width.
Long paths in settings are cut in the middle with the whole path in a tooltip.

Browse opens the native folder chooser on a background thread. The requesting button displays
“Choosing…” while it is open; the app keeps processing events. A closed requesting field discards
late results. Cancellation leaves the field unchanged, and a failed chooser allows retry. Clone destination
validation runs in the clone worker; an existing folder is refused before Git runs. The Clone form
keeps its values on failure, disables edits while running, and offers Continue in background.
Constrained commit metadata and diff scrollers can shrink below their toolkit default height.
Dialog bodies scroll within the viewport with a visible scrollbar, keeping the frame on screen
at maximum zoom and minimum desktop size. Legal and consent choices stay outside their scrolling
bodies; short windows pin them at the bottom. Tooltips from underlying content disappear when
a blocking overlay opens.

Update progress and completion dialogs keep their action buttons in a content-height horizontal row, immediately below the status text, rather than expanding to fill the viewport.
