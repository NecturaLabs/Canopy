# Art

Canopy's in-app illustrations are drawn in code, not shipped as image files: `nectura-ui/src/art.rs` holds
each one as a small scene of strokes, fills and nodes and paints it with the active theme's
colours, so every theme gets art in its own palette and nothing blurs at any zoom. The app icon is
the one image file: `assets/icon-512.png`, scaled to `assets/icon-*.png` and copied to
`crates/canopy-app/assets/icon-256.png` for the window icon (`assets/README.md`).

The scenes' geometry and the icon follow the briefs below and were checked on every theme.

## Rules for every illustration

- **Original work only.** No GitHub, GitLab, Git or other trademarks, logos or mascots. Provider
  marks in the UI use the Phosphor icon font.
- **No text.** Labels are rendered by the views so they can be translated and themed.
- **Theme colours only.** A part takes a role, never a colour: `Accent`, `Accent2`, `Mid` (halfway
  between them), `Success`, `Warning`, `Danger`, `Neutral` (`text_faint`) or a vertical
  `Gradient` from `Accent` to `Accent2`. Contrast then follows the theme's own tested tokens.
- **Visual language.** Geometric, calm, precise; strokes of 1.75–2.75 units with round caps; soft
  halos used sparingly; the motif is a tree canopy built from a commit graph (nodes, curved
  branches, merges).
- **Motion.** An illustration grows in once when it appears: strokes draw from their start (each
  part has a start and a span within the animation) and nodes pop in as their branch reaches
  them. Then nothing moves, so an idle screen does not repaint; with reduce motion the finished
  picture shows at once. The welcome canopy's halos breathe slowly, because that screen already
  repaints for its drifting glow. Screenshots wait until no illustration is growing.

## The illustrations

| Function | Scene box | Shown in | Brief |
|---|---|---|---|
| `logo` | 24×24 | Tab bar (22 px), About dialog | A trunk with two branches curving outwards to round nodes, nodes at the base and top. Never animated. |
| `welcome` | 640×360 | Welcome screen, above the title | An airy, symmetric canopy of branches that split and merge, accent at the trunk to accent-2 at the tips, a few halos; the lower centre stays quiet. |
| `empty_clean` | 160×120 | Changes, working tree clean | A tidy sapling in a minimal pot with three sparkles. |
| `empty_history` | 160×120 | History, no commits yet | A seed in soil with one sprout, a node at its tip and a dashed line hinting at the trunk to come. |
| `local_only` | 160×120 | Requests without a GitHub or GitLab remote | A small commit-graph tree under a glass dome on a base. |
| `empty_error` | 160×120 | A repository that failed to load | A branch ending in a split node, two fragments drifting apart, a small spark. |

## Changing or adding one

1. Edit or add its scene in `nectura-ui/src/art.rs`: path data in absolute `M L H V C Z` syntax inside the
   scene box, a role per part, and its timing. `every_scene_has_drawable_parts_inside_its_box`
   checks the geometry.
2. Render it in a dark and a light theme and at its smallest size (`tools/screenshot.sh`, the
   `screenshot` skill) and look at the result.
3. Update the table above.

## The app icon

`assets/icon-512.png`: the logo mark, bolder (a stroke about 1/16 of the tile), on a rounded
midnight tile (corner radius about 22% of its side) with a gentle radial halo. It must read at 32 px. After changing
it, regenerate the PNGs as `assets/README.md` shows and check them at 32 and 64 px.
