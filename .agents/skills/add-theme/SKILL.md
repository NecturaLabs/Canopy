---
name: add-theme
description: Recipe for adding a colour theme to Canopy, or changing a theme's colours or adding a design token. Covers the Palette constant and ThemeId variant in crates/nectura-ui/src/theme.rs, every token, the WCAG contrast test, dark and light screenshots of each view, and the docs/design.md section. Use whenever a request adds, renames, retunes or removes a theme, or adds a field to Palette.
license: MIT OR Apache-2.0 (Canopy); see LICENSE-MIT and LICENSE-APACHE
---

# Add a theme

`crates/nectura-ui/src/theme.rs` is the only place a colour is defined; views ask `palette()`.
A theme is one `Palette` constant plus one `ThemeId` variant. The theme cards in Settings, the
toolbar menu, the "Theme: X" palette entries, `--theme NAME` and the cycling in `ThemeId::next` all
iterate `ThemeId::ALL`, so none of them is edited. `ThemeId::Custom` is the user's own colours
(`nectura-ui/src/custom_theme.rs`) and is not a built-in theme: it is in `ALL` but not in `BUILT_IN`. Work through the steps in order.

## 1. Decide

- A name (one word, shown in Settings), dark or light, and the idea in a sentence. Palettes in the
  file to compare against: Midnight and Daylight (the defaults) and Observatory. A new one
  should differ in character, not by a few shades.
- `dark: true` makes egui start from its dark visuals, `false` from its light ones; the field must
  match the surfaces. The default theme is `ThemeId::Midnight` and stays so.

## 2. The `Palette` constant

Add `pub const NAME: Palette = Palette { ... };` next to the others, with every field (copy
`MIDNIGHT` or `DAYLIGHT` and retune; the doc comment on each field states its contrast target):

| Group | Fields |
|---|---|
| Identity | `name` (one word, shown in Settings), `concept` (a sentence, shown on the theme card), `dark`, `strict` (`false`; only high contrast sets it) |
| Surfaces | `standard` and `deeper`, each a `Surfaces`: the grounds (`bg`, `panel`, `pane`, `elevated`, `overlay`), control grounds (`field`, `track`, `switch_off`, `badge`), edges (`border`, `edge`, `raised_edge`, `overlay_edge`, `tab_edge`, `track_edge`, `pane_edge`, `keycap_edge`) and the `Fill` gradients (`raised*`, `primary*`, `thumb`, `keycap`, `tab`, `tooltip`, `knob`, with `tooltip_key*` and `on_tooltip_key`); read the struct for the full list |
| Text | `text`, `text_dim`, `text_faint`, `disabled_text`, `on_tooltip`, `on_tooltip_dim` |
| Accent | `accent` (text-grade), `accent2` (links), `focus`, `selection` with `selection_alpha`, `on_accent` (the label on `primary*`) |
| Status | `success`, `warning`, `danger`, `danger_base`, `on_danger`, and `tones` (`Tones`: the bases of the `accent`, `success`, `warning` and `danger` washes) |
| Outlines and overlays | `control_border`, `hover_ink` (base of hover, pressed and zebra overlays), `scrim` |
| Shadows | `shadow`, `primary_shadow`, `danger_shadow` |
| Glass | `glass_alpha`, `glass_luminance` |
| Graph | `lanes`: ten colours, in order, written in each palette (no shared constant) |
| Code | `syntax`: `Syntax` with `keyword`, `string`, `comment`, `number`, `function`, `type_`, `constant` |

A theme's structure is a separate `Design` record in `mod design` (radius scale, edges, shadows,
panes, selection, focus, hover, tabs, labels, heading face, graph, icons, art, motion, glass
tint). Midnight and Daylight share `CONCEPT_ONE`; add `NAME_DESIGN` (reuse an existing one, or
write a `Design` of its own) and select it in `ThemeId::design`.

Write colours with `hex(0xRRGGBB)`. Hex literals are allowed here and nowhere else (not in
`views/`, `ui/widgets.rs`, `app/`). Selection, hover and focus colours are derived from these in
`visuals()` and `Palette::selected()`; do not add per-theme special cases there.

## 3. Register it

