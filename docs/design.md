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

Every colour is defined in `ui/theme.rs` and nowhere else. A theme is a `Palette`: the colours
both styles share (text, accents, status, lanes, syntax, focus, selection, shadows, scrim, glass)
plus two `Surfaces` sets, one per Style. Views read the active set with `theme::surfaces(ctx)` and
the shared colours with `theme::palette(ctx)`.

- **Standard** (the default) is round-2 concept 1: flat fills, two-tone chrome (`panel` one step
  lighter than the `bg` ground in dark themes, darker in light), flush panes divided by 1 pt
  `border` seams, raised `raised` secondary buttons with a 1 pt `raised_edge`, a solid `primary`,
  and soft shadows only under floating layers. Content is painted on `pane`, which equals `bg`.
- **Deeper** is concept 4: a darker ground with the chrome on it, content panes as floating cards
  (`pane`, `pane_edge`) over gutters, gradient fills (`Fill { top, bottom }`), stronger highlights,
  layered shadows and a focused-pane edge in `focus`. The tab strip and action bar form one lifted
  `deck`; the table header strip is `pane_header` with a `focus`-tinted seam, so neither reads as
  one more neutral edge.

Interaction states are translucent overlays of `hover_ink`: `hover()`, `hover_strong()`,
`pressed()` and Dense's `zebra()`; selection is the `selected()` wash with no side bar. Status
tints (`success_tint()` and so on) are 16%. `danger` is the text-grade red; `danger_base` is
concept 1's `#e5484d` red for error rings, the danger button's focus ring and danger fills;
`danger_solid` fills the danger button. Light themes ring the switch knob in `knob_edge`.

Midnight and Daylight carry concept 1's values; the other six themes keep their own colour
identities on the same lightness ladders. Every pairing (text on every surface and state, labels
on every fill end, focus, outlines, lanes, syntax, coloured components, glass over any backdrop)
is a test in `theme.rs`, run for every theme in both styles; `Palette::ink` keeps coloured text
readable on its own tint.

### Lane colours

Ten colours, assigned to lines of history in order and reused cyclically. Each theme has its own
set, each at least 3:1 against the background, a hovered row and a selected row; Midnight uses the set below, Daylight uses deeper variants, and the other themes define
their own in `theme.rs`. Ember leads with orange and gold, Forest with sunlit gold, sky blue and
mint, and Aurora with arctic blue and muted lilac. In light themes, edges are drawn at full opacity and
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

One scale, as fields of the density's metrics (`theme::metrics()`); views never write a size of
their own. Dense shrinks the scale where noted. Icon glyphs are sized for their optical weight and
are not part of the scale.

| Token | Font | Size (pt), Comfortable / Dense | Use |
|---|---|---|---|
| `hero_size` | Inter SemiBold | 38 | Welcome screen title |
| `display_size` | Inter SemiBold | 24 | The About dialog's name |
| `headline_size` | Inter SemiBold | 15.5 / 14 | The detail pane's commit subject, first-run dialog headings |
| `title_size` | Inter SemiBold | 15 | View and dialog titles, the palette query |
| `subtitle_size` | Inter SemiBold | 14 / 13.5 | Card and group titles, the repository name |
| `lead_size` | Inter Medium or SemiBold | 13 / 12 | The text that leads a row, field labels, primary buttons |
| `body_size` | Inter Regular or Medium | 13 / 12 | Default text, table rows, buttons |
| `meta_size` | Inter Regular | 12.5 / 12 | Metadata beside a row's main text, toast bodies, setting descriptions |
| `small_size` | Inter Regular or Medium | 11.5 / 11 | Hints, counts, ref pills, status bar |
| `micro_size` | Inter SemiBold | 10.5, uppercase, +0.8 tracking | Uppercase section labels and table headers |
| `mono_size` | JetBrains Mono Regular | 12.5 / 12 | Diffs, commit message editor, error output |
| `sha_size` | JetBrains Mono Regular | 12 | Short SHAs and History dates |
| `chord_size` | JetBrains Mono Regular | 11 / 10.5 | Keycaps |

