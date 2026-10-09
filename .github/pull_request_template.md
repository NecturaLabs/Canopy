## Summary

<!-- What changes and why. Link the GitHub issues and state which acceptance criteria this PR completes. -->

## Screenshots

<!-- Required for UI changes: before and after, dark theme (and light where colours change),
     from tools/screenshot.sh on a tools/mkfixture.sh fixture. Delete this section otherwise. -->

## Checks

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace` (new Git operations are tested against a temporary repository)
- [ ] Destructive actions ask for confirmation
- [ ] Docs updated where affected: `docs/features.md`, `docs/architecture.md`, `docs/design.md`,
      `CHANGELOG.md` (Unreleased)