- Add the variant to `enum ThemeId`, add it to `ThemeId::ALL` and to `ThemeId::BUILT_IN` (the array
  lengths are part of the types: update `[ThemeId; N]`), and add the arm in `ThemeId::palette` and
  in `ThemeId::design` (a theme without a design of its own takes Midnight's).
- Removing or renaming a variant needs no alias or migration (no migrations yet): settings store the
  variant name, and a saved theme this build no longer has reads as the default theme through
  `known` in `settings.rs`, without resetting the other settings.
- Update the module doc ("Canopy ships three themes") if the count is written there.
- High contrast is a switch, not a theme: `nectura-ui/src/contrast.rs` derives every theme's high-contrast
  version from its palette, keeping its grounds and hues and raising each foreground token to
  the ratio `CONTRAST_REFERENCE` (Midnight at full contrast) keeps against its ground. A new
  theme gets it for free; never hand-write a contrast variant. `HIGH_CONTRAST` is built over
  `ThemeId::BUILT_IN` and indexed by position, so its array length (`[Palette; N]`) must equal
  the length of `BUILT_IN`.
- The Custom theme is built from Midnight's or Daylight's palette and design, and its high contrast
  is derived when it is built (`build_custom`). It is not in `BUILT_IN` or `HIGH_CONTRAST`; a
  change to a base palette reaches it through `CustomTheme::palette`.

## 4. Contrast

`cargo test -p canopy-app theme` runs `every_theme_meets_contrast_targets`, which loops over
`every_palette()` (each built-in theme and a sweep of custom ones, each as it is and with high
contrast on, in both styles) and fails with one line per violation (theme, token, surface,
ratio):

- Minimums (`targets`): `text` 7:1, `text_dim` and `text_faint` 4.5:1, `control_border` and `focus`
  3:1; at high contrast (`strict`) 10:1, 7:1, 5.5:1 and 4.5:1.
- Grounds (`grounds`): `pane`, `panel`, `elevated`, `overlay` and, in Deeper, `bg`, plus `field`,
  `track`, the zebra row, every hover, strong-hover and pressed state over each ground, the
  selected row and the hovered selected row, and the diff grounds (`added_bg`, `removed_bg`, the
  gutters, the selected and hovered diff lines, the warning tint). `text`, `text_dim` and
  `text_faint` meet their minimums on all of them.
- `accent`, and `control_border` and `focus` at their minimum, hold on the plain grounds and hover
  states; `accent2`, `success`, `warning` and `danger` 4.5:1 on those and the selected rows; and
  `success`, `danger` and `warning` on the diff grounds they mark; `focus` also on the diff grounds
  and the ends of `raised`.
- Every `lanes` colour 3:1 on `pane`, `panel`, `pane` under hover and the selected row.
- Every `syntax` colour 4.5:1 on `pane`, `added_bg` and `removed_bg`.
- Labels on fills, at both ends of each gradient: `on_accent` on `primary*` and `on_danger` on
  `danger_solid*` 4.5:1; `text` and `text_dim` on `raised*`, `thumb` and `tab`; `text` on `badge`;
  `on_tooltip` and `on_tooltip_dim` on `tooltip`; `on_tooltip_key` on `tooltip_key`; a dark theme's
  `knob` 3:1 on `switch_off`.
- Each status colour's 16% wash (alpha 41): `text` at its minimum and the colour itself 4.5:1 over it.

`amendment_tokens_meet_their_targets` adds `danger_base` on the grounds, the danger highlight and
tint under `text` and `danger`, Deeper's deck and pane header under every text token, and the
knob ring; `coloured_components_are_readable` requires the ink of each lane, accent, status and
`text_dim` colour (`Palette::ink_on`) to read 4.5:1 on its ref pill, avatar, badge and change fills.

Fix the colours, never the test or its thresholds (AGENTS.md: do not weaken a test to pass). Also
run `cargo test -p canopy-app` (`the_themes_are_midnight_daylight_observatory_and_custom` and the
settings tests).
A new token (a field on `Palette`) goes into every existing theme, into this test, and into
`docs/design.md`.

## 5. Look at it

Contrast numbers do not show harmony. Render the fixture and read the images (the `screenshot`
skill; never a real repository):

```sh
tools/mkfixture.sh "$S/showcase"
tools/screenshot.sh "$S/showcase" "$S/history.png"  --theme NAME
tools/screenshot.sh "$S/showcase" "$S/changes.png"  --theme NAME --view changes
tools/screenshot.sh "$S/showcase" "$S/settings.png" --theme NAME --overlay settings
tools/screenshot.sh "$S/showcase" "$S/history-small.png" --theme NAME --size 1024x640
```

`--theme` takes the name, case-insensitive. Check on every image: graph lanes distinguishable and
the HEAD ring visible; added and removed diff lines readable with syntax colours; selection, hover
and focus rings visible; `text_faint` still legible; primary button gradient and label; ref pills;
the conflict banner on the `conflict` fixture. For a dark theme also render Daylight (`--light`)
and for a light theme Midnight, to confirm nothing leaked: a view that hard-codes a colour looks
right in one and wrong in the other. Then `--overlay settings` shows the theme card (a miniature
drawn from the palette) and that the new card selects and persists. For a full pass use the
`canopy-visual-qa` agent. Do not close the related GitHub Issue until the visual and interaction checks are verified; record any remaining checks on the issue.

## 6. Docs

- `docs/design.md`: add a row to the theme table under "Colour tokens" (columns Theme, Idea,
  Ground and signature: the `bg`, `panel` and `border` hex values and the primary and focus
  colours, equal to `theme.rs`) and a sentence on what its `Design` does differently, and fix any
  sentence that counts the themes ("Three themes."). Tokens are described there by group, not
  one row each; a new `Palette` field goes into the paragraph that covers its group.
- `CHANGELOG.md` under `[Unreleased]`, and `docs/features.md` / `README.md` where they list the
  themes by name.

## Done when

`cargo test -p canopy-app` passes with the new theme in `ThemeId::ALL`; the screenshots above were
read in the new theme and in the opposite-mode default; `docs/design.md` has the token table;
`tools/check.sh` passes; and the diff has had the `canopy-review` agent's review.
