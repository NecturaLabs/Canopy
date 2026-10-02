# Architecture decision records

Short, point-in-time records of decisions that were consequential and hard to reverse: the
problem, the options, the choice and its costs. Product rules live in `AGENTS.md`; these records
hold the reasoning behind them. An accepted record is not edited when the code moves on: write a
new one that supersedes it and change the old one's status line only. The `project-docs` skill
has the template and the test for what earns one.

| ADR | Decision | Status |
|---|---|---|
| [0001](0001-git-cli-for-mutations-libgit2-for-reads.md) | libgit2 reads; the `git` executable runs every mutation and all Git network traffic | accepted |
| [0002](0002-egui-as-the-ui-toolkit.md) | egui/eframe is the UI toolkit | accepted |
| [0003](0003-network-services-and-consent.md) | The closed list of services Canopy contacts (GitHub/GitLab APIs, update manifest, opt-in Sentry crash reports) and the consent that gates them | accepted |
| [0004](0004-hosting-api-tokens.md) | Where hosting API tokens come from, how a saved one is stored and which hosts may receive it | accepted |
| [0005](0005-credential-prompts-through-canopys-askpass.md) | git and ssh ask for credentials through Canopy's own askpass | accepted |
| [0006](0006-canopy-updates-itself-in-place.md) | Canopy installs an update itself where its installer put it, and runs the installer elsewhere | accepted |
| [0007](0007-vendored-eframe-with-frame-callback-patch.md) | Canopy builds a patched copy of eframe so a window the compositor does not draw never blocks and triggers "Application Not Responding" | accepted |
