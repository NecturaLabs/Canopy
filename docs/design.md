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
  `border` seams, raised `raised` secondary buttons with a 1 pt `raised_edge`, tonal primary and
  danger buttons, and soft shadows only under floating layers. Content is painted on `pane`, which equals `bg`.
- **Deeper** is concept 4, "a floating instrument": the ground recedes and what you work on floats
  over it. The tab strip sits on the ground and the action bar is a lifted card (`deck`, with the
  deck's gradient, highlight and shadow and a `pane_edge` ring). Content panes are cards (`pane`,
  `pane_edge`) with the gutter on every side, gradient fills (`Fill { top, bottom }`), stronger
  highlights and layered shadows. History splits into two cards, the commit table and the commit
  details, at the details panel's resize handle; a hovered or dragged split shows a thin
  `focus`-tinted line. The focused card carries the `focus` gradient edge and a lit `focus` line
  along its top, over the `pane_header` strip with its `focus`-tinted seam
  (`kit::paint::pane_focus`, which the gallery's pane samples use too).

Each theme builds both styles from one surface ladder in its own neutral hue: Standard's chrome is
at least 2.5 L* off the ground and Deeper's cards at least 4 L* (`surface_steps_are_visible`), so
the steps read on any display without becoming boxes.

Interaction states are translucent overlays of `hover_ink`: `hover()`, `hover_strong()`,
`pressed()` and Dense's `zebra()`; selection is the `selected()` wash with no side bar. Status
tints (`success_tint()` and so on) are 16% washes of `Palette::tones`: in dark themes the status
colours themselves, in light themes a brighter, more chromatic hue of each, so a wash reads clean
rather than beige, khaki or lilac; text and the status colour stay readable on every wash
(`status_washes_keep_text_readable`). `danger` is the text-grade red; `danger_base` is concept 1's
`#e5484d` red for error rings, the danger button's focus ring and danger fills; `danger_solid`
fills the danger button. Light themes ring the switch knob in `knob_edge`. Disabled controls use
`Palette::disabled_text` on `Surfaces::disabled_fill` with a `disabled_edge` ring (icon buttons too), flat in both
styles and never dimmed again with `DISABLED_ALPHA` (`disabled_tokens_are_readable`: at least
3:1, 4.5:1 at high contrast). A banner's or callout's action can be disabled too, so disabled text
also keeps 3:1 on every status wash and 3.5:1 on the washes of the grounds banners sit on
(`disabled_actions_read_on_banners`; high contrast 4.5:1 throughout).

Three themes. Midnight and Daylight are round-2 concept 1, the look the owner locked: one design
in dark and in light (`CONCEPT_ONE` in `theme.rs`, `the_default_themes_are_concept_one`).
Observatory is the Midnight that build 690 shipped, kept as a theme of its own with its navy
ground, periwinkle signature, selection bar, zenith lift and constellation art
(`observatory_is_the_released_midnight`).

| Theme | Idea | Ground and signature |
|---|---|---|
| Midnight (default) | Concept 1 | neutral blue-black `#0F1012` under `#151619` chrome, `#24262B` seams, an indigo `#5E6AD2` primary and `#7C86FF` focus |
| Daylight | Concept 1 by day | white canvas `#FFFFFF` under `#F7F7F8` chrome, `#E6E6EA` seams, the same indigo primary |
| Observatory | Build 690's Midnight | navy-ink `#0B0F19` under `#111723` chrome, a periwinkle `#646CD6` primary, starlight links |

Custom is a fourth choice, not a palette in the table: Midnight or Daylight recoloured from the
user's choices (`ui/custom_theme.rs`, saved as `custom_theme`). Settings, Appearance shows a
"Custom theme" group when it is chosen: Based on (Midnight or Daylight), Accent (nine hue swatches),
Accent hue (0 to 359), Accent intensity (20 to 140%), Tint (the same swatches), Tint hue and Tint
strength (0 to 100%). The defaults are Midnight's own colours. The accent recolours the indigo
family (accent, focus, selection, the primary button and its tones) and the tint gives the grounds,
lines and text a hue. Every colour keeps its luminance; the grounds are then kept as far apart as
the base keeps them (Standard's panel 2.5 L* from its panes, Deeper's cards 4 L* from the window),
`contrast::lift` moves each foreground to at least the base theme's contrast ratios times 1.04, and
every colour drawn as text is moved until it reads on each wash it is drawn over (hover, selection,
diff rows, status tints). Status colours, graph lanes, syntax, danger and diff colours keep the
base's hues, and the structure (`Design`) is the base's. It is not in `ThemeId::BUILT_IN`; its
high-contrast version is derived when it is built, like the others.

High contrast is a switch over any theme, Custom included (Settings, Appearance; the toolbar's
theme menu; the command palette's "High contrast: On" and "Off"), not a theme. `ui/contrast.rs`
derives each theme's high-contrast colours once: the theme's grounds and hues stay, and every
colour drawn on them moves, in `OKLab` lightness, to the contrast ratio the reference keeps on
its ground (`theme::CONTRAST_REFERENCE`, Midnight at full contrast): lighter on a dark theme,
darker on a light one. A colour that already contrasts more, the graph lanes (3:1 already) and the
selection wash stay the theme's own; text on solid fills takes white or black, whichever reads.
The structure is `HIGH_CONTRAST_DESIGN` for every theme: outlines instead of shadows, an outlined
selection, a double focus ring, an underlined active tab, links underlined at rest, Atkinson
Hyperlegible titles, filled active icons, line art without glows, tighter corners, no glass. The
strict targets (text 10:1, outlines and focus 4.5:1) are tested for every theme at high contrast
(`every_palette` in the theme tests).

The history graph's node shape (dots, rings, squares, diamonds) and its lines between lanes
(curves, metro, circuit) are the user's choice in Settings (`theme::NodeShape`, `theme::Curve`,
`theme::set_graph`), the same in every theme.

Links are for going somewhere (a web page, release notes, another screen); an action that does
something (copy, dismiss, create) is a button, a Ghost one where it should stay quiet. Link
buttons are Inter Medium in `accent2` and underlined on hover and focus; where the link colour alone does
not set a link apart from body text, the theme's `Design::link_underline` underlines them at rest
too: at high contrast, for low vision. Links inside body text (the legal texts) are underlined at
rest in every theme (WCAG 1.4.1), heavier on hover and focus, each line of a wrapped link on its own
text. A web link (`LinkButton::url`) ends in an arrow, shows its address in a tooltip and offers
Copy link in its context menu (Copy address for an e-mail link). Every link is at least 24 points
tall (`theme::MIN_TARGET`), WCAG 2.5.8's 24 CSS pixels read as 24 points at 100% scale.

Every pairing (text on every surface and state, labels on every fill end, focus, outlines, lanes,
syntax, coloured components, glass over any backdrop) is a test in `theme.rs`, run for every theme
in both styles; `Palette::ink` keeps coloured text readable on its own tint.

### Lane colours

Ten colours, assigned to lines of history in order and reused cyclically. Each theme has its own
set in `theme.rs`, led by the theme's own voices and spread round the hue wheel at one balanced
lightness: any two lanes of a theme are at least CIELAB ΔE 25 apart (`graph_lanes_are_distinct`),
and each is at least 3:1 against the background, a hovered row and a selected row. Light themes
use deeper lanes. In light themes, edges are drawn at full opacity and the node fill uses the
panel colour. Midnight's set:

| Index | Hex | | Index | Hex |
|---|---|---|---|---|
| 0 | `#7C86FF` indigo | | 5 | `#EE8266` coral |
| 1 | `#4CC38A` green | | 6 | `#CB81E4` orchid |
| 2 | `#E5739F` rose | | 7 | `#43C3C3` teal |
| 3 | `#D9A54A` gold | | 8 | `#9EBF5F` leaf |
| 4 | `#4EB8E9` sky blue | | 9 | `#DDAA88` sand |

Lanes 0 to 3 are concept 1's; 4 to 9 are derived at the same lightness, with 6 and 9 moved the
smallest step that keeps every pair dE 25 apart.

Avatar backgrounds are picked from the same ten colours by a stable hash of the author's email.

## Typography

Inter for interface text, JetBrains Mono for SHAs, diffs, paths in diffs and anything that must
align. Both are bundled (SIL OFL 1.1), so metrics are consistent on Linux, macOS and Windows. The
mono face is JetBrains Mono NL, the build without programming ligatures: a version-control client
shows exact text, so `->`, `!=` or `::` draw as the characters typed and look like what copying
them gives (`fonts::tests::code_text_has_no_ligatures`). Every family ends in Canopy CJK, a
subset of Noto Sans CJK (everyday Han, kana and Hangul, 5.7 MB), so CJK file names, branches and
messages draw in a matching sans instead of missing-glyph boxes.
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
`overlay_edge` ring. Deeper adds two-layer shadows to controls, the `Pane` shadow under cards, the field recess and the deck. Shadows take each
theme's own tinted `shadow` colour (cool slate in Daylight, the ground's hue in dark themes), and Deeper's light-theme cards add a tight contact layer under a wide soft one,
so they lift off a light ground as clearly as off a dark one. High contrast has no
highlights and rings floating layers in `control_border`. The top highlight line is white at the
level's alpha, from `theme::highlight_line`.

