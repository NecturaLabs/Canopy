## Summary

<!-- What changes and why. Link the GitHub issues and state which acceptance criteria this PR completes. -->

## Verification

<!-- The checks you ran and the manual checks you did, with what you saw. Say what you could not
     check. "Tests pass" alone is not enough. -->

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
