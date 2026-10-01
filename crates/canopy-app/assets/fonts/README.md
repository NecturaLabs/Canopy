# Bundled fonts

| File | Font | Licence | Source |
|---|---|---|---|
| `Inter-Regular.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf` | Inter 4.1 by Rasmus Andersson | SIL Open Font License 1.1 (`OFL-Inter.txt`) | npm `inter-ui@4.1.1`, converted from WOFF2 with fontTools; private-use code points dropped from the cmap so Phosphor icons win |
| `JetBrainsMono-Regular.ttf` | JetBrains Mono by JetBrains | SIL Open Font License 1.1 (`OFL-JetBrainsMono.txt`) | npm `jetbrains-mono@1.0.6`, converted from WOFF2 with fontTools |

The OFL allows bundling with software. The fonts are embedded in the binary with `include_bytes!`
(`crates/canopy-app/src/ui/fonts.rs`). Any new font must have a row here and its licence text alongside.
