# Product

<!-- impeccable:product-schema 1 -->

## Platform

desktop — a native desktop application (Rust, `eframe`/`egui`) for Linux first, then macOS and
Windows, with one design language on every OS. None of the schema's values (web, ios, android,
adaptive) fits; it is not a web app and does not adapt its look per OS.

## Users

All developers who use Git, from people still learning it to experts who live in the terminal.
Beginners' needs (clarity, safety, explanations of what an action will do) weigh as much as power
users' (speed, keyboard control, density). Many run Linux tiling desktops such as Omarchy on
Hyprland; macOS and Windows users are served too.

## Product Purpose

Canopy is a fast, beautiful Git client: it shows a repository's history as a commit graph, stages
and commits changes (down to lines), resolves conflicts, runs merges, rebases, pulls and pushes,
and reviews GitHub and GitLab pull and merge requests. Success is that a developer does their
daily Git work in Canopy faster and with more confidence than in the CLI or another client.

## Positioning

- Native and fast (Rust), Linux-first rather than ported to Linux.
- Keyboard-first in the spirit of Omarchy and lazygit: a launcher-style command palette, a
  shortcuts sheet, vim motions and single-key actions, with a GUI's clarity.
- Private: no telemetry, analytics or backend; it contacts only `git`, the repository's own
  GitHub or GitLab API, avatar hosts, its own update feed when asked, and opt-in crash reports.
- Canopy is the authority on what it does to a repository: every action passes the option chosen
  in Canopy to git explicitly, and the UI says when that differs from the user's git
  configuration.
- A handful of carefully designed themes, and illustrations drawn in each theme's colours.

## Operating Context

Open all day beside an editor and a terminal; used in short, frequent bursts (check history,
stage, commit, pull, push) and occasional long sessions (reviewing a pull request, resolving a
conflict, an interactive rebase). Repositories range from tiny to very large. Window sizes range
from a tiled half-screen (minimum 900×560) to 4K, at interface zoom up to 160%.

## Capabilities and Constraints

- Hosting: GitHub and GitLab only (including self-hosted GitLab), plus local-only repositories.
- Git mutations run through the `git` executable; reads use libgit2. The UI thread never does
  repository I/O.
- Destructive operations (discard, hard reset, delete branch or tag, drop stash, force push) go
  through a confirmation; force pushes always use `--force-with-lease`.
- Errors show git's own message, never a generic failure.
- Every HTTP feature has an off switch and sends nothing before the user accepts the Terms and
  Privacy Policy.
- Keybindings never use Super (the window manager's), and bare keys never fire while a text field
  has focus.
- Undecided: localisation; a density setting.

## Brand Commitments

- Name: Canopy, by NecturaLabs. Tagline in use: "A fast, beautiful, Linux-first Git client".
- The mark: a trunk with two branches curving outwards to round nodes (a canopy built from a
  commit graph); the app icon is that mark on a midnight rounded square (`assets/icon-*.png`).
- Icons come from Phosphor; fonts are Inter and JetBrains Mono.
- No third-party credits inside the app.

## Evidence on Hand

- Demo repositories from `tools/mkfixture.sh` and headless screenshots from
  `tools/screenshot.sh` (every view, dialog and theme); README images in `docs/screenshots/`.
- No customers, testimonials, benchmarks or press exist yet; do not invent any.

## Product Principles

1. Speed is a feature: every interaction responds within a frame; long work shows progress.
2. Keyboard parity: everything reachable by mouse is reachable from the keyboard or the palette.
3. Never surprise the user with their repository: say what an action will do, confirm what is
   destructive, and report git's own words when something fails.
4. Calm by default, colour with a job: content (graph, diffs, state) carries the colour.
5. Private by design: nothing leaves the machine unless the user chose it.

## Accessibility & Inclusion

Contrast targets are enforced by tests for every theme (text 7:1, secondary text 4.5:1, controls
and graph lanes 3:1), including a high-contrast theme. Reduce motion is a setting. Custom-drawn
controls carry AccessKit names; a full screen-reader pass (Orca) is still open.
