//! Hostile-repository tests for issue #28: a repository whose own files name programs starts none
//! of them while it is not trusted, and starts the ones the user's own actions run once it is.
//!
//! Each program source gets a *control*: ordinary git, run as a terminal would, starts the
//! program, so the fixture is armed and the absence of the marker means something. Then the same
//! action goes through the repository thread of an untrusted repository (refused, naming the
//! source, no marker) and of a trusted one (it goes through and git runs the program, as in a
//! terminal). Programs are small scripts that only `touch` a marker file in the test's own
//! temporary directory.
//!
//! The tests run in a child process with an empty git configuration of its own (the worker's git
//! inherits this process's environment) and build every repository in a temporary directory.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use canopy_core::Oid;

use crate::repo_config::in_isolated_git_env;
use crate::worker::{Event, Job, Op, READ_ONLY, Worker};

/// One repository in its own temporary directory, with a place for its markers.
struct Arena {
    root: tempfile::TempDir,
    repo: PathBuf,
}

impl Arena {
    /// A repository with two diverged branches (`main` and `topic`, both changing `a.txt`) and
    /// attributes that name the driver `probe` for filters, merges and diffs; no driver is
    /// configured yet, so git ignores them.
    fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let repo = root.path().join("repo");
        std::fs::create_dir(&repo).expect("repo");
        let arena = Self { root, repo };
        let git = |args: &[&str]| {
            assert!(terminal_git(&arena.repo, args).0, "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        for (key, value) in [
            ("user.name", "Ada Lovelace"),
            ("user.email", "ada@example.com"),
            ("commit.gpgsign", "false"),
            ("protocol.file.allow", "always"),
        ] {
            git(&["config", key, value]);
        }
        arena.write("a.txt", "one\ntwo\nthree\n");
        arena.write(
            ".gitattributes",
            "a.txt filter=probe merge=probe diff=probe\n",
        );
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "base"]);
        git(&["switch", "-q", "-c", "topic"]);
        arena.write("a.txt", "one\ntwo\nthree-topic\n");
        git(&["commit", "-q", "-am", "topic"]);
        git(&["switch", "-q", "main"]);
        arena.write("a.txt", "one-main\ntwo\nthree\n");
        git(&["commit", "-q", "-am", "main"]);
        arena
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.repo.join(name), text).expect("write");
    }

    fn git(&self, args: &[&str]) {
        assert!(terminal_git(&self.repo, args).0, "git {args:?}");
    }

    fn config(&self, key: &str, value: &str) {
        self.git(&["config", key, value]);
    }

    /// The marker `label`'s program leaves.
    fn marker(&self, label: &str) -> PathBuf {
        self.root.path().join(format!("ran-{label}"))
    }

    /// A program that leaves `label`'s marker, then runs `body` (a shell script).
    fn program(&self, label: &str, body: &str) -> PathBuf {
        // A path without spaces or other characters a shell would split on.
        let slug: String = label
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let path = self.root.path().join(format!("program-{slug}"));
        let text = format!(
            "#!/bin/sh\ntouch '{}'\n{body}\n",
            self.marker(label).display()
        );
        std::fs::write(&path, text).expect("script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("mode");
        path
    }

    fn hook(&self, name: &str, label: &str) {
        let hooks = self.repo.join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).expect("hooks");
        let program = self.program(label, "exit 0");
        std::fs::copy(&program, hooks.join(name)).expect("hook");
        std::fs::set_permissions(hooks.join(name), std::fs::Permissions::from_mode(0o755))
            .expect("mode");
    }

    fn ran(&self, label: &str) -> bool {
        self.marker(label).exists()
    }

    fn reset(&self, label: &str) {
        let _ = std::fs::remove_file(self.marker(label));
    }

    fn head(&self) -> Oid {
        let out = terminal_git(&self.repo, &["rev-parse", "HEAD"]).1;
        Oid::from_str(out.trim()).expect("oid")
    }
}

