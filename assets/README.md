# Assets

| File | Description |
|---|---|
| `icon.svg` | Application icon, 256x256 viewBox. The master file. |
| `icon-64.png`, `icon-256.png`, `icon-512.png` | Rasterised from `icon.svg`, transparent background. |

The icon is original work owned by NecturaLabs and covered by the repository's [LICENSE](../LICENSE).
It is the in-app Canopy mark (`crates/canopy-app/src/ui/art.rs`, `logo`) on a midnight rounded
square: a trunk with two branches that curve outwards to round nodes, violet, blue and cyan, bold
enough to read at 32 px.

`crates/canopy-app/assets/icon-256.png` is a copy of `icon-256.png` embedded as the window icon.

## Regenerating the PNGs

Any SVG renderer that supports gradients works, for example:

```sh
for size in 64 256 512; do rsvg-convert -w $size -h $size icon.svg -o icon-$size.png; done
cp icon-256.png ../crates/canopy-app/assets/icon-256.png
```

The committed PNGs were rendered with `rsvg-convert`. Check the result at 32px and 64px: the
mark must stay legible at small sizes.
