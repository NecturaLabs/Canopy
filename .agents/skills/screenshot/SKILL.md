---
name: screenshot
description: See the running Canopy app as an image. Use whenever a change affects anything visible (views, widgets, theme, fonts, layout, graph drawing, dialogs), before ticking a backlog entry, when asked how something looks, or to attach screenshots to a pull request. Renders headlessly on Xvfb with software OpenGL from a deterministic fixture repository and views the PNG with the Read tool.
---

# Screenshot the app

Canopy can render itself once and save a PNG: `canopy --screenshot <file.png>` waits until the
repository has loaded, captures the window and exits. `tools/screenshot.sh` wraps this with a
release build, a virtual X server and software rendering. Without Xvfb it falls back to a real
window inside a running Hyprland session, on workspace `CANOPY_WORKSPACE` (default 3), which the
user sees open and close.

## Procedure

1. Pick a scratch directory (`S`): your scratchpad if you have one, otherwise
   `screenshots-out/` in the repository (ignored by git).
2. Create a fixture. Never screenshot the user's own repositories; they may contain private data.

   ```sh
   tools/mkfixture.sh "$S/showcase"            # add --force to recreate
   tools/mkfixture.sh "$S/conflict" conflict   # for conflict lists and banners
   ```

3. Render the views and themes the change touches. At minimum, for a visual change:

   ```sh
   tools/screenshot.sh "$S/showcase" "$S/history-dark.png"
   tools/screenshot.sh "$S/showcase" "$S/history-light.png" --light
   tools/screenshot.sh "$S/showcase" "$S/changes-dark.png" --view changes
   tools/screenshot.sh "$S/showcase" "$S/changes-light.png" --view changes --light
   ```

   Add `--size 1024x640` to check small windows, `--theme NAME` for any of the themes in
   `ThemeId::ALL`, `--view requests` for pull and merge requests (against `tools/mock-hosting.py`,
   see `tools/README.md`) and `--overlay NAME` for dialogs and screens (`tools/screenshot.sh -h`
   lists them). The first run builds the release binary, which takes a few minutes; later runs
   only rebuild what changed. Set `CANOPY_BIN` to reuse an existing binary.
4. Look at every image with the Read tool. For detail, crop and enlarge first:
   `convert in.png -crop 600x360+240+80 +repage -scale 200% crop.png`.
5. Compare against `docs/design.md`. For a second opinion or a full pass, use the
   `canopy-visual-qa` agent.
6. For the README images, regenerate `docs/screenshots/history.png`, `changes.png` and
   `welcome.png` at 1440x900 in the dark theme from the `showcase` fixture.

## What this can and cannot tell you

- **Can**: layout, alignment, clipping, colours and contrast, fonts, icons, graph geometry,
  empty and error states, both themes, different window sizes.
- **Cannot**: real frame times or smoothness (Mesa llvmpipe renders on the CPU, far slower than a
  GPU), animations (only one frame is captured), HiDPI scaling (Xvfb runs at scale 1), Wayland
  specifics, driver bugs, and interaction (hover, menus, dialogs need a state the app can start
  in). Say so in your report rather than guessing.

## Troubleshooting

- `screenshots need xvfb-run or a running Hyprland session`: install Xvfb (Arch
  `xorg-server-xvfb`, Debian/Ubuntu `xvfb`), or run from inside Hyprland. Ask before installing a
  package.
- Timeout after 60 s or no PNG: run the binary under `xvfb-run` by hand with
  `CANOPY_LOG=canopy=debug` and read the log; a GL context error usually means Mesa is missing.
- A black or empty image: the repository did not load; check the fixture path.

## Cleanup

Delete fixtures and images you no longer need (`rm -rf "$S/showcase" "$S/showcase.remote.git"`).
Do not commit anything from `screenshots-out/`; only deliberate updates to `docs/screenshots/`.