/// Runs `git` as a terminal would (no Canopy hardening), without ever waiting for a prompt.
/// Returns whether it succeeded and what it printed.
#[expect(
    clippy::disallowed_methods,
    reason = "the control runs ordinary git to show that the fixture's program does run"
)]
fn terminal_git(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_AUTHOR_NAME", "Ada Lovelace")
        .env("GIT_AUTHOR_EMAIL", "ada@example.com")
        .env("GIT_COMMITTER_NAME", "Ada Lovelace")
        .env("GIT_COMMITTER_EMAIL", "ada@example.com")
        .output()
        .expect("git runs");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Events of `worker` until `done` accepts one, without the snapshots the file watcher may
/// trigger; fails after a timeout.
fn events_until(worker: &Worker, done: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    loop {
        let e = worker
            .events
            .recv_timeout(Duration::from_secs(60))
            .expect("the worker answers");
        if matches!(e, Event::Snapshot(_) | Event::Status(_)) {
            continue;
        }
        let last = done(&e);
        seen.push(e);
        if last {
            return seen;
        }
    }
}

/// Sends `op` and returns the error it ended with, `None` when it succeeded.
fn run_op(worker: &Worker, op: Op) -> Option<String> {
    worker.send(Job::Op(op));
    let events = events_until(worker, |e| matches!(e, Event::OpFinished { .. }));
    match events.last() {
        Some(Event::OpFinished { result, .. }) => result.as_ref().err().cloned(),
        other => panic!("{other:?}"),
    }
}

/// Waits until the repository thread has finished everything sent before this call.
fn settle(worker: &Worker) {
    worker.send(Job::Tools);
    events_until(worker, |e| matches!(e, Event::ToolsMissing(_)));
}

/// What a job in a case reports: the refusal text of an operation that was not run, or `None`
/// when it ran (successfully or not).
type Attempt = fn(&Worker, &Arena) -> Option<String>;

/// A program source with how to arm a repository for it, what runs it in a terminal, and what
/// Canopy does that runs it.
struct Case {
    name: &'static str,
    /// What a refusal must name.
    names: &'static str,
    arm: fn(&Arena),
    /// What an ordinary terminal runs that starts the program.
    control: fn(&Arena),
    /// The user's action that starts it when the repository is trusted.
    attempt: Attempt,
    /// Held for the life of the case (a server a source talks to).
    serve: bool,
}

fn commit() -> Op {
    Op::Commit {
        message: "from canopy".into(),
        amend: false,
        signoff: false,
    }
}

fn fetch() -> Op {
    Op::Fetch {
        prune: false,
        quiet: false,
    }
}

/// Stages a change to `a.txt` and returns once it is in the index.
fn stage_a_change(arena: &Arena) {
    arena.write("a.txt", "one-main\ntwo\nthree\nstaged\n");
    arena.git(&["add", "a.txt"]);
}

/// What a signature read says when it was not tried.
const NOT_CHECKED: &str = "signature not checked";

/// A server that answers every request with `401 Unauthorized`, so git asks its credential
/// helpers for a login.
struct Unauthorized {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Unauthorized {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().expect("address").port();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                stream.set_nonblocking(false).expect("blocking");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("timeout");
                let mut request = Vec::new();
                let mut chunk = [0_u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&chunk[..n]),
                    }
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"x\"\r\n\
                      Content-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        Self {
            port,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Unauthorized {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A fake `gpg`: signs with a fixed armor and verifies anything as good.
const FAKE_GPG: &str = r#"
case "$*" in
*--verify*)
  echo "[GNUPG:] NEWSIG"
  echo "[GNUPG:] GOODSIG ABCDEF0123456789 Ada Lovelace <ada@example.com>"
  echo "[GNUPG:] VALIDSIG ABCDEF0123456789 2026-01-01 1767225600 0 4 0 1 8 00 ABCDEF0123456789"
  echo "[GNUPG:] TRUST_ULTIMATE 0 pgp"
  ;;
*)
  cat >/dev/null
  echo "-----BEGIN PGP SIGNATURE-----"
  echo ""
  echo "ZmFrZQ=="
  echo "-----END PGP SIGNATURE-----"
  echo "" >&2
  echo "[GNUPG:] SIG_CREATED D 1 8 00 1767225600 ABCDEF0123456789" >&2
  ;;
esac
"#;

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "pre-commit hook",
            names: "hook pre-commit",
            arm: |a| {
                stage_a_change(a);
                a.hook("pre-commit", "pre-commit hook");
            },
            control: |a| a.git(&["commit", "-q", "-m", "control"]),
            attempt: |w, _| run_op(w, commit()),
            serve: false,
        },
        Case {
            name: "post-checkout hook",
            names: "hook post-checkout",
            arm: |a| a.hook("post-checkout", "post-checkout hook"),
            control: |a| a.git(&["checkout", "-q", "topic"]),
            attempt: |w, _| run_op(w, Op::Checkout("topic".into())),
            serve: false,
        },
        Case {
            name: "clean filter",
            names: "filter.probe.clean",
            arm: |a| {
                let program = a.program("clean filter", "cat");
                a.config("filter.probe.clean", &program.to_string_lossy());
                a.write("a.txt", "one-main\ntwo\nthree\nmore\n");
            },
            control: |a| a.git(&["add", "-A"]),
            attempt: |w, _| run_op(w, Op::StageAll),
            serve: false,
        },
        Case {
            name: "smudge filter",
            names: "filter.probe.smudge",
            arm: |a| {
                let program = a.program("smudge filter", "cat");
                a.config("filter.probe.smudge", &program.to_string_lossy());
            },
            control: |a| a.git(&["checkout", "-q", "topic"]),
            attempt: |w, _| run_op(w, Op::Checkout("topic".into())),
            serve: false,
        },
        Case {
            name: "merge driver",
            names: "merge.probe.driver",
            arm: |a| {
                let program = a.program("merge driver", "exit 0");
                a.config(
                    "merge.probe.driver",
                    &format!("{} %O %A %B", program.display()),
                );
            },
            control: |a| a.git(&["merge", "-q", "--no-edit", "topic"]),
            attempt: |w, _| {
                run_op(
                    w,
                    Op::Merge("topic".into(), canopy_core::MergeMode::Default),
                )
            },
            serve: false,
        },
        Case {
            name: "submodule update command",
            names: "submodule.sub.update",
            arm: |a| {
                let source = a.root.path().join("subsource");
                std::fs::create_dir(&source).expect("subsource");
                for args in [
                    &["init", "-q", "-b", "main"][..],
                    &["commit", "-q", "--allow-empty", "-m", "sub"],
                ] {
                    assert!(terminal_git(&source, args).0, "git {args:?}");
                }
                a.git(&[
                    "-c",
                    "protocol.file.allow=always",
                    "submodule",
                    "add",
                    "-q",
                    &source.to_string_lossy(),
                    "sub",
                ]);
                a.git(&["commit", "-q", "-m", "add sub"]);
                // The submodule is now ahead of what the superproject records, so an update has
                // something to do.
                assert!(
                    terminal_git(
                        &a.repo.join("sub"),
                        &["commit", "-q", "--allow-empty", "-m", "ahead"]
                    )
                    .0
                );
                let program = a.program("submodule update command", "exit 0");
                a.config("submodule.sub.update", &format!("!{}", program.display()));
            },
            control: |a| a.git(&["submodule", "update"]),
            attempt: |w, _| {
                run_op(
                    w,
                    Op::SubmoduleUpdate {
                        path: None,
                        init: false,
                    },
                )
            },
            serve: false,
        },
        Case {
            name: "core.sshCommand",
            names: "core.sshcommand",
            arm: |a| {
                a.git(&["remote", "add", "origin", "ssh://git@127.0.0.1/x.git"]);
                let program = a.program("core.sshCommand", "exit 1");
                a.config("core.sshCommand", &program.to_string_lossy());
            },
            control: |a| {
                // Fails, having started the program.
                terminal_git(&a.repo, &["fetch", "--all"]);
            },
            attempt: |w, _| run_op(w, fetch()),
            serve: false,
        },
        Case {
            name: "credential helper",
            names: "credential.helper",
            // The server is started by the case runner and its port given in the arena's `port` file.
            arm: |a| {
                let port = std::fs::read_to_string(a.root.path().join("port")).expect("port");
                a.git(&[
                    "remote",
                    "add",
                    "origin",
                    &format!("http://127.0.0.1:{}/x.git", port.trim()),
                ]);
                let program = a.program("credential helper", "cat >/dev/null");
                a.config("credential.helper", &program.to_string_lossy());
            },
            control: |a| {
                terminal_git(&a.repo, &["fetch", "--all"]);
            },
            attempt: |w, _| run_op(w, fetch()),
            serve: true,
        },
        Case {
            name: "gpg.program (signing)",
            names: "gpg.program",
            arm: |a| {
                stage_a_change(a);
                let program = a.program("gpg.program (signing)", "exit 1");
                a.config("gpg.program", &program.to_string_lossy());
                a.config("commit.gpgsign", "true");
            },
            control: |a| {
                terminal_git(&a.repo, &["commit", "-q", "-m", "control"]);
            },
            attempt: |w, _| run_op(w, commit()),
            serve: false,
        },
        Case {
            name: "gpg.program (verifying a signature)",
            names: "gpg.program",
            arm: |a| {
                // A commit signed by the fake program, made before the marker is armed.
                let program = a.program("gpg.program (verifying a signature)", FAKE_GPG);
                a.config("gpg.program", &program.to_string_lossy());
                a.git(&["commit", "-q", "--allow-empty", "-S", "-m", "signed"]);
                a.reset("gpg.program (verifying a signature)");
            },
            control: |a| {
                terminal_git(&a.repo, &["log", "-1", "--show-signature"]);
            },
            attempt: |w, a| {
                w.send(Job::Signature(a.head()));
                let events = events_until(w, |e| matches!(e, Event::Signature { .. }));
                match events.last() {
                    Some(Event::Signature { status: None, .. }) => Some(NOT_CHECKED.into()),
                    _ => None,
                }
            },
            serve: false,
        },
    ]
}

