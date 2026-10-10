# ADR 0001: How Canopy reads and changes repositories

**Status:** accepted
**Date:** 2026-10-01

## Context and problem statement

Canopy has to read repository state fast enough to draw a commit graph, status and diffs for large
histories every time something changes, and it has to change repositories (stage, commit, branch,
merge, rebase, fetch, push) in a way users trust. The two available engines behave differently.
libgit2 (through the `git2` crate) gives structured, in-process access with no output parsing.
The `git` executable is what users run in their terminal, with their hooks, signing, attribute
filters, credential helpers, SSH configuration and `includeIf` rules, which libgit2 reimplements
only partially and with different defaults (rename detection, no `rerere`, sparse checkout
differences).

Some project documents described the line between the two differently: that libgit2 also performs
"local writes" (index, refs, working tree). The code does not: every mutation in
`crates/canopy-core/src/ops.rs` runs the `git` executable. This record fixes the rule so the
documents, the agent prompts and the code agree.

## Decision drivers

- A change made in Canopy must leave the repository exactly as the same change made in a terminal
  would, hooks and signing included.
- Reads must be fast and structured: they run on every refresh and feed the graph layout.
- No OpenSSL or libssh2 linked into the binary on any platform.
- Errors shown to the user should be git's own message.

## Considered options

1. libgit2 for everything, including network (libgit2 built with HTTPS and SSH).
2. The `git` executable for everything, parsing porcelain output for reads.
3. libgit2 for reads and local writes (index, refs, working tree); the `git` executable for
   network operations, merge, rebase and history rewriting.
4. libgit2 for reads only; the `git` executable for every mutation and all network traffic.
5. gitoxide (`gix`) instead of libgit2 for reads.

## Decision outcome

Chosen option: "4. libgit2 for reads only; the `git` executable for every mutation", because it
is the only option under which every change Canopy makes runs the user's own hooks, signing,
filters and credential setup, while reads stay in-process and structured. Option 3 still lets a
local write skip a `pre-commit` hook, a `.gitattributes` filter or commit signing depending on
which call path an operation happened to take, which is the inconsistency users would notice
first. Options 1 and 2 give up one of the two drivers outright. Option 5 is not rejected on
merit; it is deferred to GitHub issue #102 (gitoxide for history traversal), which evaluates it with
measurements and would replace only the read side.

Concretely:

- `ops.rs` holds every mutation and calls `cli::run`, `cli::run_with_input` (patches and messages
  on stdin) or `cli::run_streaming` (long network jobs, `--progress` lines). The conflict
  resolver (`conflict.rs`) is the one other place: it stages and checks out sides through
  `cli::run`, and saving a hand-edited resolution is a plain atomic write of that working-tree
  file (`write_resolution`, guarded by a fingerprint so it never overwrites a newer edit).
- `cli.rs` builds every `git` command through `cli::command`, which sets `GIT_TERMINAL_PROMPT=0`,
  `GIT_EDITOR=true` and `GIT_MERGE_AUTOEDIT=no` and keeps git's stderr in the error.
- `git2` is a workspace dependency with `default-features = false`, so no HTTPS or SSH transport
  exists; `deny.toml` bans `openssl-sys` and `libssh2-sys`. It is used to read (status, revwalk,
  diff, refs, config) and to look up state an operation needs, never to write.

### Consequences

- **Good:** behaviour matches the terminal, so the `git` CLI is the test oracle for every
  operation; no TLS or SSH library is linked for Git traffic; git's error text reaches the user
  unchanged.
- **Good:** one rule to review against: a libgit2 write or network call in a diff is a defect.
- **Bad:** every mutation spawns a process, and Canopy depends on a `git` executable of a minimum
  version on `PATH` at runtime.
- **Bad:** with `GIT_TERMINAL_PROMPT=0` a remote that needs credentials fails unless a prompt
  program is registered; Canopy registers its own (ADR 0005).
- **Bad:** libgit2's view can lag a write made by the CLI, so the worker takes a fresh snapshot
  after every job that mutated the repository instead of patching its copy.

## Related

- `docs/architecture.md`, "Why libgit2 and the git CLI": the summary this record backs.
- ADR 0005: credential prompts for the `git` processes this rule creates.
- GitHub issue #102, "Evaluate gitoxide for history traversal".
- `.claude/skills/add-git-operation/SKILL.md` and `.claude/agents/canopy-git.md`: state the same
  layer rule.
