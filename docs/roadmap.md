# Roadmap

What Canopy plans for the next year, and what it will not do. The detail lives in
[GitHub Issues](https://github.com/NecturaLabs/Canopy/issues), ordered by priority
(`priority:p0-critical` to `priority:p3-low`); this page names the themes and links them.

## Themes

- **Trust and safety**: signed updates and installers (#188), Windows code signing (#203),
  safe force pushes and other Git correctness fixes (#196, #197), and repository protections
  (#202).
- **Real-environment verification**: the live and per-platform checks of shipped features (#118
  and the issues labelled `needs-verification`), installers on real machines (#70).
- **Accessibility and keyboard use**: a screen-reader audit (#63), complete keyboard reach
  (#176), following the system's appearance and contrast (#205, #206).
- **Scale**: the 30-repository stress test (#62), incremental graph rendering (#85) and
  benchmarks (#84).
- **Workflows**: interactive rebase (#79), reflog and undo (#78), bisect (#100), image diffs
  (#93), Git LFS controls (#57).
- **Platforms and packaging**: Flatpak, AUR, .deb, .rpm and AppImage (#95), Linux arm64 and
  Intel macOS (#96), native updates on macOS and Windows (#61).

## Not planned

These are product decisions, not gaps (see [AGENTS.md](../AGENTS.md#product-rules)):

- hosting services other than GitHub and GitLab (self-hosted GitLab included);
- telemetry, analytics or any backend service of Canopy's own; crash reports stay opt-in;
- storing tokens or passwords: hosting access uses the `gh` and `glab` sign-ins;
- replacing egui with another UI toolkit;
- in-app illustrations shipped as SVG files.