Settings → Keyboard reuses the shortcuts sheet rows and keycaps. Palette entries lead with
repository and current-view actions before app commands; staging buttons include their keys
in tooltips.

The shortcuts sheet keeps whole groups in order and balances its two columns by their estimated
rendered heights, including stacked descriptions; narrow windows use a single column.

Other section headings use sentence case at SMALL semibold in `text-dim`, without tracking.

The global Body, Button, Small and Heading styles use the typography tokens in `theme.rs`.
History rows are 30 points tall, diff lines 20 points.

Settings grows with the viewport up to 1120 × 900 logical points, keeping a 24-point outer
margin and a compact section picker when its navigation sidebar would crowd the content.

## Spacing

A 4 pt base grid: `XS` 4, `SM` 8, `MD` 12, `LG` 16, `XL` 24, `XXL` 32. Everything else that has a
size (type scale, icons, radii, control and row heights, toolbar anatomy, graph geometry) is a
`Metrics` field, one table per (Density, Style) pair, read with `theme::metrics()`.

| | Comfortable (default, concept 1) | Dense (concept 5) |
|---|---|---|
| Body text | 13 | 12 |
| Controls (sm / md / lg) | 24 / 28 / 32 | 20 / 24 / 28 |
| History row / file row | 28 / 30 | 24 / 24 |
| Tab strip / action bar / status bar | 42 / 58 / 28 | 34 / 36 / 24 |
| Tool buttons | 46 tall, icon over caption | 26 tall, icon beside label |
| Zebra rows | off | on |

Deeper overrides the geometry only: controls 30 (26 Dense), panes as cards with a 6 pt gutter
(4 Dense).

## Radii

| Field | Comfortable | Dense | Deeper (Comfortable / Dense) |
|---|---|---|---|
| `radius_sm` | 5 | 3 | 6 / 4 |
| `radius` | 7 | 5 | 10 / 7 |
| `radius_lg` | 12 | 6 | 14 / 8 |
| `pill_radius` | full | 3 | 6 / 4 |

## Elevation

`theme::material(style, dark, level)` gives each `Elevation` level its top highlight, bottom line,
recess and shadow layers; `kit::paint::surface` paints them. In Standard only buttons, the
segmented thumb and the switch knob carry a 1 to 2 px contact shadow, and floating layers (menus,
popovers, tooltips, toasts, dialogs, palette, cheatsheet) a two-layer soft shadow with a 1 pt
`overlay_edge` ring. Deeper adds two-layer shadows to controls, coloured glows under primary and
danger buttons, the `Pane` shadow under cards, the field recess and the deck. Contrast has no
highlights and rings floating layers in `control_border`. The top highlight line is white at the
level's alpha, from `theme::highlight_line`.

Translucent surfaces (off by default) blur what is behind floating layers: a layer asks
`kit::glass::layer_fill` for its fill, which adds the blurred backdrop (dual Kawase, about 12 pt,
saturation x1.15, luminance clamped to `glass_luminance`, dithered) and returns `overlay` at
`glass_alpha`; `kit::paint::surface` moves the backdrop between the layer's shadow and its fill.
The command palette uses it today (it places `kit::glass::backdrop_shape` under its egui frame
itself); the other floating layers do as they move to the kit. At most four layers a frame are
glass; tooltips, the settings screen and panes stay opaque. The
setting is unavailable with the Contrast theme, when the system asks for reduced transparency
(macOS, Windows) and when the graphics driver cannot run the blur; any failure at run time turns
it off for the session and every layer is opaque again.

## Motion

Motion is feedback, never decoration. Durations, curves and springs live in `ui/motion.rs`; the
kit's helpers in `ui/kit/anim.rs`.

