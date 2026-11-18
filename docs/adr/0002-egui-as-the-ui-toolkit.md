# ADR 0002: Which UI toolkit Canopy's desktop app is built on

**Status:** accepted
**Date:** 2026-10-01

## Context and problem statement

Canopy is a Linux-first desktop Git client for Linux, macOS and Windows, written in Rust. Its
interface is dominated by things a stock widget set does not provide: a virtualised commit-graph
table with custom-painted lanes and curves, diff views painting only visible lines, a command
palette, toasts and several themes built from one token set. The goal was a modern,
polished, striking result and briefly considered a React front end before deciding against
it. The toolkit determines the language mix, the build and packaging story, how rendering cost
scales with repository size, and how hard theming and custom drawing are; changing it later means
rewriting every view.

## Decision drivers

- One language and one toolchain (Rust, Cargo) for the whole product.
- Custom drawing (graph, diff, pills) must be cheap to write and cheap per frame, independent of
  repository size.
- A single native binary per platform, no bundled browser runtime.
- Every colour, spacing and radius from one token file, so several themes stay consistent.
- Works on X11 and Wayland, macOS and Windows.

## Considered options

1. `egui`/`eframe` (immediate mode, glow renderer).
2. A React (web) front end in a webview shell such as Tauri, with the Rust backend behind it.
3. Electron with a web front end.
4. GTK 4 through `gtk-rs`.
5. Qt through Rust bindings.
6. Another Rust-native retained-mode toolkit (for example `iced` or Slint).

## Decision outcome

Chosen option: "1. `egui`/`eframe`", because it keeps the product in one language with no
JavaScript toolchain or browser runtime, makes custom painting (the graph, diffs, ref pills) a
direct drawing call against a frame that only lays out what is visible, and lets the whole
palette come from one Rust module (`crates/nectura-ui/src/theme.rs` (moved there by [ADR 0015](0015-component-library-in-a-private-submodule.md))) applied to egui's
visuals. The owner reviewed the egui build and chose it over React ("no need for react, what u
have there looks phenomenal"). Options 2 and 3 add a second language, an IPC boundary between UI
and backend, and a web runtime; 4 and 5 bring large C/C++ dependencies that complicate building
and shipping on macOS and Windows; 6 was not shown to offer anything egui lacks for this
interface.

### Consequences

- **Good:** rendering cost depends on what is on screen, not on repository size, provided views
  read only tab state and never touch git, the network or the disk (the worker/`Job` rule in
  `AGENTS.md`).
- **Good:** themes are data: adding one is a `Palette` constant plus a `ThemeId` variant in
  `ui/theme.rs` (the `add-theme` skill).
- **Good:** the app can render itself headlessly (`--screenshot` under Xvfb), which gives agents
  and reviewers images to check.
- **Bad:** no native platform look; menus, dialogs and widgets are Canopy's own and must be
  designed (`docs/design.md`).
- **Bad:** accessibility depends on egui's AccessKit integration (enabled in `eframe`'s features);
  an audit is tracked in GitHub issue #63 and screen-reader support is not claimed until it is done.
- **Bad:** immediate mode punishes per-frame work: no allocation of large collections, no
  repository access and no re-layout of off-screen rows per frame.
- **Bad:** egui's API changes between releases (the 0.36 `App::ui` and panel changes recorded in
  `CONTRIBUTING.md`), so upgrades are deliberate work.

## Related

- `docs/design.md`: the design system implemented on top of egui.
- `docs/architecture.md`: the threading model that keeps the UI thread free of Git work.
- `AGENTS.md`, Product rules, "Toolkit".
