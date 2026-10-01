# Art assets

In-app illustrations, loaded through egui's SVG image loader (resvg/usvg). Every file is plain
SVG 1.1 (paths, circles, ellipses, rects, linear and radial gradients, opacity), with a transparent
background, no text, no fonts, no CSS, no filters and no external references. Each one reads on both
Midnight (`#0D1016`) and Daylight (`#F7F8FA`).

The briefs, rules and replacement procedure are in [`docs/art-prompts.md`](../../../../docs/art-prompts.md).
When an asset is replaced, keep its file name and `viewBox`, and update its row below (source:
prompt number and model, date).

| File | Purpose | viewBox | Display size | Status | Licence |
|---|---|---|---|---|---|
| `logo.svg` | Logo mark (tab bar, welcome screen, About dialog); prompt 2 | `0 0 24 24` | 20–90 px square | Placeholder: original, hand-written SVG, to be replaced by generated art per `docs/art-prompts.md` | Proprietary, NecturaLabs (original work) |
| `welcome.svg` | Welcome screen backdrop, low emphasis behind the title; prompt 3 | `0 0 640 360` | about 640×360 (16:9), scaled to the welcome column width | Placeholder: original, hand-written SVG, to be replaced by generated art per `docs/art-prompts.md` | Proprietary, NecturaLabs (original work) |
| `empty-clean.svg` | Changes view, working tree clean; prompt 4 | `0 0 160 120` | 160×120 px | Placeholder: original, hand-written SVG, to be replaced by generated art per `docs/art-prompts.md` | Proprietary, NecturaLabs (original work) |
| `empty-history.svg` | History view, repository without commits; prompt 5 | `0 0 160 120` | 160×120 px | Placeholder: original, hand-written SVG, to be replaced by generated art per `docs/art-prompts.md` | Proprietary, NecturaLabs (original work) |
| `local-only.svg` | Local-only repository (sidebar remotes section, Publish dialog); prompt 6 | `0 0 160 120` | 160×120 px (down to about 80×60) | Placeholder: original, hand-written SVG, to be replaced by generated art per `docs/art-prompts.md` | Proprietary, NecturaLabs (original work) |
| `empty-error.svg` | Repository failed to load; prompt 7 | `0 0 160 120` | 160×120 px | Placeholder: original, hand-written SVG, to be replaced by generated art per `docs/art-prompts.md` | Proprietary, NecturaLabs (original work) |