| Token | Duration | Use |
|---|---|---|
| (none) | 0 | Rows, selection, menu-item highlight, palette filtering, keyboard-opened menus change at once |
| `FAST` / `FAST_OUT` | 80 / 120ms | Hover in and out, press release, focus ring in |
| `BASE` | 150ms | Chevrons, label crossfades, tooltips, field halo |
| `SLOW` | 220ms | Dialog and popover entrance |
| `TOAST_IN` | 280ms | Toast slide-in |
| `TOAST_SHIFT` | 200ms | Toasts sliding into the place of one that left |
| `TAB_CLOSE` | 250ms | A closed repository tab collapsing |
| `EXIT_POPUP` / `EXIT_DIALOG` / `EXIT_TOAST` | 100 / 120 / 160ms | Exits |
| `TOOLTIP_DELAY` / `TOOLTIP_SKIP` | 450 / 300ms | First tooltip delay; no delay right after one closed |
| `LOADING_DELAY` / `LOADING_MIN` | 150 / 400ms | Spinners appear late and stay a minimum time |

Curves are cubic Béziers (`EASE_OUT` by default); springs (`SWITCH`, `PRESS`, the indicator
pair) use the exact damped solution and repaint only while moving. Buttons scale their content
to 0.97 while pressed. Settings → Appearance → Motion offers Match system (the default), Reduced
and Full; the system answer comes from `platform::prefers_reduced_motion` on a background thread.
Reduced makes every duration zero, lands every spring at once, turns the press scale off and
stills the welcome screen. Only spinners and skeletons repaint continuously, and only while shown.

## Iconography

Phosphor icons (`egui-phosphor`), regular weight at 16px for toolbar and list icons, fill weight
for state markers (HEAD, conflict, current branch). Icons inherit `text-dim` and take `text` on
hover and `accent` when active. Toolbar buttons pair an icon with a short label; icon-only buttons
always have a tooltip naming the action and its shortcut.

A glyph beside text takes the text's size. A glyph drawn on its own takes one of the icon sizes
in the density's metrics:

| Token | Size (px), Comfortable / Dense | Use |
|---|---|---|
| `icon_xs` | 12 | Disclosure chevrons, close crosses on tabs and recent repositories |
| `icon_sm` | 13 | Sidebar row glyphs, tab and segmented-control icons |
| `icon_md` | 15 | Buttons, palette rows, settings navigation, request status |
| `icon_lg` | 16 / 15 | Beside a dialog's title |
| `icon_tool` | 17 / 15 | Toolbar tools |
| `icon_toast` | 18 | A notification's leading glyph |
| `icon_xl` | 24 / 22 | The welcome screen's action cards |
| `icon_hero` | 32 / 28 | The centre of an empty or finished state |

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
- History column widths follow the currently visible rows: graph lane spans, ref pills and
  messages, author names and formatted dates. Metadata sits after the measured message width
  instead of at the far window edge. Narrow views omit lower-priority columns; long messages
  elide, and busy graphs compress their lane spacing. Measurements stay bounded to the viewport.
- History details sit below the graph, initially 40% of its available height, growing with
  a tall window. The resizable pane can shrink to 260 points; its maximum leaves
  history controls and six complete rows. Expand opens the details in the content area, with Back
  to history to restore the split. Both controls sit beside the commit summary and remain
  available while files load or no file is selected. When that readable split cannot fit, History and Commit details
  share the area through two explicit mode controls; a selected commit initially shows its details.
- Commit details use a compact author, SHA and date above the changed files. Commit information
  starts open, opens independently of pane expansion and retains its state; its body scrolls while
  the files
  stay reachable. The open metadata body reserves room for up to two changed files and their
  controls. History prefers six visible rows and can retain the split with two rows in a short
  window. Automatic inspector height adapts when the diff arrives; a manual resize takes precedence.
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
- The action toolbar uses `ACTION_BAR_HEIGHT` (52 points), with 40-point tools, body-size captions
  and room for its horizontal scrollbar. Tabs and toolbar actions scroll horizontally when needed; toolbar overflow has a visible
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
- SSH host trust uses equal outlined choices, with refusal focused when nothing else has focus.
  Host/fingerprint lead; raw SSH text is in Details. Copy and published-fingerprint comparison
  links help users verify the key; parsing never grants trust.
- Request details separate navigation from review actions. The method picker and primary Merge
  action form one group; Close is in More. Description precedes expanded checks and reviews.
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
- The shared operation banner (merge, rebase, cherry-pick) leads History, Changes and Requests.
  State and conflict count precede Continue; unresolved conflicts or active work disable it.
  Abort is separated and confirmed. The merge composer says Conclude merge and starts with
  Git's pending message when no draft exists.
