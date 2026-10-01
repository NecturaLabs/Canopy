---
name: add-theme
description: Recipe for adding a colour theme to Canopy, or changing a theme's colours or adding a design token. Covers the Palette constant and ThemeId variant in crates/canopy-app/src/ui/theme.rs, every token, the WCAG contrast test, dark and light screenshots of each view, and the docs/design.md section. Use whenever a request adds, renames, retunes or removes a theme, or adds a field to Palette.
license: Proprietary (NecturaLabs); see LICENSE
---

# Add a theme

`crates/canopy-app/src/ui/theme.rs` is the only place a colour is defined; views ask `palette()`.
A theme is one `Palette` constant plus one `ThemeId` variant. The theme cards in Settings, the
toolbar menu, the "Theme: X" palette entries, `--theme NAME` and the cycling in `ThemeId::next` all
iterate `ThemeId::ALL`, so none of them is edited. Work through the steps in order.

## 1. Decide

- A name (one word, shown in Settings), dark or light, and the idea in a sentence. Palettes in the
  file to compare against: Midnight and Daylight (the defaults), Aurora, Ember, Neon, Forest,
  Paper. A new one should differ in character, not by a few shades.
- `dark: true` makes egui start from its dark visuals, `false` from its light ones; the field must
  match the surfaces. The default theme is `ThemeId::Midnight` and stays so.

## 2. The `Palette` constant

Add `pub const NAME: Palette = Palette { ... };` next to the others, with every field:

| Group | Fields |
|---|---|
| Surfaces | `bg`, `panel`, `elevated`, `hover` |
| Lines | `border` (decorative hairline), `control_border` (outline of a text field, checkbox or switch track, 3:1 on every surface) |
| Text | `text`, `text_dim`, `text_faint` |
| Colour with a job | `accent`, `accent2`, `success`, `warning`, `danger`, `added_bg`, `removed_bg` |
| Primary button | `button`, `button2` (a gradient), `on_button` (its label) |
| Graph | `lanes`: ten colours, in order (`DARK_LANES` and `LIGHT_LANES` are shared by the default pair; a distinctive theme gets its own) |
| Code | `syntax`: `keyword`, `string`, `comment`, `number`, `function`, `type_`, `constant` |
| Mode | `name`, `dark` |

Write colours with `hex(0xRRGGBB)`. Hex literals are allowed here and nowhere else (not in
`views/`, `ui/widgets.rs`, `app/`). Selection, hover and focus colours are derived from these in
`visuals()` and `Palette::selected()`; do not add per-theme special cases there.

## 3. Register it

- Add the variant to `enum ThemeId`, add it to `ThemeId::ALL` (the array length is part of the
  type: update `[ThemeId; N]`), and add the arm in `ThemeId::palette`.
- Never rename or remove an existing variant: settings store the variant name (`Settings.theme`,
  serde), and an unknown name makes the whole settings file fail to load, so every setting falls
  back to its default. To retire or rename one, keep the old name deserialising (a
  `#[serde(alias = "OldName")]` on the variant that replaces it); `Settings::migrate` shows how
  the pre-0.2 `dark` flag was mapped to Daylight.
- Update the module doc ("Canopy ships seven themes") if the count is written there.

## 4. Contrast

`cargo test -p canopy-app theme` runs `every_theme_meets_contrast_targets`, which loops over
`ThemeId::ALL` and fails with one line per violation (theme, token, surface, ratio):

- `text` 7:1 on `bg`, `panel`, `elevated`, `hover`, the selected row (over `panel` and `bg`),
  `added_bg` and `removed_bg`; `text_dim` and `text_faint` 4.5:1 on the same;
- `accent`, `accent2`, `success`, `warning`, `danger` 4.5:1 and `control_border` and every lane 3:1
  on the four plain surfaces; `on_button` 4.5:1 on `button` and `button2`;
- every `syntax` colour 4.5:1 on `bg`, `added_bg` and `removed_bg`.

Fix the colours, never the test or its thresholds (AGENTS.md: do not weaken a test to pass). Also
run `cargo test -p canopy-app` (the `next_cycles_through_all_themes` test and the settings tests).
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
`canopy-visual-qa` agent. Tick nothing in `docs/backlog.md` until these images exist.

## 6. Docs

- `docs/design.md`: add a section `### Name (dark|light)` after the existing theme sections, with
  the same table as Midnight: one row per token (`bg`, `panel`, `elevated`, `hover`, `border`,
  `control-border`, `text`, `text-dim`, `text-faint`, `accent`, `accent-2`, `success`, `warning`,
  `danger`, `added-bg`, `removed-bg`) with its hex and use. Keep the hex values equal to
  `theme.rs`, and fix any sentence that counts the themes.
- `CHANGELOG.md` under `[Unreleased]`, and `docs/features.md` / `README.md` where they list the
  themes by name.

## Done when

`cargo test -p canopy-app` passes with the new theme in `ThemeId::ALL`; the screenshots above were
read in the new theme and in the opposite-mode default; `docs/design.md` has the token table;
`tools/check.sh` passes; and the diff has had the `canopy-review` agent's review.