Translucent surfaces (off by default) blur what is behind floating layers: a layer asks
`kit::glass::layer_fill` for its fill, which adds two paint callbacks and returns `overlay` at
`glass_alpha`. The capture copies and blurs what is behind the layer (dual Kawase, about 12 pt)
and stays before the layer's shadow, so the shadow never shows through the glass;
`kit::paint::surface` moves the composite between the shadow and the fill. The composite raises
saturation x1.15, keeps the backdrop's luminance in the band the contrast checks assume
(`glass_luminance`: in dark themes anything brighter is scaled down to it; in light themes every
colour is lifted that share of the way to white, so the floor holds and what lies behind keeps
its shapes), dithers and masks to the layer's rounded rectangle. A caller that places shapes
itself takes both from `kit::glass::backdrop_shapes`: the capture before its shadow, the
composite after it (`backdrop_shape` gives them as one shape, which blurs a shadow placed before
it into the glass). The command palette and menus still place the one shape after their shadow
(`kit::overlay::surface_shapes`), so their shadow darkens the glass until they take the two; the
other floating layers get glass as they move to the kit. At most four layers a frame are glass; tooltips, the settings
screen and panes stay opaque. Dark themes leave glass little room: the dimmest text on it must
keep 4.5:1, so the backdrop shows as soft light and dark shapes rather than colour. The
setting is unavailable with high contrast, when the system asks for reduced transparency
(macOS, Windows) and when the graphics driver cannot run the blur; any failure at run time turns
it off for the session and every layer is opaque again.