- Changes list headers: "Stage all" and "Unstage all" are small kit buttons (`Size::Sm`). Discard
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
- The clean summary scrolls in a short pane so Publish/Push/Pull and the last commit remain
  reachable at maximum zoom. The conflict resolver's body can shrink below its normal scroll
  minimum rather than painting code across its fixed Save footer.
- Minimum window size is 900×560 desktop units (`app::update_window_minimum`); interface zoom does not increase it. Compact layouts are checked at 960×600 and at
  interface zoom up to 160%; dimensions above are logical points after zoom.
- Below `COMPACT_WIDTH` the view switcher names only the active view; the others keep their icon
  and count. In windows under 600 points tall the history inspector starts at 260 points and the
  commit composer stops at 260, so the lists keep their rows. Prose and its fields (pull request
  details) stop at `MEASURE`, 920 points, and the welcome screen centres its content on tall
  windows. Layouts are checked at 900×560, 960×600, 1440×900 and 2560×1440.

## Components

### Component library

`ui/kit/` holds the components. Each is a builder: `new(..)` with the required inputs, chained
setters (`size`, `icon`, `enabled`, `tooltip`, `disabled_reason`, `shortcut`, `id_salt`,
`full_width`, `min_width`), and `show(ui)`; each also implements `egui::Widget`. Components read
colours, metrics and options themselves and report their AccessKit role and label. Buttons come in
Primary, Secondary, Ghost and Danger at Sm, Md and Lg, plus icon, tool, link and split buttons;
disabled controls are the whole control at 42% and stay hoverable for their reason. Keyboard focus
draws a 2 pt ring 2 pt outside the control (`danger_base` on danger buttons), only after keyboard
use. `tools/screenshot.sh <repo> out.png --overlay gallery [--style deeper] [--density dense]
[--gallery-page N]` renders every component in every state; a page past the end shows the last,
and each page is numbered at the bottom right.


### Ref pill

Rounded rectangle (10-point radius, height 19px, horizontal padding 7px) with a Small Medium label and a
leading icon. Branch, remote, tag and HEAD colours use distinct tinted fills and outlines;
the actual drawn text uses a neutral ink with at least 4.5:1 contrast against its fill.
When a local branch and its upstream point to the same commit they merge into one pill showing
both icons. Pills that do not fit collapse to a `+N` badge with a tooltip.

### Segmented control

`kit::choice::Segmented` (text, icons with tooltips, or both) for a choice
between a few exclusive options: settings choices, the merge dialog's merge or rebase, the clone
source and protocol, the request state filter and the diff layout. A recessed `bg` track with a
hairline `border`; the chosen segment is raised (`elevated`, restrained edge and micro-shadow),
with an accent baseline and `text`,
the others `text-dim` brightening on hover. Each segment is a radio button for assistive
technology and shows the focus ring.

### Switch and checkbox

On/off settings use `kit::toggle::Switch`: a 40×24 track, `bg` with a `control-border` outline when off
and the `button` colour when on, with an 18-point thumb that slides (`FAST`). Checkboxes in forms
use `kit::toggle::Checkbox`: an 18-point square, radius 4, filled with `button` and an `on-button` tick
when checked. Both carry their name for assistive technology and show the focus ring.

### Scroll edges

Bodies that scroll inside a card (the command palette, the cheatsheet, dialog bodies) are
`kit::overlay::scroll_body`: they fade their top and bottom 12 points into the card's ground, at 85%
strength, where more content lies beyond, so a cut row reads as "more" (`kit::paint::scroll_fade`);
their scrollbar stands 2 points in from the card's edge; and once they overflow, 8 points of room
follow the last line so it scrolls clear of the edge. The ground is `kit::overlay::ground`, the fill
the layer really has (the translucent overlay over glass, else the opaque overlay), so the fades and
row washes match the card in both. The palette opens on whole entries, its group headings counted.
A scrollbar's handle is never shorter than 32 points, so a long document's stays easy to see and
grab.

### Banner and callout

