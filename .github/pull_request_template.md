<!--
One focused change per pull request; a refactor goes in its own. The title takes commit-subject
form, `type(scope): summary` (CONTRIBUTING.md, Commit messages).
-->

## Summary

<!-- What was wrong or missing, what this does about it and why. Name any decision or shortcoming
     a reviewer should know about. -->

## Issue

<!-- Closes #N when this completes the issue's acceptance criteria, Refs #N when it does part of
     it, or None: <why>. -->

## Review focus

<!-- Where to look hardest, and what is safe to skim. -->

## Verification

<!-- The checks you ran and the manual checks you did, with what you saw. "Tests pass" alone is
     not enough. -->

## Not verified

<!-- What you could not run or see, and why. None, if everything was checked. -->

## Screenshots

<!-- Required for UI changes: before and after, dark theme (and light where colours change),
     from tools/screenshot.sh on a tools/mkfixture.sh fixture. Drag the images in here so GitHub
     hosts them; never commit screenshots taken only for a pull request.
     If nothing on screen changes, one line is enough:
     No visible change: <why> -->

## Checks

- [ ] `tools/check.sh` passes (formatting, `cargo clippy --workspace --all-targets --locked -- -D
      warnings`, workspace tests and the repository's other gates)
- [ ] New Git operations are tested against a temporary repository
- [ ] Destructive actions ask for confirmation
- [ ] Docs updated where affected: `docs/features.md`, `docs/architecture.md`, `docs/design.md`,
      `CHANGELOG.md` (Unreleased)
- [ ] No credentials, tokens or personal repository paths