/// Every program source that the user's own actions run: git runs it in a terminal, the
/// repository thread refuses the action in an untrusted repository and names the source, and
/// in a trusted one the action goes through and git runs the program as in a terminal.
#[test]
fn an_untrusted_repository_starts_no_program_and_a_trusted_one_runs_the_users_own() {
    if !in_isolated_git_env(
        "hostile::an_untrusted_repository_starts_no_program_and_a_trusted_one_runs_the_users_own",
    ) {
        return;
    }
    for case in cases() {
        let label = case.name;
        let server = case.serve.then(Unauthorized::start);

        // The control: ordinary git runs the program, so the case is armed.
        let control = Arena::new();
        if let Some(server) = &server {
            std::fs::write(control.root.path().join("port"), server.port.to_string())
                .expect("port");
        }
        (case.arm)(&control);
        assert!(!control.ran(label), "{label}: arming started the program");
        (case.control)(&control);
        assert!(
            control.ran(label),
            "{label}: ordinary git never ran it, so the case proves nothing"
        );

        // Untrusted: refused, naming the source, and nothing runs.
        let arena = Arena::new();
        if let Some(server) = &server {
            std::fs::write(arena.root.path().join("port"), server.port.to_string()).expect("port");
        }
        (case.arm)(&arena);
        let worker = Worker::spawn(arena.repo.clone(), &egui::Context::default());
        let refusal = (case.attempt)(&worker, &arena);
        settle(&worker);
        let Some(text) = refusal else {
            panic!("{label}: it was not refused");
        };
        if text != NOT_CHECKED {
            assert!(text.starts_with(READ_ONLY), "{label}: refused as {text}");
            assert!(
                text.contains(case.names),
                "{label}: the refusal names {} in {text}",
                case.names
            );
        }
        assert!(!arena.ran(label), "{label}: ran in an untrusted repository");

        // Trusted: it is the user's own action, and git runs the program as in a terminal.
        crate::trust::add_for_tests(arena.root.path().to_path_buf());
        let refusal = (case.attempt)(&worker, &arena);
        settle(&worker);
        assert!(
            refusal.is_none_or(|text| !text.starts_with(READ_ONLY)),
            "{label}: still refused after trusting"
        );
        assert!(
            arena.ran(label),
            "{label}: a trusted repository runs it, as in a terminal"
        );
        drop(server);
    }
}

