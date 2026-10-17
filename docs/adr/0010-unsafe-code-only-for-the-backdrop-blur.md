# ADR 0010: Where Canopy may use unsafe code

**Status:** accepted
**Date:** 2026-10-05

## Context and problem statement

Canopy forbade `unsafe` everywhere (`unsafe_code = "forbid"` for the workspace). The redesign's
Translucent surfaces option blurs what lies behind menus, dialogs, toasts and the palette. egui has
no backdrop blur; the only live one is an OpenGL paint callback through `egui_glow`, and every
`glow` call is an `unsafe fn`. `forbid` cannot be relaxed by an inner `allow`, so a blur needs a
policy change. The owner approved it in these words: "i authorize usage of unsafe as long as we
make absolute sure no harm comes from it."

## Considered options

1. **Keep `forbid`; no live blur.** Fake depth with an opaque surface, or blur a one-off screenshot
   on the CPU when a modal opens (one frame late, stale when the content behind it moves).
2. **Put the OpenGL code in a vendored crate** so the workspace lint never sees it. Hides the
   unsafe code from Canopy's own checks instead of confining it.
3. **Deny instead of forbid, and allow unsafe code in exactly one module** of the app, with
   machine-checked rules for that module and a test that fails on unsafe code anywhere else.

## Decision outcome

Chosen option: 3, because it delivers the live blur the owner chose while keeping every line of
unsafe code in one reviewed file under Canopy's own lints.

- The workspace sets `unsafe_code = "deny"` and the clippy lints `undocumented_unsafe_blocks` and
  `multiple_unsafe_ops_per_block` (deny) and `unnecessary_safety_comment` (warn).
- `crates/nectura-ui/src/kit/glass/gl.rs` alone begins with
  `#![allow(unsafe_code, reason = "OpenGL calls for the backdrop blur; owner-approved 2026-10-05")]`.
  Each OpenGL call is one `unsafe` block with a `// SAFETY:` comment.
- `crates/canopy-app/tests/unsafe_policy.rs` (run by `tools/check.sh` and CI's test jobs) fails
  when `unsafe`, or any attribute naming `unsafe_code`, appears in another Rust file under
  `crates/`, when that file loses its allow, or when the workspace or a crate stops denying the
  lint. It ignores comments and string literals, so messages such as "unsafe path" are not code.
- The module's rules: OpenGL calls only on the context egui_glow paints with (or eframe's at exit),
  objects tied to that context and deleted once, no client memory handed to the driver beyond
  fixed-size state queries, sizes validated before use, state saved and restored, panics caught
  before they reach egui, and any failure turns the blur off for the session with layers falling
  back to opaque. The module documentation in `gl.rs` states them where the code is.

### Consequences

- **Good:** a live blur at no idle cost; the unsafe surface is one file a reviewer can read whole,
  and the compiler, clippy and a test hold the line around it.
- **Bad:** Canopy now carries unsafe code; driver bugs in the OpenGL paths it uses can still crash
  the process. The blur is off by default, Contrast never uses it, and the setting falls back to
  opaque when anything fails, which bounds who meets that code. Allowing unsafe code elsewhere
  needs a new decision that supersedes this one.

## Related

- [ADR 0007](0007-vendored-eframe-with-frame-callback-patch.md) — the vendored eframe stays
  untouched; the blur uses its public `egui_glow` callback.
