# Art prompts

Canopy's artwork will be generated with Codex / OpenAI. Until then the app uses placeholders: the
app icon in `assets/icon.svg` (rasterised to `assets/icon-*.png` and
`crates/canopy-app/assets/icon-256.png`) and vector shapes drawn in code (the logo mark in
`widgets::paint_logo`, the empty-state glyphs in the views). The plan in `HANDOFF.md` moves every
in-app illustration to an SVG file in `crates/canopy-app/assets/art/`, rendered through egui's image
loaders, so finished art replaces a file without code changes.

## Rules for every asset

- **Original work only.** No GitHub, GitLab, Git or other trademarks, logos or mascots (no
  Octocat, no tanuki, no Git diamond). Provider marks in the UI use the Phosphor icon font, which is
  how most apps handle them.
- **No text** in any image. Labels are rendered by the app so they can be translated and themed.
- **Transparent background** for everything except the app icon tile.
- **Theme-friendly.** Illustrations sit on dark and light themes. Use the palette below and keep
  contrast through shape and luminance, not only hue. The default theme is Midnight: background
  `#0D1016`, panel `#12161E`, text `#E6E9EF`, accent violet `#8B5CF6`, accent cyan `#22D3EE`,
  success `#34D399`, warning `#FBBF24`, danger `#F87171`.
- **Visual language.** Geometric, calm, precise; thin-to-medium strokes with rounded caps; soft
  glows used sparingly; the motif is a tree canopy built from a Git commit graph (nodes, curved
  branch lines, merges).
- **Delivery.** Prefer SVG (Codex can write it directly): a single file, `viewBox` as listed, no
  external references, no embedded raster images, no scripts, under 20 KB. When an image model
  produces a raster instead, deliver PNG at 4x the listed size with transparency, and add it next
  to the SVG with the same base name.

Each asset has two prompts: one for Codex writing SVG code, one for an image model. Adjust the
wording, not the constraints.

## 1. App icon

- Files: `assets/icon.svg` (master, `viewBox="0 0 256 256"`), then rasterise to
  `assets/icon-64.png`, `assets/icon-256.png`, `assets/icon-512.png` and copy the 256 px one to
  `crates/canopy-app/assets/icon-256.png` (the window icon). Must read clearly at 32 px.
- Codex: "Write a single SVG app icon, viewBox 0 0 256 256, for a Git client named Canopy. A
  rounded square tile (corner radius 56) filled with a deep midnight gradient from #1A1033 at the top
  left to #0B1220 at the bottom right. On it, a stylised tree canopy formed by a Git commit graph: a
  vertical trunk line rising from the bottom centre, two or three branch lines that curve outwards
  and upwards from it with smooth cubic curves, and five or six commit nodes as circles with a
  darker inner dot. Stroke all lines with a linear gradient from violet #8B5CF6 to cyan #22D3EE,
  stroke width about 12, round caps. Add a subtle outer glow with feGaussianBlur at low opacity.
  Geometric, balanced, symmetrical enough to read as a tree, legible at 32 pixels. No text."
- Image model: "App icon for a desktop Git client called Canopy. A rounded-square tile with a deep
  midnight-blue to indigo gradient. Centered, a minimalist glowing tree whose trunk and branches are
  drawn as smooth lines with small circular nodes, like a commit graph. Lines glow with a violet to
  cyan gradient. Flat vector style, crisp edges, soft glow, high contrast, centred composition,
  readable at small sizes. No text, no letters, no logos. 1024x1024."

## 2. Logo mark (in-app)

- File: `crates/canopy-app/assets/art/logo.svg`, `viewBox="0 0 24 24"`. Used at 20–90 px in the tab
  bar, welcome screen and About dialog, on any theme, so no tile behind it.
- Codex: "Write an SVG logo mark, viewBox 0 0 24 24, transparent background. A tree built from a
  commit graph: a vertical trunk from (12,21) to (12,5), one branch curving up-left to a node near
  (5,6.5), one curving up-right to a node near (19,5.5), and nodes at the trunk's base and top.
  Strokes 2 units wide with round caps; trunk violet #8B5CF6, left branch the midpoint #5797F2,
  right branch cyan #22D3EE. Nodes are filled circles of radius 2.6 with a dark #0D1016 centre of
  radius 1.1. No text."
- Image model: "Minimal logo mark: a small tree made of a commit graph, a straight trunk with two
  curved branches ending in circular nodes, violet to cyan gradient lines, transparent background,
  flat vector, perfectly crisp at small sizes. No text. 512x512."

## 3. Welcome illustration

- File: `crates/canopy-app/assets/art/welcome.svg`, `viewBox="0 0 640 360"`. Shown behind or above
  the welcome title, at low emphasis, so the Open / Clone / Init cards stay the focus.