/// Arms the `merge=probe` driver of the arena's attributes, so any merge git computes for
/// `a.txt` (both branches changed it) starts the program.
fn arm_merge_driver(arena: &Arena, label: &str) {
    let program = arena.program(label, "exit 0");
    arena.config(
        "merge.probe.driver",
        &format!("{} %O %A %B", program.display()),
    );
}

/// The squash check of the merged-branches read runs `git merge-tree`, which runs merge
/// drivers: opening an untrusted folder asks for it on every snapshot, so it is not made there
/// (the reachable and upstream-gone marks come from libgit2 and stay). Trusted, it runs as in a
/// terminal.
#[test]
fn the_merged_branches_read_runs_no_merge_driver_while_untrusted() {
    if !in_isolated_git_env(
        "hostile::the_merged_branches_read_runs_no_merge_driver_while_untrusted",
    ) {
        return;
    }
    let label = "merge driver merged";
    let control = Arena::new();
    arm_merge_driver(&control, label);
    control.git(&["merge-tree", "--write-tree", "main", "topic"]);
    assert!(
        control.ran(label),
        "ordinary git never ran the driver, so the case proves nothing"
    );

    let arena = Arena::new();
    arm_merge_driver(&arena, label);
    let worker = Worker::spawn(arena.repo.clone(), &egui::Context::default());
    let read = |serial| {
        worker.send(Job::MergedBranches {
            serial,
            bases: vec!["main".into()],
            squash: true,
        });
        let events = events_until(&worker, |e| matches!(e, Event::MergedBranches { .. }));
        match events.into_iter().last() {
            Some(Event::MergedBranches { result, .. }) => result.expect("the read answers"),
            other => panic!("{other:?}"),
        }
    };
    let rows = read(1);
    assert!(
        rows.iter().any(|row| row.name == "topic"),
        "the other branches are still listed: {rows:?}"
    );
    assert!(
        !arena.ran(label),
        "the squash check ran the merge driver of an untrusted repository"
    );

    crate::trust::add_for_tests(arena.root.path().to_path_buf());
    read(2);
    assert!(
        arena.ran(label),
        "a trusted repository runs the squash check, as in a terminal"
    );
}

