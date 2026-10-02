# Vendored crates

Third-party code built from this folder instead of crates.io, each with a patch of ours. The
workspace excludes `vendor/` (`Cargo.toml`), so it is compiled as a dependency and never linted,
formatted or tested as Canopy's own code. Change it only through `vendor/<crate>.patch` and the
script below, never by editing the copy.

## eframe 0.36.2

`vendor/eframe` is the published `eframe` 0.36.2 crate (manifest, sources and the embedded icon;
licence `MIT OR Apache-2.0`) plus `vendor/eframe.patch`, and `[patch.crates-io]` in `Cargo.toml`
points the build at it. The rest of the egui family stays on crates.io.

**Why.** eframe's glow backend never calls winit's `Window::pre_present_notify` (its wgpu backend
does). On Wayland that call is what makes winit hold back `RedrawRequested` until the compositor
sends a frame callback. Without it, a window the compositor is not drawing, such as one on another
workspace, is repainted anyway; Mesa's vsync wait inside `eglSwapBuffers` then never ends, the UI
thread stops dispatching Wayland events, and Hyprland shows "Application Not Responding" for
Canopy even though nothing crashed. A file change in the open repository is enough to trigger it.
[ADR 0007](../docs/adr/0007-vendored-eframe-with-frame-callback-patch.md) has the evidence and the
alternatives.

**The patch.** One call to `window.pre_present_notify()` immediately before the buffer swap of the
main paint path in `vendor/eframe/src/native/glow_integration.rs`, with a comment. Nothing else
differs from the published crate. It does not cover immediate viewports (`show_viewport_immediate`),
which paint inside their parent's frame and not on `RedrawRequested`; Canopy opens none.

**Verify.** `tools/vendor-eframe.sh --check` downloads the crate, checks its SHA-256, applies the
patch and diffs the result against `vendor/eframe`. `tools/stall-check.py` is the behaviour test:
it hides Canopy's window on a stand-in compositor and fails when the app stops answering pings.
`tools/check.sh` runs both, and the second also fails when the build does not use `vendor/eframe`
(after an eframe upgrade cargo only warns that an unused patch was ignored).

**Advisories.** cargo-deny may not match RustSec advisories to a path crate, so watch eframe's
releases and advisories by hand while this copy exists.

**Upgrade eframe.** Bump `VERSION` and `SHA256` in `tools/vendor-eframe.sh`, regenerate
`vendor/eframe.patch` against the new crate (the call usually applies unchanged), run
`tools/vendor-eframe.sh`, then `cargo update -p eframe`, `python3 tools/notices.py`,
`cargo deny check` and `tools/check.sh`. Check first whether the new release calls
`pre_present_notify` in `glow_integration.rs` itself; if it does, delete `vendor/`, the `exclude`
and `[patch.crates-io]` entries in `Cargo.toml`, `tools/vendor-eframe.sh` and this file, and keep
`tools/stall-check.py`, which then guards the upstream behaviour.