## Motion

Motion is feedback, never decoration. Durations, curves and springs live in `ui/motion.rs`; the
kit's helpers in `ui/kit/anim.rs`.

| Token | Duration | Use |
|---|---|---|
| (none) | 0 | Rows, selection, menu-item highlight, palette filtering, keyboard-opened menus change at once |
| `FAST` / `FAST_OUT` | 80 / 120ms | Hover in and out, press release, focus ring in |
| `PRESS_IN` / `PRESS_MIN` | 50 / 100ms | A press eases in; it stays visible at least this long after the button goes up, so a quick click (or one that starts an operation) is seen |
| `BASE` | 150ms | Chevrons, label crossfades, tooltips, field halo |
| `SLOW` | 220ms | Dialog and popover entrance |
| `TOAST_IN` | 280ms | Toast slide-in |
| `TOAST_SHIFT` | 200ms | Toasts sliding into the place of one that left |
| `TAB_CLOSE` | 250ms | A closed repository tab collapsing |
| `EXIT_POPUP` / `EXIT_DIALOG` / `EXIT_TOAST` | 100 / 120 / 160ms | Exits |
| `TOOLTIP_DELAY` / `TOOLTIP_SKIP` | 450 / 300ms | First tooltip delay (Settings' Tooltip delay: Quick 150ms, Normal, Slow 1s); no delay right after one closed |
| `LOADING_DELAY` / `LOADING_MIN` | 150 / 400ms | Spinners appear late and stay a minimum time |

Curves are cubic Béziers (`EASE_OUT` by default); springs (`SWITCH`, `PRESS`, the indicator
pair) use the exact damped solution and repaint only while moving. Buttons scale their content
to 0.97 while pressed. Settings → Appearance → Motion offers Match system (the default), Reduced
and Full; the system answer comes from `platform::prefers_reduced_motion` on a background thread.
Reduced makes every duration zero, lands every spring at once, turns the press scale off,
stops egui's scroll glide (`motion::scroll_animation`: a wheel step or a scroll-to jumps) and
stills the welcome screen. Only spinners and skeletons repaint continuously, and only while shown.

## Iconography

Phosphor icons (`egui-phosphor`), regular weight at 16px for toolbar and list icons, fill weight
for state markers (HEAD, conflict, current branch). Icons inherit `text-dim` and take `text` on
hover and `accent` when active. Toolbar buttons pair an icon with a short label; icon-only buttons
always have a tooltip naming the action and its shortcut.

Settings → Appearance → Tooltips sets which tooltips show (`overlay::TooltipLevel`). Helpful, the
default, shows what the screen does not: an icon-only control's name, why a control is
unavailable, cut-off text, shortcuts, and details such as full dates, addresses and paths. A
tooltip that only describes a labelled control (a kit button, toggle, field or chip with its name
on screen and no shortcut) or says how to use something is `Tooltip::extra` and shows only at
All. Every level, Off included, shows why a control is unavailable, also for kit buttons that
draw themselves disabled. A truncated label's tooltip shows only when
the label is actually cut.

A glyph beside text takes the text's size. A glyph drawn on its own takes one of the icon sizes
in the density's metrics:

| Token | Size (px), Comfortable / Dense | Use |
|---|---|---|
| `icon_xs` | 12 | Disclosure chevrons, close crosses on tabs and recent repositories |
| `icon_sm` | 13 | Sidebar row glyphs, tab and segmented-control icons |
| `icon_md` | 15 | Buttons, palette rows, settings navigation, request status |
| `icon_lg` | 16 / 15 | Beside a dialog's title |
| `icon_tool` | 17 / 15 | Toolbar tools |
| `icon_toast` | 18 / 15 | A notification's leading glyph |
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
- Commit details use a compact author, SHA and date above the changed files, and nothing twice:
  the author's email (and the committer, when someone else made the commit) is the name's tooltip,
  the full date with its zone the date's. The commit message's body sits flush under them, takes
  its natural height up to the room left above two changed files and the "N files changed" line
  (at least one line) and scrolls past it behind a faded edge, never cutting a line through its
  glyphs. It repeats the commit summary only when the pane header cannot show it whole (or, in a
  compact window, has no header). History prefers six visible rows and can retain the split with
  two rows in a short window. Automatic inspector height adapts when the diff arrives; a manual
  resize takes precedence.
- Commit metadata, the Changes file list, and the request list take at most 40% of their
  available width, leaving the rest for the diff or request details. The Changes file list and the
  request list are resized with `kit::split::SplitHandle` (drag, the arrow keys while it has focus,
  a double-click resets it) and keep their width across restarts. Pane limits follow the
  current viewport. Read-only diff headers share a wrapping row at short heights to keep more
  changed lines visible without reducing text size.
- Draft requests use a dim pencil icon in the list, distinct from the green open request icon;
  the full state and branch pair remain available on hover.
- The right-aligned glyphs on a request row's title line come from the list's rollup when there
  is one (texts "Checks passed", "Checks failed", "Checks running"; a neutral state such as a
  skipped pipeline shows nothing), otherwise from the request's remembered details ("All N checks
  passed", "N of M checks failed"…). "Review required" (GitHub only) is `USER_CIRCLE_DASHED` in
  `Tone::Neutral`: a to-do, not a warning.
- The request conversation ends with a primary Comment action. It stays visibly placed below the
  editor when empty, with a short explanation of why it is disabled.
- Review summaries name each reviewer's state in text as well as showing its icon and colour.
- History hides date, author and SHA columns as space decreases; full metadata remains in
  commit details and row tooltips. Row content is clipped to the table's scroll viewport.
- History search stays hidden until Ctrl+F or `/` opens it and focuses the field. The same
  shortcut, Esc or the close button hides it and clears the query. The commit count does not
  occupy the table header; an in-progress operation banner remains visible without search.
- The action toolbar uses `action_bar_height` (58 points in Comfortable density), with 46-point tools, body-size captions
  and room for its horizontal scrollbar. Tabs and toolbar actions scroll horizontally when needed; toolbar overflow has no
  scroll bar and fades out at the edge it hides behind (Scroll edges). The commit form scrolls independently above its fixed Commit button. Long status paths truncate with
  a tooltip. Diff controls share a row with the filename when space permits and wrap below it, and hunk actions use named icons in narrow
  panes.
- When there is no remote, the toolbar says "Local only" as dim status text with an explanatory
  tooltip, rather than styling it as an active control. The sidebar's Remotes section shows a small
  card instead of rows: an `elevated` surface in an `edge` hairline with a cloud-off glyph, "Local
  only", one line on what that means, and two full-width Sm buttons, a Primary Publish… over a
  Secondary Add remote….
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
  status is a non-interactive dim label above them, without a card ground. The toolbar's
  local-only status uses the tools' icon/caption alignment and density, separated by a quiet
  divider, without a badge capsule or button behaviour. Its explanation is about repository
  remotes, rather than a promise about all application network activity.
- Ignore management uses the shared form dialog, labelled fields/selects and segmented pages
  for Add rule, Ignored files and Why ignored. A preview names the destination and impact before
  Add rule is enabled. Long path lists scroll inside the dialog; the footer stays reachable.
  Stop tracking lives in the file context menu behind a danger confirmation that explains the
  staged removal and keeps the working copy. New repository offers optional starter templates.
- An unborn branch counts as a branch in the sidebar and shows its name with "no commits yet" in
  both the sidebar and status bar.
- Diff hunks: the action buttons are 26 px tall. Stage (success tint) is the one filled button;
  Discard is a ghost that turns `danger` under the pointer and sits apart on the right, and on
  very wide panes the actions stay within 960 px of the left edge. A new or deleted file is always
  one column without the empty line-number column (the layout switch shows disabled on Unified,
  its tooltip saying why), and
  its lines keep the full added/removed tint only in the gutter, with a quiet wash over the code.
  A line wider than the pane fades out at the edge. The hint "Click lines to pick part of a hunk"
  shows on a hunk header only when it fits beside the whole hunk title; otherwise the title shows
  alone, so the two never overlap.
- Conflict side previews (no line numbers) drop the indentation their lines share, and a line
  too long for the card is cut back to a word or number boundary before its ellipsis, so a number
  is never cut in two, unless that would lose more than a quarter of what fits. A file both sides
  deleted offers one "Delete it", not one per side.
- The shared operation banner (merge, rebase, cherry-pick) leads History, Changes and Requests,
  drawn the same way in all three (`views/state_banner.rs`): the kit warning `Banner` with
  Continue and Abort as ghost buttons at its right end.
  State and conflict count precede Continue; unresolved conflicts or active work disable it.
  Abort is separated and confirmed. The merge composer says Conclude merge and starts with
  Git's pending message when no draft exists.
- Changes list headers: "Stage all" and "Unstage all" are small kit buttons (`Size::Sm`); the
  Unstaged header adds "Stage selected" immediately to the right of "Stage all" (disabled unless an
  unstaged file is selected). Ctrl-click toggles and Shift-click extends a selection of several
  unstaged files from the open one (`RepoTab::marked`, the kit's `Selection` rules); Stage
  selected, Space and the row menu stage all of them, and any plain selection drops the rest. Narrow columns drop Stage selected's label first, then Stage all's.
  The diff header has no Stage file button: staging a file is that button, the row's action and
  Space. Discard
  all lives in a ⋯ menu (danger-coloured item, then a confirmation) a gap away from Stage. A zero
  count is neutral, not warning amber, and an empty section is left out while another has content
  (a conflict hides Unstaged and Staged when they are empty).
- The disabled primary button is flat with a hairline outline and faint text in every theme, never
  the primary fill. When the commit button is disabled for a reason its label does not give, a line
  above it says so ("Write a summary to commit", "Resolve the conflicts to commit").
- The commit button never cuts its label in a narrow column (a small window at a large zoom): it
  drops its keys first (they stay in its tooltip), then the branch, then the file count ("Commit
  2 files to main", "Commit 2 files", "Commit"); screen readers still hear the whole label.
- The commit composer follows the file lists while both fit the column, under a hairline, and
  pins to the bottom with the lists scrolling above it only when they would crowd it (about
  260 points of composer). Row actions (stage, unstage, mark resolved) show on the selected row
  and under the pointer on any row. Changes and commit details draw file rows the same way
  (`commit_detail::file_list_row_with`); a conflicted file's action is a labelled ghost
  "Resolve conflicts…".
- What the composer ships: its commit button is one Primary action (`kit::Button`), not the
  design spec's Commit / Commit and push `SplitButton`, and Amend last commit is a plain
  `Checkbox` with no indeterminate state. Both kit components exist; the composer does not use
  them yet.
- Conflict cards: the two "Keep this version" buttons are the card's main choices (Secondary);
  "Both: current first", "Both: incoming first", "Ancestor" and "Edit" are Ghost until chosen. A
  chosen option shows a check. The resolver footer's "Save resolved file" is the one Primary.
- Without a GitHub or GitLab remote, Requests offers Publish… (no remote at all) or Add remote…
  (remotes elsewhere) under its empty-state illustration.
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
Primary, Secondary, Ghost and Danger at Sm, Md and Lg, plus icon, tool, link and split buttons.
Primary and Danger are tonal, never solid (`Palette::tonal`): the raised control ground tinted 16%
with the accent (or `danger`), 22% hovered and 28% pressed, a hairline of it mixed 45% into
`raised_edge`, and a semibold label in that colour lifted to 4.5:1; they cast the control's shadow
like a Secondary. Disabled controls are the whole control at 42% and stay hoverable for their reason. Keyboard focus
draws a 2 pt ring 2 pt outside the control (`danger_base` on danger buttons), only after keyboard
use. `tools/screenshot.sh <repo> out.png --overlay gallery [--style deeper] [--density dense]
[--gallery-page N]` renders every component in every state; a page past the end shows the last,
and a strip under the content numbers each page from 1 ("Page 4 of 19" for `--gallery-page 3`).

Rows separate selection from being current: `ListRow::selected` is the selection wash and mark
(the sidebar row last clicked, while History shows its commit), `ListRow::current` the checked-out
branch's accent icon and semibold label, announced as current rather than selected. A selected row
that has keyboard focus also draws the focus ring.

Keyboard and screen readers (`kit::a11y`): a list is one Tab stop (`kit::row::Roving`, used by
History's commits, each sidebar section and card grids): Tab lands on the row last visited (else the
selected or checked-out one), the arrows move inside (Left and Right too in a grid), and the next
Tab leaves the list. A click puts focus on the clicked row, so the arrows and Shift+F10 continue
from it. A row's actions stay revealed while one of them has focus; a tree row's chevron
is no Tab stop. Containers are named groups holding their parts in the order shown: a list, a tree,
a menu, a radio group, a segmented control, a tab list, a select's open list, a callout, a dialog.
Linux's adapter drops two states, so there the name carries them: a disclosure's ", expanded" or
", collapsed", and a disabled control's ", unavailable" (roles it reports as read-only excepted);
a disabled control's reason is its description everywhere. Overlays (the palette, the cheatsheet,
dialogs) are modal: Tab stays inside, and closing gives focus back to what had it. Dragging a split
never takes focus; Tab does.


### Select

Select lists stay inside their containing dialog, settings card or main panel. They open above
the field when that side has more room, limit their width to the host and scroll when the options
cannot fit. Pointer presses focus the field, so arrows, Home/End, typing and Enter work after
opening with the mouse as well as from the keyboard, and the highlighted option scrolls into
view. A list that ends up under a modal layer (egui keeps each layer's rank between uses, so a
press and release in one frame can leave it there) is raised above it at once.

### Text field

`kit::field` draws every text input (single-line, search, secret, numeric and the commit composer);
no view uses egui's `TextEdit` directly. The placeholder reads at 4.5:1 and the field's bottom edge
and well at 3:1 (4.5:1 at high contrast) on every surface, tested per theme, style and contrast; the
look is cached per palette, so a high contrast or custom theme never reuses its base theme's. In a
dark theme the well is a faint lift of the surface; in a light theme it is a plate as light as the
lightest surface, so a field never reads as a disabled control. The summary's character count shows
once something is typed. The commit summary and description are one message box: two fields stacked with
square corners at the seam (`Join`), the description's top edge being the summary's bottom edge; their
placeholders are the short names ("Summary", "Description (optional)"), and the commit button says
what is missing. Focus
shows a ring for keyboard and pointer alike; the borderless search of the palette and the
cheatsheet, which has no box, turns the 2 pt line under it to the focus colour. An error is message text with an icon under the field,
announced as invalid with the message as its description, and never shows before the user typed.
The small buttons inside a field (clear, reveal, stepper) keep their look but take a 24-point hit
area (`IconButton::min_target`), and Tab reaches the clear button. A value wider than its field shows
whole in a tooltip at the Helpful level (up to 600 characters), never for a secret.

### Ref pill

Rounded rectangle (10-point radius, height 19px, horizontal padding 7px) with a Small Medium label and a
leading icon. One tonal design for every ref in its commit's lane colour (`Palette::pill`): the
`badge` surface tinted 16% with the lane colour (28% for the checked-out branch), a hairline of it
at 45%, and a white label and glyph wherever white reads at 4.5:1 on the pill over every component
surface; where it does not (the light themes), label and glyph are the lane colour lifted only as
far as 4.5:1 needs (`Palette::ink`). The kind shows only in the glyph (branch, dot for the checked-out branch, cloud
for a remote branch, tag, archive for a stash); the checked-out branch's label is semibold.
When a local branch and its upstream point to the same commit they merge into one pill showing
both icons. Pills that do not fit collapse to a `+N` badge with a tooltip.

### Swatches

`kit::choice::Swatches` is a choice between colours: a row of 22-point circles, 8 pt apart, each
with a hairline edge so a colour close to the ground keeps a shape. The chosen one shrinks a little
and is ringed in `text`; hover rings the others in `text-dim`. It takes (value, colour, name)
options, and a value none of them holds (set by a slider beside it) rings none. Like a segmented
control it is a radio group for assistive technology: each swatch is a radio button named by its
colour name, which is also its tooltip. Only the chosen swatch is a Tab stop and Left and Right
move the choice; the focus ring sits 1.5 pt outside the circle. Settings uses it for the custom
theme's Accent and Tint.

### Segmented control

`kit::choice::Segmented` (text, icons with tooltips, or both) for a choice
between a few exclusive options: settings choices, the merge dialog's merge or rebase, the clone
source and protocol, the request state filter and the diff layout. A recessed `bg` track with a
hairline `border`; the chosen segment is raised (`elevated`, restrained edge and micro-shadow),
with an accent baseline and `text`,
the others `text-dim` brightening on hover. Each segment is a radio button inside the control's
radio group for assistive technology and shows the focus ring. A segment with a count badge can
keep room for a two-digit count (`Segmented::badge_room`), so the view switcher and the toolbar
beside it do not move as the count comes and goes.

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
grab. Scroll bars are a 4-point handle in the text colour: floating ones appear at 45% while the
area is hovered; code, diff and detail boxes (`theme::solid_scroll`) keep theirs visible at 45%,
with its own 8-point lane beside the content, and every handle goes to 75% while hovered or dragged.
The commit composer's fields use the solid bars too, never egui's full-colour slab.

Two edges fade further, at full strength, because what they cut is larger than a list row: a
Settings page fades into its card over 28 points (`XL + XS`, a line of row text) where more of the
page lies beyond, so the line the card's edge cuts through never reads half drawn; and the action
bar, which scrolls sideways with no scroll bar, fades tools hidden past an edge over 48 points
(`XXL + LG`, about a tool), opaque at the edge, so a cut tool dissolves instead of reading as a
clipped word ("Mer").

### Banner and callout

`kit::surface::Banner` is the one repository-state notice at the top of a view (a merge, rebase or
cherry-pick in progress): the tone's tint over the chrome, a seam in the tone, icon, title and body
left, and up to two `Sm` ghost actions right. Each action is a `NoticeAction`: a label, an optional
glyph and its tone (Abort's `danger` circle), and, when unavailable, disabled with the reason on
hover ("Resolve the 2 conflicts first"), so History, Changes and Requests share one layout.
`Callout` is the inline note: tint, icon, optional title, wrapped text and up to two actions under
it, their labels in line with the text (`Callout::show_actions` reports a click). Screen readers
hear it as a group named for its tone (Note, Success, Warning, Error) holding the title, text and
actions once each; the icon is painted, never spoken. `Card::compact`
gives picture-led cards (the settings theme cards) `SM` padding instead of `card_pad`.

### Document tabs

`kit::tabs::DocTabs` are the repository tabs in the title bar (spec 4.8): 30 / 24 points tall, at
least 100 wide, padding 10 left and 6 right, 7 between icon, name, counts and close. The active
tab is the `tab` fill with its 1 pt `tab_edge` ring (and `Elevation::Tab` in Deeper); the others
are transparent until hovered (`hover`, label to `text`). A folder icon (a spinner while the
repository works), the name, ahead/behind `↑n ↓n` in mono `accent`, and a close button on hover
and on the active tab; a middle click closes too.

Width follows the name, in three steps as the tabs outgrow the strip:

1. Room: every tab is as wide as its whole name, counts and close button need. Nothing is
   ellipsised and no tooltip repeats a name that is fully visible.
2. Squeeze: the longest names give up width first, all down to one shared width (shorter names and
   the active tab untouched; the active tab keeps at most 60 % of the strip), no tab below 120
   points. A shortened name ends in "…" and its tooltip carries it whole. A tooltip shows only when
   it adds something: the whole name of a shortened tab, the path and switching shortcut, or the
   running job.
3. Scroll: when 120 points each is still too much, scrolling makes the room, so the names are whole
   again (a very long one stops at 280). The strip then carries its controls, only while it
   overflows, left to right: a scroll button (`CARET_LEFT`, "Scroll tabs left"), the tabs, a scroll
   button ("Scroll tabs right") and "All repositories" (`CARET_DOWN`), a menu of every open tab with
   its icon, the active one checked and a scrolling body when the list is long (the same menu
   surface as every other). The scroll buttons are small icon buttons (`Size::Sm`), disabled at the
   ends (dimmed, out of the focus order), and move the tabs 70 % of the strip with the standard
   scroll glide (a jump with reduced motion). The wheel scrolls the strip too, vertical notches
   included. The edge that hides tabs fades 16 points into the deck.

The active tab scrolls into view, past the fade, whenever it changes (switching by click, keys or
the list), tabs open or close, or the strip is resized; a tab that takes keyboard focus does too.
Widths glide over `FAST_OUT` when the squeeze changes. A closed tab collapses to nothing over
`TAB_CLOSE` while its neighbours slide in. AccessKit: a `TabList` named "Repositories", a `Tab` per
tab with its selected state, the close button named "Close <name>", and the controls named as
above.

### Illustrations

Drawn in code in the theme's colours (`ui/art.rs`, `docs/art-prompts.md`): the logo mark, the
welcome canopy and four empty-state spots. Each grows in once when it appears and then stays
still; reduce motion shows it finished. Midnight and Daylight show them as drawn over a plain
ground (`ArtMotif::Plain`); Observatory joins them into constellations (`Constellation`) and high
contrast draws bold line art without glows (`Signal`).

### Context menu

A secondary click opens a context menu at the pointer; Shift+F10 opens the focused row's below it
with its first available item focused (`kit::menu::context`); a menu opened with the pointer holds
that focus unseen, so only the hovered item is lit. In an open menu Up, Down, Home and
End move between the items and wrap, nothing behind it moves, and Escape gives focus back to the
row (the Menu key is not reported by the windowing layer). Screen readers hear a menu of its items.
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
the last picture, and an author without a picture keeps initials (never a generated pattern).

### Toast

Overlay cards (glass with Translucent surfaces on) share a bounded bottom-right scroll stack, at
most 408 points wide, so five errors remain reachable even in a compact zoomed window. Newest
notices lead the stack. Every card has one layout in every theme, padded `toast_pad` (tighter in
Dense) and shaped like the other floating layers of the style: the kind's glyph (`icon_toast`,
centred on the title's line), a semibold title that truncates, the body in `text_dim` at
`meta_size` with 1.45 leading 4 points below it, and, only when the notification asks for a
decision, a row of two buttons 10 points lower. A card has no close button: it goes on a click
(when it asks for nothing), on its buttons or on its timer. A finished operation says what happened in words (the commit's subject and id, the refs created,
updated or removed, what a pull did to the branch); git's own text is kept for failures. Known
failures lead with a plain explanation, any other error shows git's first line under its title, and git's full output is in
the notification center (its tooltip and Copy). Screen readers hear an error as an alert with its
message.

A decision (`ui::toast::Offer`) is labelled for what it does, with a tonal Primary Sm button and a
Ghost Sm one beside it: a rejected push offers Pull / Not now for its repository, refused
credentials Open Accounts / Not now, a new build Review update / Later, an installed one Restart
now / Later. Taking it closes the card and removes it from the center; declining only closes the
card, and the center offers it until it is taken or cleared.

A card lasts as long as its words take to read: two seconds to notice it plus a quarter second a
word, within 5 to 10 seconds for information and success and 8 to 15 for an error; a decision
stays 15 seconds, then waits in the center and counts on the bell. Design systems settle on these
ranges (Atlassian flags 5 to 15 s, Canva 5 s up to 10, SAP Fiori 4 to 10), and as WCAG 2.2.1 asks,
the timers run only while the pointer is off the stack, no card button has keyboard focus and the
window has focus. The stack retains five notices; a sixth pushes out the oldest routine one, then
the oldest error, and a pending decision last. A card enters from the right and leaves to the
right as it fades, and the cards above one that left slide into its place (`TOAST_SHIFT`, instant
with reduced motion).

Each notification has a topic (`ui::toast::Topic`: Errors, Needs your attention, Updates,
Finished operations, Confirmations); Settings → Notifications chooses which show as
cards, and by default only the first three do. Every notification, card or not, goes to the
notification center: the bell at the right end of the status bar (`BELL`, `BELL_SLASH` while
cards are off) opens a 360-point popover listing the last 100, newest first. Each row is laid out
like its card (the same glyph, title, body and gaps), with its relative time at the right of the
title (exact in the tooltip), up to four lines of text (all of it in the tooltip), and buttons
only for what can be done: a decision's own two as Xs (tonal Primary and Ghost), and Copy (Ghost)
on a failure, whose output is worth keeping; their labels line up with the text. Rows part by
`SM` space and a hairline at 8% of the text colour over the surface, the same in every theme. A
header holds a Ghost Clear all and the settings gear. Notifications of a counting topic (errors,
attention, updates) that showed no card, and decisions whose card ran out, raise a strong count
badge on the bell, solid `danger_solid` with `on_danger` text when an unread one is an error, else
`accent`, kept inside the status bar; opening the center clears it,
and while it is open new notifications go to its list instead of a card.

### Command palette

Placed near the top of the window, up to 620 points wide, `radius-lg`, overlay elevation. Its
height stays inside the viewport. A search field at the top, then scrollable results: icon, command name with matched characters in `accent`, and
the shortcut right-aligned in `text-faint`. Up/Down move, Enter runs, Esc closes. It is a modal
dialog named "Command palette" (the cheatsheet: "Keyboard shortcuts"): Tab stays in its search,
the results are a list box whose highlighted entry the search reports as active, and closing gives
focus back to what had it. Branch names
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
buttons as in the approved preview: secondary (outlined) then primary (the tonal accent button,
the same widget everywhere, with a focus ring). Tab reaches the footer's buttons in reading order,
left to right; a choice between two equal answers (crash reports) is two Secondary buttons with no
Enter default and no initial focus. Destructive confirmations use a
tonal `danger` button (`kit::Button::danger`, its label in `danger` lifted to 4.5:1) whose
label names the action ("Delete branch", "Discard 3 files"), never
"OK"; Cancel has the initial focus, so Enter does not confirm a destructive dialog unless the button
has focus. The button with the initial focus shows its focus ring when the dialog opens, so it is
clear what Enter presses. A choice that is riskier than the main one (trusting a whole folder) is a quiet link in
`warning` with a line saying what it covers, never a second full-width button. Informational
destinations (About: What's new, Releases, the legal texts) are text links in `accent`, with one
primary button for the real action.

The legal reader (`views/dialogs/reader.rs`) opens each document at the top every time it opens,
the first-start consent included; while it stays open, each document keeps its place as you
switch between the Terms and the Privacy Policy.

The merge dialog explains every merge mode beside its radio choice so they can be compared before
selection. The sentence below the choices still names the exact branches affected by the selected
mode.

Action dialogs lead with the effect on the repository and the choice Canopy will pass to Git.
They do not append a universal command preview: an action may run several Git commands, its
arguments can depend on repository state at execution time, and a guessed preview would imply a
guarantee it cannot keep. A future optional command disclosure must use the same redacted argv
that the operation actually executes. Differences from the user's Git configuration are shown
with the setting, value and source, as the Git settings work in the backlog requires.

The partial-clone trust dialog lists external Git folders needed by a linked worktree. Its
confirmation says "Trust these folders" and includes the displayed directories; the parent-folder
choice retains those directories when they are outside the parent. The status bar's Downloads off
chip explains visible refusals and disappears when their commit details are hidden.

Diff metadata below the header identifies symbolic-link target paths and mode changes alongside
content changes, using the theme's metadata size and subdued text colour.

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
width, using a 180-point minimum so the theme name and default/dark/light caption stay apart. Text-field placeholders are italic and faint, and fields in a section share one width.
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

Narrow Settings controls occupy a content-height row below their help. A notification's choice
buttons also use a bounded row so successive frames cannot grow the card beyond the viewport.

### Screen readers

What AccessKit reports is part of each component, checked by tests that read a frame's nodes.
A control is named by what it does, once: a card or row names itself and hides the text drawn
inside it (the welcome quick actions give their help as the description and their keys as the
keyboard shortcut). Icon glyphs are never spoken (`commit_detail::without_icons` strips the
private-use characters and hides a label that is only a glyph); counts read as words ("2 ahead,
1 behind origin/main"). A disabled button adds ", unavailable" to its name and its reason as the
description (`commit_detail::say_unavailable`), because the Linux AT-SPI adapter reports every
button as enabled. Diff lines read as "Added line 12: …", "Removed line 4: …" or "Line 7: …"
(pickable ones as selectable, with their picked state), hunk headers as "Hunk 2: @@ … @@", and
the resolver's code blocks by name (Current version, Incoming version, Ancestor, Result,
Unchanged lines 4 to 9) with their first 100 lines as the value. The window is named
"<repository> — Canopy". Keyboard focus is never out of sight: a Settings page scrolls to the
control Tab moves onto.
