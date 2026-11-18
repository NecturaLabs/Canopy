# Documentation map

Where each kind of fact is written down, and how far to trust it. `tools/check-docs.py` (part of
`tools/check.sh`) fails when a document under `docs/` is missing from this page or a path named
in a document no longer exists.

## Where to look

| You want to know | Read |
|---|---|
| What to work on next; what is open | [GitHub Issues](https://github.com/NecturaLabs/Canopy/issues?q=is%3Aissue+is%3Aopen) ([migration record](backlog.md)) |
| Which module owns what, how data and threads flow, where a common change goes | [architecture.md](architecture.md) |
| Why something was decided the way it was | [adr/](adr/README.md) |
| What Canopy does today, what is planned, how it compares | [features.md](features.md) |
| Colours, type, spacing, motion, components | [design.md](design.md) |
| Who Canopy is for, what it is for, its positioning and principles | [../PRODUCT.md](../PRODUCT.md) |
| Which key does what | [keybindings.md](keybindings.md) (the table itself is `BINDINGS` in `crates/canopy-app/src/keymap.rs`) |
| How to run the app live, test an update end to end, release, what the checks are | [playbook.md](playbook.md) |
| Rules that hold for every task, commands, product limits | [../AGENTS.md](../AGENTS.md) |
| Setup and library notes for people writing code | [../CONTRIBUTING.md](../CONTRIBUTING.md) |
| How to do a recurring job step by step | `.claude/skills/<name>/SKILL.md`: `add-git-operation`, `add-keybinding`, `add-theme`, `hosting-api`, `release`, `screenshot`, `fixture-repo`, and the process skills `testing`, `independent-review`, `project-docs`, `memory-hygiene`, `agent-instructions`, `agent-orchestration` |
| What the scripts under `tools/` do | [../tools/README.md](../tools/README.md) |
| Installers, packaging and the update feed | [../packaging/README.md](../packaging/README.md) |
| The landing website: running it, deploying it, where its pictures and claims come from | [../site/README.md](../site/README.md) |
| What leaves the machine, and under which terms | [legal/privacy-policy.md](legal/privacy-policy.md), [legal/terms-of-service.md](legal/terms-of-service.md) |
| How to check performance with 30 repositories open | [stress-test.md](stress-test.md) |
| Which sources each area of the work is audited against, and when it last was | [playbook/practices-audit.md](playbook/practices-audit.md) |
| How the illustrations and the app icon are made | [art-prompts.md](art-prompts.md) |
| What changed in a release | [../CHANGELOG.md](../CHANGELOG.md) |

## How far to trust each kind

- **Kept true with the code, in the same change as the code** (a stale line is a defect):
  `architecture.md`, `design.md`, `features.md`, `keybindings.md`, `playbook.md`, `legal/`,
  `../tools/README.md`, `../packaging/README.md`, `../site/README.md` and `../AGENTS.md`.
  `tools/check-docs.py` checks the parts a script can: that every source module is in its crate's
  section of `architecture.md`, that every path in backticks or link exists (in every Markdown
  document except the ones below and the process skills written for any repository; `EXCLUDED` in
  the script lists them), and that the two copies of the skills match.
- **Point in time, never edited when the code moves on**: the records in `adr/`. A later record
  supersedes an earlier one.
- **The queue**: GitHub Issues holds the open tasks and their verification status. Update
  issue descriptions and close verified work there, not in `backlog.md`, which is now a static
  migration record and tracker link. See `../AGENTS.md` for issue workflow rules.
- **Research, dated, may lag the code**: [specs/ui-spec.md](specs/ui-spec.md),
  [specs/lfs-flow-spec.md](specs/lfs-flow-spec.md) and
  [specs/git-identity.md](specs/git-identity.md) (findings and design of the shipped identity
  settings). Where one disagrees with the code, the code is right: fix or delete the sentence.
- **Temporary, delete when the status line says so** (check `git log` before trusting any state in
  them): [specs/HANDOFF-canopy-2-core-safety.md](specs/HANDOFF-canopy-2-core-safety.md) and
  [specs/HANDOFF-canopy-3-core-workflows.md](specs/HANDOFF-canopy-3-core-workflows.md) (what two
  merged branches added and the app still has to wire up). The `memory-hygiene` skill tends them. A
  session never leaves a standing handoff at the repository root: each item goes where it lives.
- **Generated, do not edit**: `legal/third-party-notices.txt` (`tools/notices.py`) and the images in
  `screenshots/` (`tools/screenshot.sh`, the `screenshot` skill).

## Adding a document

List it in the table above and give it one of the kinds above. A document that only one task needs
is a spec or a handoff, with a status line that says when it goes.
