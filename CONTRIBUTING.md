# Contributing

Canopy is an open-source NecturaLabs project, and contributions are welcome. These notes apply to
everyone working on it, human or agent; [AGENTS.md](AGENTS.md) has the full conventions. Report
security problems privately, as [SECURITY.md](SECURITY.md) says, never in a public issue.

## Your first contribution

1. Find an issue: [`good first issue`](https://github.com/NecturaLabs/Canopy/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22)
   and [`help wanted`](https://github.com/NecturaLabs/Canopy/issues?q=is%3Aissue+is%3Aopen+label%3A%22help+wanted%22)
   mark ones that are ready to pick up. Say in the issue that you are working on it.
2. Issues are the maintainer's backlog, so only collaborators can open them. To report a bug,
   propose a feature or agree the approach for a large change, start a
   [discussion](https://github.com/NecturaLabs/Canopy/discussions) first; a pull request that
   arrives unannounced may not fit.
3. Fork the repository, clone it with `--recurse-submodules`, and make a branch.
4. Make the change with its tests, run `tools/check.sh` (see [Checks](#checks)), and commit with
   [Conventional Commits](#commit-messages) and a sign-off (`git commit -s`).
5. Open a pull request against `dev`, fill in the template and link the issue. `dev` is the default
   branch and takes all work; `main` only takes `dev` when a build is released. A pull request
   opened against `main` is moved to `dev`. Only the maintainer merges.

Questions about using Canopy go to [Discussions](https://github.com/NecturaLabs/Canopy/discussions)
([SUPPORT.md](SUPPORT.md)).

## What fits Canopy

Canopy is a Git client for GitHub, GitLab (including self-hosted GitLab) and local repositories,
built with egui. Contributions that keep to the product rules in [AGENTS.md](AGENTS.md#product-rules)
are welcome. These will be declined: support for other hosting services, telemetry or analytics,
contacting any new host without a documented reason and an off switch, storing credentials, a
different UI toolkit, and code copied from GPL-licensed Git clients or under any licence not
compatible with MIT OR Apache-2.0. [docs/roadmap.md](docs/roadmap.md) says what is planned.

## What to expect

Canopy has one maintainer, so reviews happen when there is time. A new issue or pull request
usually gets a first reply within a week. A pull request is merged once its checks pass and the
review's blocking points are resolved; one that stalls for 30 days without a reply from its author
may be closed, and can be reopened at any time. How decisions are made: [GOVERNANCE.md](GOVERNANCE.md).

## Licence of contributions

Canopy is licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT License](LICENSE-MIT), at your option. Unless you explicitly state otherwise, any contribution
you intentionally submit for inclusion in Canopy, as defined in the Apache-2.0 licence, is dual
licensed as above, without any additional terms or conditions. Please sign off each commit
(`git commit -s`) to certify the [Developer Certificate of Origin](https://developercertificate.org/):
that you wrote the change or otherwise have the right to submit it under these licences. Commits
made on the GitHub website are signed off automatically.

A pull request from a fork runs CI with a read-only token and no secrets, once a maintainer has
approved the run.

## Setup

1. Install stable Rust with [rustup](https://rustup.rs), with rustfmt and clippy. Canopy
   builds with the latest stable release and pins none; when a new clippy reports something,
   fix it in its own commit. The minimum supported Rust version is 1.95 (egui 0.36.2 needs it).
2. Install `git` 2.32 or newer (required by the fixture scripts) and, on Linux, the runtime libraries listed in the
   [README](README.md#install-from-source).
3. Fetch the component library: `crates/nectura-ui` is a submodule of the public repository
   [`NecturaLabs/nectura-ui`](https://github.com/NecturaLabs/nectura-ui)
   (`git clone --recurse-submodules`, or `git submodule update --init` in an existing clone).
   Change it in its own repository, then commit the new submodule pointer here.
4. Optional: `cargo install cargo-deny` to run the licence and advisory checks locally, and
   `xvfb` to take headless screenshots.

```sh
tools/mkfixture.sh /tmp/canopy-demo
cargo run -p canopy-app -- /tmp/canopy-demo
```

Use fixtures for anything destructive, never a repository you care about.

## Checks

`tools/check.sh` runs formatting (`cargo fmt --all --check`), Clippy
(`cargo clippy --workspace --all-targets --locked -- -D warnings`), the unsafe-code policy,
workspace tests (`cargo test --workspace --locked`), release tooling self-tests, third-party
licence notices, documentation and landing-site checks, the vendored eframe verification,
and the hidden-window stall check on Linux when its required system libraries are present.
It also tests the installer on Linux and macOS, and runs ShellCheck and `cargo deny check` when
installed. CI adds platform-specific jobs. The local checks must pass before merging; a failure documented in a [GitHub Issue](https://github.com/NecturaLabs/Canopy/issues) is reported, not hidden. Keep `TMPDIR` on
a filesystem that accepts non-UTF-8 file names (the default `/tmp` does; NTFS does not).

## Lints

The workspace `Cargo.toml` sets the lint levels for every crate: clippy's `all`, `pedantic` and
`cargo` groups, the nursery and restriction lints that fit this code, and extra rustc lints, with
the reason for each lint turned on or left out beside it. `clippy.toml` refuses the calls that
break three project rules: starting a program other than through `canopy_core::cli`, writing
with libgit2, and making a colour outside `nectura-ui/src/theme.rs`. It lists the usual calls, not every way
round a rule, so review still applies the rules themselves. When a finding is deliberate, put
`#[expect(lint, reason = "...")]` on the narrowest item or statement that covers it; `allow` is
itself a warning, and an `expect` fails once its finding is gone. Views convert between egui's
integer and float types through the helpers in `ui/mod.rs` (`points_i8`, `count_f32`,
`saturate_u8` and the rest) rather than casting by hand.

## Issues and pull requests

Use [GitHub Issues](https://github.com/NecturaLabs/Canopy/issues) for all backlog work, defects and follow-ups. Before starting, check for an existing issue and record partial progress there; close it only when its acceptance criteria are met. Do not add tasks to `docs/backlog.md`, which is a historical pointer. Use the issue forms and triage conventions below; [AGENTS.md](AGENTS.md) gives the project workflow rules.

### Issue content

Use a specific, action-oriented title without type or priority prefixes. State the problem or
deliverable, relevant files and constraints, and checkable acceptance criteria. Bugs need observed
and expected behaviour, reproduction steps and environment details. Say when facts are unknown.
When it makes verification concrete, add "Where to see it": a safe fixture, the workflow and the
exact command to run. Record each owner decision with its date, answer and alternatives, and keep
requirements that remain unresolved.

Write direct statements. Remove vague praise, canned summaries, repeated instructions and stale
status text. Keep evidence links and unresolved requirements. Progress notes should name the
commit/build, checks actually run and remaining gaps. Close compound issues only when every
criterion is verified; put independent follow-ups in linked issues. For an exact duplicate, link
the canonical issue, add `duplicate`, and close it as a duplicate. Closed duplicates may use
only `duplicate`; schedule work on the canonical issue. Related or parent/child scope is not a duplicate. Issues labelled `archived` are closed
placeholders that keep the numbers of issues and pull requests from before the source was
published; they take no other label.

### Labels and triage

[`.github/labels.json`](.github/labels.json) records label names, colours and descriptions. Keep it
and the live catalogue in sync. Label text carries the meaning; colours help scanning.

Every triaged issue except a closed duplicate has exactly one type:

| Type | Use |
|---|---|
| `bug` | Incorrect existing behaviour or a failure |
| `enhancement` | A new or expanded user-facing capability |
| `maintenance` | Internal upkeep, refactoring, tooling or process work |
| `documentation` | Documentation changes without product behaviour changes |
| `verification` | Checks of implemented behaviour, with recorded evidence |
| `owner-action` | Owner decisions, legal information or external-account tasks |

Apply one `area:*` label for the main affected area. The form's Area answer maps to these labels:

| Form area | Label |
|---|---|
| Git operations | `area:git` |
| Interface and themes | `area:ui` |
| Accessibility | `area:accessibility` |
| GitHub and GitLab | `area:hosting` |
| Settings | `area:settings` |
| Platform and installers | `area:platform` |
| Security and privacy | `area:security` |
| Performance | `area:performance` |
| Tests and quality checks | `area:tests` |
| Documentation | `area:docs` |
| Builds and releases | `area:release` |
| Planning and project workflow | `area:planning` |
| Legal | `area:legal` |
| Application lifecycle | `area:app` |

Every open issue has one priority. Maintainers review the default P2 during triage:

| Priority | Meaning |
|---|---|
| `priority:p0-critical` | Confirmed critical incident or release blocker; address immediately |
| `priority:p1-high` | High-priority work before the normal backlog |
| `priority:p2-normal` | Normal planned work; default pending triage |
| `priority:p3-low` | Deferred or low-impact work |

Apply one size label to open issues, estimating the remaining work:

| Size | Meaning |
|---|---|
| `size:small` | One bounded correction or focused verification |
| `size:medium` | Several related changes or checks within one workflow |
| `size:large` | Cross-module/platform work, a broad audit or a compound feature |
| `size:needs-estimate` | Scope or external dependencies are unclear; record what is missing |

Priority is urgency; size is remaining work, not a delivery date. Reassess size when scope changes.

Forms apply the type, P2 priority, `size:needs-estimate` and `needs-triage`; the verification
form also applies `needs-verification`. Dropdown answers do not apply labels automatically.
During triage, check for duplicates, confirm actionable scope and acceptance criteria, apply the
matching area, review priority and estimate size, then remove `needs-triage`. Resolve "Not sure"
area answers before removing the flag. API/CLI issues follow the same conventions.

`good first issue` and `help wanted` sit beside the type, area, priority and size: they mark open
work that has checkable acceptance criteria and needs no owner decision.
Use `needs-verification` when live, platform or owner acceptance checks remain, and `security`
for credential, trust, privacy or execution risks. These flags supplement the type and area;
they do not replace them. GitHub's open/closed state tracks completion. `dependencies`,
`github_actions` and `rust` are automation labels for dependency PRs, not issue types.

Blank web issues are disabled. Forms activate on the default branch; live label and issue edits
take effect immediately. See GitHub's
[issue-form syntax](https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/syntax-for-issue-forms)
and [template configuration](https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/configuring-issue-templates-for-your-repository).

Keep independently reviewable work in atomic commits. Contribute to the existing feature branch or pull request for the work; do not open a duplicate PR. A PR is one focused change (a refactor goes apart from a behaviour change), on a branch named `<type>/<topic>` (`fix/rebase-abort`), titled like a commit subject (the Pull request title check asks for it), with the template filled in: summary, issue (`Closes #N`, or `Refs #N` for part of one), review focus, verification and what was not verified. CI must pass on a branch that is up to date with `dev`; **Update branch** on the pull request brings it up to date (keep **Allow edits by maintainers** on, so the maintainer can do it for you). Wait for the owner's approval before merging.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):
`<type>(<scope>): <summary>`, summary in the imperative and at most 72 characters.

- Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `style`, `revert`
  (`revert: feat(changes): ...`, not GitHub's default `Revert "..."`).
- Scopes: a crate (`core`, `app`) or a module (`graph`, `worker`, `diff`, `history`, `theme`, ...).
- Breaking changes: `!` after the scope and a `BREAKING CHANGE:` footer.
- Security fixes: a releasable type (`fix`, `feat`, `perf`, `revert`, `build(deps)`) with the
  `security` scope or a `Security:` footer naming the advisory (`Security: GHSA-...`), so the
  release notes list them under Security. Other types never reach the notes.
- Credit: each line of the release notes ends `by @login` for its outside contributors, from the
  commit's author and `Co-authored-by:` trailers (`@login` for a GitHub no-reply address, else the
  author's name, never the address), and a New contributors section names first-time contributors
  with their pull request. Maintainers and bots are left out.

Example: `feat(changes): stage individual lines from the diff viewer`.

## Before merging

- [ ] Changes are reviewable as individual commits, based on current `dev` or the agreed PR branch.
- [ ] `tools/check.sh` passes, and all required GitHub Actions checks pass for the PR head.
- [ ] New or changed Git operations have tests against a temporary repository.
- [ ] UI changes include before and after screenshots (dark, and light where colours change),
      taken with `tools/screenshot.sh` on a fixture, or a line `No visible change: <why>`. The
      Visual evidence check asks for one when a pull request touches the UI or the site; it is a
      reminder, not a required check.
- [ ] `docs/features.md`, `docs/architecture.md`, `docs/design.md` and `CHANGELOG.md`
      (Unreleased) are updated where the change touches them.
- [ ] Destructive actions go through a confirmation dialog.
- [ ] No credentials, tokens or personal paths in code, tests or screenshots.

## Library notes

API details of the pinned versions that older examples get wrong:

- **egui 0.36**: the app implements `eframe::App::ui(&mut self, ui, frame)`; panels are
  `egui::Panel::left/right/top/bottom(id).show(ui, …)` and take a `Ui`, not a `Context`;
  `CentralPanel::default().show(ui, …)`; `ctx.global_style()` and
  `ctx.egui_wants_keyboard_input()`; `Shadow`, `Margin` and `CornerRadius` have integer fields.
- **git2 0.21** returns `Result` from many string getters (`Reference::name`, `Remote::url`;
  `StringArray` iteration yields `Result<Option<&str>>`); prefer the `*_bytes` variants.
- **Inter**: the bundled files have their private-use code points removed so Phosphor icons
  render; re-apply that when updating Inter (`crates/nectura-ui/assets/fonts/README.md`).

Update this section when one of these crates is upgraded.
