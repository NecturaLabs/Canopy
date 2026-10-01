# Tools

Shell scripts for development, verification and installation. All are Bash, run from any
directory, and print usage with `-h` or when called without required arguments.

| Script | Purpose |
|---|---|
| `check.sh` | The local CI: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`. Must pass before a change is done. |
| `mkfixture.sh [--force] <dir> [kind]` | Creates a deterministic demo repository with plain `git` (fixed identities and dates, user config ignored). Kinds: `showcase` (default), `conflict`, `linear`, `big`. |
| `screenshot.sh <repo> <out.png> [--light] [--view history\|changes] [--size WxH]` | Renders Canopy on Xvfb with software OpenGL, saves one frame once the repository has loaded and prints the PNG path. |
| `install-desktop.sh [--uninstall]` | Builds a release binary and installs it with its desktop entry, AppStream metainfo and icons under `~/.local` (Linux). |

## Fixture kinds

- `showcase`: about 40 commits of a small Rust/web project. Merged (`feature/auth`,
  `feature/graph`), fast-forwarded (`fix/typo`) and open (`feature/dark-mode`) branches, a
  `release/1.0` branch, annotated tags `v0.1.0`, `v0.2.0` and `v1.0.0`, five authors, and a bare
  remote at `<dir>.remote.git` added as `origin`. `main` is two commits ahead of `origin/main` and
  `feature/dark-mode` one commit behind its upstream. It also has one stash entry and a dirty
  working tree: a modified file with two hunks, a staged change, an untracked file and a deleted
  file. This is the fixture used for screenshots.
- `conflict`: stopped mid-merge with `src/retry.rs` conflicted and `README.md` merged cleanly.
- `linear`: 100 linear commits.
- `big`: `BIG_COMMITS` (default 5000) mainline commits written with `git fast-import`, a
  three-commit `topic/<n>` branch merged back every 50 commits and a tag every 500. Use it to check
  graph layout and scrolling performance.

The same kind always produces the same commit IDs, so screenshots and bug reports are
reproducible. `--force` replaces an existing directory only when it carries the marker file
this script writes, so it cannot delete a real repository.

## Typical session

```sh
tools/mkfixture.sh /tmp/canopy-demo
tools/screenshot.sh /tmp/canopy-demo screenshots-out/history.png
tools/screenshot.sh /tmp/canopy-demo screenshots-out/changes-light.png --light --view changes
```

`screenshots-out/` is ignored by git. The images in `docs/screenshots/` are regenerated the same
way at 1440x900.

## Requirements

- `git` 2.32 or newer for `mkfixture.sh` (it relies on `GIT_CONFIG_GLOBAL`).
- `xvfb-run` and Mesa (llvmpipe) for `screenshot.sh`. Software rendering is slow and says nothing
  about real frame times; use it for layout and visuals only.
