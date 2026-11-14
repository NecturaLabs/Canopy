## Summary

<!-- What changes and why. Link the GitHub issues and state which acceptance criteria this PR completes. -->

## Screenshots

<!-- Required for UI changes: before and after, dark theme (and light where colours change),
     from tools/screenshot.sh on a tools/mkfixture.sh fixture. Delete this section otherwise. -->

## Checks

- [ ] `tools/check.sh` passes (formatting, `cargo clippy --workspace --all-targets --locked -- -D
      warnings`, workspace tests and the repository's other gates)
- [ ] New Git operations are tested against a temporary repository
- [ ] Destructive actions ask for confirmation
- [ ] Docs updated where affected: `docs/features.md`, `docs/architecture.md`, `docs/design.md`,
      `CHANGELOG.md` (Unreleased)
- [ ] No credentials, tokens or personal repository paths