`kit::surface::Banner` is the one repository-state notice at the top of a view (a merge, rebase or
cherry-pick in progress): the tone's tint over the chrome, a seam in the tone, icon, title and body
left, and up to two `Sm` ghost actions right. Each action is a `NoticeAction`: a label, an optional
glyph and its tone (Abort's `danger` circle), and, when unavailable, disabled with the reason on
hover ("Resolve the 2 conflicts first"), so History, Changes and Requests share one layout.
`Callout` is the inline note: tint, icon, optional title, wrapped text and up to two actions under
it, their labels in line with the text (`Callout::show_actions` reports a click). `Card::compact`
gives picture-led cards (the settings theme cards) `SM` padding instead of `card_pad`.

### Document tabs

`kit::tabs::DocTabs` are the repository tabs in the title bar (spec 4.8): 30 / 24 points tall,
100 to 220 wide, padding 10 left and 6 right, 7 between icon, name, counts and close. The active
tab is the `tab` fill with its 1 pt `tab_edge` ring (and `Elevation::Tab` in Deeper); the others
are transparent until hovered (`hover`, label to `text`). A folder icon (a spinner while the
repository works), the name ellipsised, ahead/behind `↑n ↓n` in mono `accent`, and a close button
on hover and on the active tab; a middle click closes too, and the tooltip gives the path and the
switching shortcut. Tabs that need more room than they have scroll sideways (the wheel scrolls
them) and fade 16 points into the deck at an edge that hides more; the active tab scrolls into view
when it changes. A closed tab collapses to nothing over `TAB_CLOSE` while its neighbours slide in.
AccessKit: a `TabList` named "Repositories", a `Tab` per tab with its selected state, and the close
button named "Close <name>".

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

Overlay cards (glass with Translucent surfaces on) share a bounded bottom-right scroll stack, at
most 408 points wide, so five errors remain reachable even in a compact zoomed window. Newest
notices lead the stack. Titles truncate without displacing Close. Errors stay until dismissed,
offer Close and Copy, and keep raw output in a scrollable monospace Details disclosure. Known
failures lead with a plain explanation. A rejected push offers Pull for its originating repository;
account failures offer Accounts. Other toasts dismiss on a card click or after four seconds, which
run only while the pointer is off the stack and the window has focus. The stack retains five
notices; a card enters from the right and leaves to the right as it fades, and the cards above one
that left slide into its place (`TOAST_SHIFT`, instant with reduced motion).

### Command palette

Placed near the top of the window, up to 620 points wide, `radius-lg`, overlay elevation. Its
height stays inside the viewport. A search field at the top, then scrollable results: icon, command name with matched characters in `accent`, and
the shortcut right-aligned in `text-faint`. Up/Down move, Enter runs, Esc closes. Branch names
appear as "Checkout <branch>" entries. Under Appearance, besides the themes, it sets Style,
Density, Translucent surfaces (On only where it can work) and Motion, as Settings does.

Field placeholders end in an ellipsis when they ask the user to act (for example, "Search
shortcuts…"); example values and field names do not.

The toolbar stays visible for frequent repository actions and status; the palette is the faster
keyboard path and the home for less frequent actions. Neither hides essential actions available
through the other. At narrow widths, compact toolbar controls preserve room for repository data
while their labels and shortcuts remain discoverable in tooltips and the palette.

### Dialog

Overlay card, `radius-lg`, `xl` padding, 460 points wide by default, centred and no taller than
fits between the tab strip and the status bar (`kit::overlay::modal_room`, which the settings
screen may take too). Title, body (a `scroll_body` when it overflows), then right-aligned `Md`
buttons as in the approved preview: secondary (outlined) then primary (the solid `button` fill,
the same widget everywhere, with a focus ring). Tab reaches the footer's buttons in reading order,
left to right; a choice between two equal answers (crash reports) is two Secondary buttons with no
Enter default and no initial focus. Destructive confirmations use a
solid `danger` primary button (`kit::Button::danger`, its text white or black by contrast
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

Narrow Settings controls occupy a content-height row below their help. Notification dismiss
controls also use bounded rows so successive frames cannot grow the notification beyond the viewport.