/// The base comparison predicts conflicts with `git merge-tree`, which runs merge drivers: an
/// untrusted repository refuses it, naming the source, and a trusted one runs it as in a
/// terminal.
#[test]
fn the_base_comparison_runs_no_merge_driver_while_untrusted() {
    if !in_isolated_git_env("hostile::the_base_comparison_runs_no_merge_driver_while_untrusted") {
        return;
    }
    let label = "merge driver base";
    let control = Arena::new();
    arm_merge_driver(&control, label);
    control.git(&["merge-tree", "--write-tree", "main", "topic"]);
    assert!(
        control.ran(label),
        "ordinary git never ran the driver, so the case proves nothing"
    );

    let arena = Arena::new();
    arm_merge_driver(&arena, label);
    let worker = Worker::spawn(arena.repo.clone(), &egui::Context::default());
    let compare = |serial| {
        worker.send(Job::BaseCompare {
            serial,
            base: "topic".into(),
        });
        let events = events_until(&worker, |e| matches!(e, Event::BaseCompare { .. }));
        match events.into_iter().last() {
            Some(Event::BaseCompare { result, .. }) => result,
            other => panic!("{other:?}"),
        }
    };
    let refused = compare(1).expect_err("an untrusted repository refuses the comparison");
    assert!(refused.starts_with(READ_ONLY), "{refused}");
    assert!(refused.contains("merge.probe.driver"), "{refused}");
    assert!(
        !arena.ran(label),
        "the comparison ran the merge driver of an untrusted repository"
    );

    crate::trust::add_for_tests(arena.root.path().to_path_buf());
    let preview = compare(2).expect("a trusted repository compares");
    assert_eq!(preview.conflicts, Some(Vec::new()), "{preview:?}");
    assert!(
        arena.ran(label),
        "a trusted repository runs it, as in a terminal"
    );
}
