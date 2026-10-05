//! Cloning a repository on a background thread.

use super::*;

#[derive(Debug)]
pub(super) enum CloneMsg {
    Progress(String),
    Done(Result<PathBuf, String>),
}

impl CanopyApp {
    pub(super) fn start_clone(&mut self, ctx: &egui::Context, url: String, dest: PathBuf) {
        let (tx, rx) = unbounded();
        let thread_ctx = ctx.clone();
        let thread_url = url.clone();
        let spawned = std::thread::Builder::new()
            .name("clone".into())
            .spawn(move || {
                let send = |m: CloneMsg| {
                    let _ = tx.send(m);
                    thread_ctx.request_repaint();
                };
                let result = clone_repository(&thread_url, &dest, |l| {
                    send(CloneMsg::Progress(l.to_owned()))
                });
                send(CloneMsg::Done(result));
            });
        match spawned {
            Ok(_) => self.clone = Some((url, rx, "Starting…".into())),
            // The status bar never shows a clone that is not running.
            Err(e) => {
                self.dialogs.finish_clone(false);
                tracing::error!("could not start the clone thread: {e}");
                self.toasts.push(
                    ctx,
                    ToastKind::Error,
                    "Clone failed",
                    format!("Could not start the clone: {e}"),
                );
            }
        }
    }
}

fn clone_repository(
    url: &str,
    dest: &std::path::Path,
    progress: impl FnMut(&str),
) -> Result<PathBuf, String> {
    if dest
        .try_exists()
        .map_err(|e| format!("Could not check {}: {e}", dest.display()))?
    {
        return Err(format!(
            "The destination folder already exists: {}",
            dest.display()
        ));
    }
    canopy_core::ops::clone(url, dest, progress).map_err(|e| e.to_string())?;
    Ok(dest.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_existing_destination_is_refused_before_cloning() {
        let dir = tempfile::tempdir().expect("temporary clone fixture");
        let source = dir.path().join("source");
        Repo::init(&source).expect("empty local repository");
        let dest = dir.path().join("existing");
        std::fs::create_dir(&dest).expect("existing destination");
        let result = clone_repository(&source.to_string_lossy(), &dest, |_| {});
        assert!(
            result.is_err(),
            "existing folders must be refused before Git accepts an empty one"
        );
        assert!(
            !dest.join(".git").exists(),
            "existing folder must be untouched"
        );
    }
}
