//! Git identity work shared by repository tabs and the first-launch question.

use std::path::PathBuf;

use canopy_core::identity::{self, Identity, Scope};
use crossbeam_channel::{Receiver, Sender, unbounded};

pub enum Job {
    ReadRepo(PathBuf),
    ReadGlobal,
    Set {
        repo: Option<PathBuf>,
        scope: Scope,
        name: String,
        email: String,
    },
    Clear {
        repo: Option<PathBuf>,
        scope: Scope,
    },
    Config {
        repo: Option<PathBuf>,
        scope: Scope,
        setting: canopy_core::config::Setting,
        value: Option<String>,
    },
    Recommendations,
}

pub enum Event {
    Repo {
        path: PathBuf,
        result: Result<Identity, String>,
    },
    Global(Result<Identity, String>),
    Changed {
        scope: Scope,
        cleared: bool,
        result: Result<(), String>,
        /// After a clear: what Git uses now, to tell the user what the clear left in force.
        now: Option<Identity>,
    },
    ConfigChanged {
        repo: Option<PathBuf>,
        recommended: bool,
        result: Result<(), String>,
    },
}

pub struct IdentityWorker {
    tx: Sender<Job>,
    pub events: Receiver<Event>,
}

impl IdentityWorker {
    pub fn start(ctx: &egui::Context) -> Option<Self> {
        let (tx, jobs) = unbounded();
        let (events, rx) = unbounded();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("git-identity".into())
            .spawn(move || {
                for job in jobs {
                    let event = match job {
                        Job::Config {
                            repo,
                            scope,
                            setting,
                            value,
                        } => {
                            let home = crate::settings::home_dir();
                            let dir = repo.as_deref().unwrap_or(&home);
                            Event::ConfigChanged {
                                result: canopy_core::config::set(
                                    dir,
                                    scope,
                                    setting,
                                    value.as_deref(),
                                )
                                .map_err(|error| error.to_string()),
                                repo,
                                recommended: false,
                            }
                        }
                        Job::Recommendations => Event::ConfigChanged {
                            repo: None,
                            recommended: true,
                            result: canopy_core::config::apply_recommended(
                                &crate::settings::home_dir(),
                            )
                            .map_err(|error| error.to_string()),
                        },
                        Job::ReadRepo(path) => Event::Repo {
                            result: Identity::read(&path).map_err(|e| e.to_string()),
                            path,
                        },
                        Job::ReadGlobal => {
                            Event::Global(Identity::read_global().map_err(|e| e.to_string()))
                        }
                        Job::Set {
                            repo,
                            scope,
                            name,
                            email,
                        } => Event::Changed {
                            result: identity::set(repo.as_deref(), scope, &name, &email)
                                .map_err(|e| e.to_string()),
                            scope,
                            cleared: false,
                            now: None,
                        },
                        Job::Clear { repo, scope } => {
                            let result =
                                identity::clear(repo.as_deref(), scope).map_err(|e| e.to_string());
                            let now = if result.is_ok() {
                                match repo.as_deref() {
                                    Some(path) => Identity::read(path),
                                    None => Identity::read_global(),
                                }
                                .ok()
                            } else {
                                None
                            };
                            Event::Changed {
                                result,
                                scope,
                                cleared: true,
                                now,
                            }
                        }
                    };
                    if events.send(event).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .ok()?;
        Some(Self { tx, events: rx })
    }

    pub fn send(&self, job: Job) -> bool {
        self.tx.send(job).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::Duration;

    use super::*;

    fn next(worker: &IdentityWorker) -> Event {
        worker
            .events
            .recv_timeout(Duration::from_secs(30))
            .expect("the worker answers")
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "tests drive git directly to build and check repositories"
    )]
    fn local(path: &std::path::Path, key: &str) -> Option<String> {
        let out = Command::new("git")
            .current_dir(path)
            .args(["config", "--local", "--get", key])
            .output()
            .expect("git runs");
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "tests drive git directly to build and check repositories"
    )]
    fn jobs_run_in_order_and_answer_with_events() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().to_path_buf();
        let init = Command::new("git")
            .current_dir(&path)
            .args(["init", "-q"])
            .status()
            .expect("git runs");
        assert!(init.success());
        let worker = IdentityWorker::start(&egui::Context::default()).expect("thread starts");

        assert!(worker.send(Job::Set {
            repo: Some(path.clone()),
            scope: Scope::Local,
            name: "Worker Test".into(),
            email: "worker@example.com".into(),
        }));
        match next(&worker) {
            Event::Changed {
                cleared, result, ..
            } => {
                assert!(!cleared);
                assert_eq!(result, Ok(()));
            }
            _ => panic!("a save answers with Changed"),
        }
        assert_eq!(local(&path, "user.name").as_deref(), Some("Worker Test"));
        assert_eq!(
            local(&path, "user.email").as_deref(),
            Some("worker@example.com")
        );

        assert!(worker.send(Job::ReadRepo(path.clone())));
        match next(&worker) {
            Event::Repo { path: read, result } => {
                assert_eq!(read, path, "the answer names the repository it read");
                assert!(result.is_ok(), "{result:?}");
            }
            _ => panic!("a read answers with Repo"),
        }

        assert!(worker.send(Job::Set {
            repo: Some(path.clone()),
            scope: Scope::Local,
            name: "Someone".into(),
            email: "not an address".into(),
        }));
        match next(&worker) {
            Event::Changed { result, .. } => {
                assert!(result.is_err(), "an unusable address is refused");
            }
            _ => panic!("a refused save still answers with Changed"),
        }
        assert_eq!(
            local(&path, "user.name").as_deref(),
            Some("Worker Test"),
            "a refused save writes nothing"
        );

        assert!(worker.send(Job::Clear {
            repo: Some(path.clone()),
            scope: Scope::Local,
        }));
        match next(&worker) {
            Event::Changed {
                cleared, result, ..
            } => {
                assert!(cleared);
                assert_eq!(result, Ok(()));
            }
            _ => panic!("a clear answers with Changed"),
        }
        assert_eq!(local(&path, "user.name"), None);
        assert_eq!(local(&path, "user.email"), None);

        assert!(worker.send(Job::Config {
            repo: Some(path.clone()),
            scope: Scope::Local,
            setting: canopy_core::config::Setting::FetchPrune,
            value: Some("false".into()),
        }));
        assert!(worker.send(Job::Config {
            repo: Some(path.clone()),
            scope: Scope::Local,
            setting: canopy_core::config::Setting::PushDefault,
            value: Some("current".into()),
        }));
        for _ in 0..2 {
            match next(&worker) {
                Event::ConfigChanged {
                    repo,
                    recommended,
                    result,
                } => {
                    assert_eq!(repo.as_ref(), Some(&path));
                    assert!(!recommended);
                    assert_eq!(result, Ok(()));
                }
                _ => panic!("config saves answer in queue order"),
            }
        }
        assert_eq!(local(&path, "fetch.prune").as_deref(), Some("false"));
        assert_eq!(local(&path, "push.default").as_deref(), Some("current"));
        assert!(worker.send(Job::Config {
            repo: Some(path.clone()),
            scope: Scope::Local,
            setting: canopy_core::config::Setting::FetchPrune,
            value: None,
        }));
        assert!(matches!(
            next(&worker),
            Event::ConfigChanged { result: Ok(()), .. }
        ));
        assert_eq!(local(&path, "fetch.prune"), None);

        assert!(worker.send(Job::ReadGlobal));
        assert!(matches!(next(&worker), Event::Global(_)));
    }
}