- Codex: "Write an SVG illustration, viewBox 0 0 640 360, transparent background, for the welcome
  screen of a Git client. An abstract canopy of a tree seen from below, made of many thin curved
  branch lines (stroke 1.5–3) that split and merge like a commit graph, with small circular nodes at
  the junctions. Lines fade from violet #8B5CF6 at the trunk to cyan #22D3EE at the tips, with
  opacity falling off towards the edges so it blends into a dark background #0D1016. A few nodes
  have a soft glow. Calm, airy and balanced; leave the centre-bottom third lighter so text can sit
  over it. No text."
- Image model: "Wide abstract illustration: the canopy of a tree made of thin glowing lines and
  small round nodes, like a network or commit graph, branching and merging, violet at the base
  fading to cyan at the tips, fading to transparent at the edges, on a transparent background. Calm,
  elegant, minimal, vector style, lots of negative space in the lower centre. No text. 1920x1080."

## 4. Empty state: clean working tree

- File: `crates/canopy-app/assets/art/empty-clean.svg`, `viewBox="0 0 160 120"`. Shown in Changes
  when there is nothing to commit, above "Working tree clean".
- Codex: "Write an SVG spot illustration, viewBox 0 0 160 120, transparent background. A small
  neat sapling in a minimal pot, its branches drawn as two short curved commit-graph lines with
  round nodes, surrounded by three tiny sparkles. Strokes 2.5, round caps; plant in success green
  #34D399 with accent violet #8B5CF6 nodes, sparkles in cyan #22D3EE. Friendly, tidy, satisfied. No
  text."
- Image model: "Small spot illustration: a tidy sapling in a minimal pot with two curved branches
  ending in round nodes, a few small sparkles around it, green and violet accents, flat vector,
  transparent background, conveys 'all clean'. No text. 640x480."

## 5. Empty state: no commits yet

- File: `crates/canopy-app/assets/art/empty-history.svg`, `viewBox="0 0 160 120"`. Shown in History
  for a repository without commits.
- Codex: "Write an SVG spot illustration, viewBox 0 0 160 120, transparent background. A seed in
  soil with a single tiny sprout and one round node at its tip, a dashed vertical line above it
  hinting at a future trunk. Strokes 2.5, round caps; sprout violet #8B5CF6, node cyan #22D3EE, soil
  as a soft grey-blue #262D3B ellipse. Hopeful, minimal. No text."
- Image model: "Small spot illustration: a seed in a little mound of soil with one tiny sprout
  ending in a glowing round node, a faint dashed line rising above it, violet and cyan accents,
  flat vector, transparent background, conveys 'just getting started'. No text. 640x480."

## 6. Local-only repository

- File: `crates/canopy-app/assets/art/local-only.svg`, `viewBox="0 0 160 120"`. Shown where a remote
  would be (sidebar remotes section and the Publish dialog) for repositories with no remote.
- Codex: "Write an SVG spot illustration, viewBox 0 0 160 120, transparent background. A small tree
  made of commit-graph lines growing inside a simple glass dome on a base, suggesting something
  self-contained and safe on this computer. Strokes 2.5, round caps; tree violet #8B5CF6 to cyan
  #22D3EE, dome outline in #9AA3B5 at 60% opacity with one highlight arc. No clouds, no network
  symbols, no text."
- Image model: "Small spot illustration: a tiny tree made of glowing lines and round nodes inside a
  glass dome on a small base, calm and self-contained, violet and cyan accents, flat vector,
  transparent background. No text. 640x480."

## 7. Error: cannot read repository

- File: `crates/canopy-app/assets/art/empty-error.svg`, `viewBox="0 0 160 120"`. Shown when a
  repository fails to load.
- Codex: "Write an SVG spot illustration, viewBox 0 0 160 120, transparent background. A commit
  graph branch line that ends in a broken, split node with two small fragments drifting apart.
  Strokes 2.5, round caps; line in #9AA3B5, broken node in danger red #F87171, a tiny amber #FBBF24
  spark at the break. Clear but not alarming. No text."
- Image model: "Small spot illustration: a line ending in a cracked round node with two fragments
  floating away, muted grey line, soft red node, a small amber spark, flat vector, transparent
  background, gentle rather than alarming. No text. 640x480."

## Replacing an asset

1. Generate the file and check it against the rules above (no text, no trademarks, transparent
   background, `viewBox` unchanged).
2. Replace the file in place, keeping its name.
3. Render every view in Midnight and Daylight with `tools/screenshot.sh` and look at the result.
4. Update the asset's row in `crates/canopy-app/assets/art/README.md` (source: prompt number and
   model, date) and, for the icon, regenerate the PNGs.
