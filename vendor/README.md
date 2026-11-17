# Vendored crates

Third-party code built from this folder instead of crates.io, each with a patch of ours. The
workspace excludes `vendor/` (`Cargo.toml`), so it is compiled as a dependency and never linted,
formatted or tested as Canopy's own code. Change it only through `vendor/<crate>.patch` and the
script below, never by editing the copy. `.gitattributes` marks `vendor/**` as `-text`, so Git
never converts line endings in the copy or the patch (a Windows checkout with
`core.autocrlf=true` would otherwise make the check below fail).

## eframe 0.36.2

`vendor/eframe` is the published `eframe` 0.36.2 crate (manifest, sources and the embedded icon;
licence `MIT OR Apache-2.0`) plus `vendor/eframe.patch`, and `[patch.crates-io]` in `Cargo.toml`
points the build at it. The rest of the egui family stays on crates.io. The published crate ships no
licence file, so none sits beside the copy; its licence text and copyright line are in
`docs/legal/third-party-notices.txt` (written by `tools/notices.py`), which every release ships in
`legal/`.

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

**Verify.** `tools/vendor-eframe.sh --check` downloads the crate (so it needs network access),
checks its SHA-256, applies the patch, diffs the result against `vendor/eframe` and asks
`cargo metadata` which eframe the workspace builds, failing when it is not `vendor/eframe`: after
an eframe upgrade cargo only warns that the unused `[patch]` entry was ignored. `tools/check.sh`
and CI (the "Format, docs, tooling and dependencies" job) run it, so a Dependabot bump of eframe, which
Dependabot groups with the egui crates, fails CI until the patch is regenerated.
`tools/stall-check.py` is the behaviour test: it hides Canopy's window on a stand-in compositor and
fails when the app stops answering pings, and it refuses to run unless the build uses
`vendor/eframe`. `tools/check.sh` runs it, and so does the "Test (Linux)" job in CI, which installs
what the check needs on its GitHub-hosted runner; there the check must pass.

**Advisories.** `cargo deny check` skips the advisory and yanked-version checks for a package that
has no registry source, which `vendor/eframe` is (cargo-deny 0.20.2; its licence is still checked
and listed in the notices). It will never report a RustSec advisory against this copy. While the
copy exists, read eframe's releases and its RustSec advisories by hand whenever the egui crates are
upgraded; a failing `--check` on a Dependabot bump is the prompt to do so.

**Upgrade eframe.** Bump `VERSION` and `SHA256` in `tools/vendor-eframe.sh`, regenerate
`vendor/eframe.patch` against the new crate (the call usually applies unchanged), run
`tools/vendor-eframe.sh`, then `cargo update -p eframe`, `python3 tools/notices.py`,
`cargo deny check` and `tools/check.sh`. Check first whether the new release calls
`pre_present_notify` in `glow_integration.rs` itself; if it does, delete `vendor/`, `.gitattributes`,
the `exclude` and `[patch.crates-io]` entries in `Cargo.toml`, `tools/vendor-eframe.sh`, its step in
`tools/check.sh` and in `.github/workflows/ci.yml`, and `eframe_is_vendored()` with its call in
`tools/stall-check.py`, which otherwise refuses to run. Keep the rest of `tools/stall-check.py`: it
then guards the upstream behaviour. Remove the mentions of the deleted files (`python3
tools/check-docs.py` finds paths that no longer exist) and mark ADR 0007 superseded.
