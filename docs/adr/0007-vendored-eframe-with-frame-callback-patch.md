# ADR 0007: Canopy builds a patched copy of eframe so a hidden window never blocks

**Status:** accepted
**Date:** 2026-10-03

## Context and problem statement

The owner saw Hyprland's "Application Not Responding" dialog for Canopy while the window sat
unfocused, and at other random times, with no crash. Hyprland pings the client of every window and
raises the dialog after `misc:anr_missed_pings` unanswered pings (three in the tested setup); a
client answers only from its event loop.

Evidence, reproduced without touching the owner's session on a stand-in compositor that withholds
frame callbacks from a hidden window and pings like Hyprland (`tools/stall-check.py`):

- With a hidden window, one repaint (a resize, or a change to a file in the open repository) is
  enough; the second blocks. Pings go unanswered, the third raises the dialog, and the pong
  arrives only when the window is shown again (the check shows it right after the third miss, and
  in the runs without the patch the first pong came 3005 to 3006 ms after its ping).
- The main thread's stack at that moment is `ppoll` under `wl_display_dispatch_queue` under
  `libEGL_mesa`: the wait for the previous frame callback inside `eglSwapBuffers`, which Mesa
  performs because eframe asks for a swap interval of one (`vsync`, the default).
- eframe's glow backend never calls winit's `Window::pre_present_notify`; its wgpu backend
  (`egui-wgpu`) does. That call is what makes winit hold back `RedrawRequested` until the
  compositor sends a frame callback, and eframe's own comments in `run.rs` assume it is held back.
  Without the call the window is repainted anyway, and the swap waits for a callback a compositor
  sends only for windows it draws. The upstream main branch (2026-09-29) and the latest release
  (0.36.2) both lack the call.

## Decision drivers

- A hidden window must never stop answering its compositor: no dialog unless Canopy crashed.
- Keep the frame pacing of a visible window, and the idle CPU use, as they are.
- No `unsafe` and no new third-party crate (`AGENTS.md`).
- The fix must survive an eframe upgrade, or fail a check when it does not.

## Considered options

1. Vendor eframe 0.36.2 with a one-call patch (`pre_present_notify` before the swap), plus a
   behaviour check.
2. Switch the renderer from glow to wgpu, which calls `pre_present_notify` already.
3. Turn vsync off (`SwapInterval::DontWait`) so the swap never waits.
4. Force a swap interval of zero through Mesa's environment, or pace frames with a sleep.
5. Wait for a fix upstream.

## Decision outcome

Chosen option: "1", because it changes exactly what is wrong and nothing else. Option 2 replaces
the whole rendering stack (new dependencies, Vulkan on Linux, a different software fallback for
the headless screenshots) for a one-call difference. Option 3 removes the only frame pacing the
glow path has, so every animation, spinner and scroll repaints as fast as the GPU allows, and
winit would still not hold repaints back. Option 4 needs `unsafe` or a re-exec and has the same
pacing cost. Option 5 leaves the owner's dialog in place for an unknown time.

`vendor/eframe` is the published crate plus `vendor/eframe.patch`; `Cargo.toml` points
`[patch.crates-io]` at it and excludes `vendor/` from the workspace so it is not linted, formatted
or tested as ours. `tools/vendor-eframe.sh --check`, run by `tools/check.sh` and CI, proves the
copy equals the published crate (SHA-256 pinned) plus the patch and that the workspace builds that
copy, so an eframe bump that leaves the `[patch]` entry unused fails CI. `tools/stall-check.py`,
run by `tools/check.sh` and by CI's Linux test job, hides the window on the stand-in compositor and
fails when three pings in a row go unanswered, so a patch that stops working, or a renderer change,
fails the check. The job installs the tools on its GitHub-hosted runner and the check is required.

### Consequences

- **Good:** a window on another workspace is not repainted until it is drawn again, so it neither
  blocks nor burns CPU; on return it repaints at once. The visible window is unchanged.
- **Good:** the patch is one call, the copy is verifiable, and removing both is a documented
  step once upstream calls `pre_present_notify` (`vendor/README.md`).
- **Cost:** an eframe upgrade is no longer a one-line version bump; it also means regenerating the
  patch and rerunning the vendor script. While the window is hidden `App::update` does not run, so
  worker results and prompts wait until the window is shown. That was already true while it was
  blocked.
- **Cost:** `Cargo.lock` lists eframe without a registry source, so `cargo deny` and
  `tools/notices.py` treat it as a path crate. Both still see its licence, but `cargo deny` skips
  the advisory and yanked-version checks for such a crate (cargo-deny 0.20.2), so a RustSec
  advisory against eframe has to be watched by hand (`vendor/README.md`).
- **Not covered:** immediate viewports, which Canopy does not open. `pre_present_notify` does
  nothing under X11 (XWayland included), macOS or Windows, so this changes nothing there. The check
  runs Mesa's software path, which needs no GPU; the hardware path waits on the same frame callback
  and matches the owner's dialog, but it was not run.
- The patch is a candidate to offer upstream to egui; that is the owner's call.
