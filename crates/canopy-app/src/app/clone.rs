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
                let result = canopy_core::ops::clone(&thread_url, &dest, |l| {
                    send(CloneMsg::Progress(l.to_owned()))
                });
                send(CloneMsg::Done(
                    result.map(|()| dest).map_err(|e| e.to_string()),
                ));
            });
        match spawned {
            Ok(_) => self.clone = Some((url, rx, "Starting…".into())),
            // The status bar never shows a clone that is not running.
            Err(e) => {
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
