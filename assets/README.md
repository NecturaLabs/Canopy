# Assets

| File | Description |
|---|---|
| `icon-512.png` | Application icon, the master file, transparent background. |
| `icon-64.png`, `icon-256.png` | Scaled down from `icon-512.png`. |

The icon is original work by NecturaLabs. Like the rest of the repository it is licensed under
[MIT](../LICENSE-MIT) or [Apache-2.0](../LICENSE-APACHE), but the licences cover the code and files,
not the Canopy name or icon as marks: a fork you distribute should use a name and icon of its own.
It is the in-app Canopy mark (`crates/nectura-ui/src/art.rs`, `logo`) on a midnight rounded
square: a trunk with two branches that curve outwards to round nodes, violet, blue and cyan, bold
enough to read at 32 px.

`crates/canopy-app/assets/icon-256.png` is a copy of `icon-256.png` embedded as the window icon.

## Regenerating the smaller PNGs

```sh
for size in 64 256; do magick icon-512.png -filter Lanczos -resize ${size}x$size icon-$size.png; done
cp icon-256.png ../crates/canopy-app/assets/icon-256.png
```

Check the result at 32px and 64px: the mark must stay legible at small sizes.
