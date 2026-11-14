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
| [0003](0003-network-services-and-consent.md) | The closed list of services Canopy contacts (GitHub/GitLab APIs, update manifest, opt-in Sentry crash reports) and the consent that gates them | partially superseded by 0008; amended by 0011 |
| [0004](0004-hosting-api-tokens.md) | Where hosting API tokens come from, how a saved one is stored and which hosts may receive it | accepted; source order superseded by 0013 |
| [0005](0005-credential-prompts-through-canopys-askpass.md) | git and ssh ask for credentials through Canopy's own askpass | accepted |
| [0006](0006-canopy-updates-itself-in-place.md) | Canopy installs an update itself where its installer put it, and runs the installer elsewhere | accepted |
| [0007](0007-vendored-eframe-with-frame-callback-patch.md) | Canopy builds a patched copy of eframe so a window the compositor does not draw never blocks and triggers "Application Not Responding" | accepted |
| [0008](0008-author-profile-pictures.md) | Author profile pictures on approved image hosts, after consent, cached locally for twelve hours, including authors without one | accepted |
| [0009](0009-partial-clone-downloads-only-in-trusted-folders.md) | A partial clone downloads missing objects on its own only inside a folder the user trusted, because git then runs the repository's own configuration | accepted; amended by 0014 |
| [0010](0010-unsafe-code-only-for-the-backdrop-blur.md) | Unsafe code only in the backdrop blur's OpenGL module (`nectura-ui/src/kit/glass/gl.rs`), held there by lints and a policy test | accepted |
| [0011](0011-request-list-rollup-through-graphql.md) | With a token, one GraphQL query per request-list refresh, on the same host, fetches the rows' checks and reviews; GitHub's GraphQL search finds requests not loaded yet | accepted |
| [0012](0012-git-defaults.md) | Canopy's git defaults are today's recommended practice, passed explicitly; a pull checks for divergence and conflicts before it runs | accepted |
| [0013](0013-hosting-credentials-from-the-cli-first.md) | Hosting credentials come from the signed-in `gh` / `glab` CLI first, then the environment, the saved token and git's credential helper; the CLI token stays in memory | accepted |
| [0014](0014-every-repository-is-trusted-or-read-only.md) | Every repository Canopy opens is trusted (inside a trusted folder) or read-only: its thread refuses every action, a prompt and a banner ask, folders are compared by identity, and trust safety can be turned off | accepted |
| [0015](0015-component-library-in-a-private-submodule.md) | The theme, fonts, motion, illustrations and generic widget kit live in the private repository `NecturaLabs/nectura-ui`, a submodule at `crates/nectura-ui` that CI fetches with a read-only deploy key (`SUBMODULE_DEPLOY_KEY`) | accepted |
| [0016](0016-launcher-lists.md) | Launcher lists show the installed programs, then the well-known ones that are not installed (Linux and macOS only), then Custom command…; a saved program that is gone opens the system's and says so | accepted |
