//! Native folder selection without blocking the application's event loop.

use std::path::{Path, PathBuf};

use crossbeam_channel::{Receiver, TryRecvError, bounded};
use egui::{Id, Ui};
use egui_phosphor::regular as icon;

use super::theme::palette;

#[derive(Debug, Default)]
pub struct FolderPicker {
    pending: Option<(Id, Receiver<Option<PathBuf>>)>,
    visible: bool,
    error: Option<(Id, String)>,
}

impl FolderPicker {
    pub fn cancel(&mut self) {
        self.pending = None;
        self.error = None;
    }

    pub fn begin_frame(&mut self) {
        self.visible = false;
    }

    /// Forget a result if the field that requested it has closed.
    pub fn end_frame(&mut self) {
        if !self.visible {
            self.cancel();
        }
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        id: Id,
        pick: impl FnOnce() -> Option<PathBuf> + Send + 'static,
    ) {
        let (tx, rx) = bounded(1);
        let ctx = ctx.clone();
        match std::thread::Builder::new()
            .name("folder-picker".into())
            .spawn(move || {
                let result = pick();
                let _ = tx.send(result);
                ctx.request_repaint();
            }) {
            Ok(_) => {
                self.pending = Some((id, rx));
                self.visible = true;
                self.error = None;
            }
            Err(error) => {
                tracing::error!("could not start folder picker: {error}");
                self.error = Some((id, error.to_string()));
            }
        }
    }

    /// Shows Browse and returns a selection only to the field that requested it.
    pub fn show(&mut self, ui: &mut Ui, salt: &str, start: &Path) -> Option<PathBuf> {
        self.show_labelled(ui, salt, start, "Browse…")
    }

    /// [`Self::show`] with the button's own wording, for where the action is more than browsing.
    pub fn show_labelled(
        &mut self,
        ui: &mut Ui,
        salt: &str,
        start: &Path,
        idle_label: &str,
    ) -> Option<PathBuf> {
        let id = Id::new(("folder-picker", salt));
        let active = self
            .pending
            .as_ref()
            .is_some_and(|(target, _)| *target == id);
        self.visible |= active || self.error.as_ref().is_some_and(|(target, _)| *target == id);
        let mut selected = None;
        if active && let Some((_, rx)) = &self.pending {
            match rx.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    selected = result;
                }
                Err(TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.error = Some((
                        id,
                        "The folder chooser closed unexpectedly. Try again.".into(),
                    ));
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let label = if self.pending.is_some() && active {
            "Choosing…"
        } else {
            idle_label
        };
        if ui
            .add_enabled(
                self.pending.is_none(),
                egui::Button::new(format!("{}  {label}", icon::FOLDER)),
            )
            .clicked()
        {
            let start = start.to_path_buf();
            self.start(ui.ctx(), id, move || {
                rfd::FileDialog::new().set_directory(start).pick_folder()
            });
        }
        if let Some((target, error)) = &self.error
            && *target == id
        {
            ui.label(egui::RichText::new(error).color(palette(ui.ctx()).danger));
        }
        selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selection_reaches_only_the_requesting_field() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let (tx, rx) = bounded(1);
        let path = PathBuf::from("selected folder");
        tx.send(Some(path.clone())).expect("selected folder");
        let mut picker = FolderPicker {
            pending: Some((Id::new(("folder-picker", "open")), rx)),
            ..Default::default()
        };
        let mut selected = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(picker.show(ui, "clone", Path::new(".")).is_none());
            assert!(
                picker.pending.is_some(),
                "another field must not consume the selection"
            );
            selected = picker.show(ui, "open", Path::new("."));
        });
        output.textures_delta.clear();
        assert_eq!(selected, Some(path));
        assert!(picker.pending.is_none());
    }

    #[test]
    fn picker_errors_end_with_the_requesting_field() {
        let mut picker = FolderPicker {
            error: Some((Id::new("open"), "chooser error".into())),
            ..Default::default()
        };
        picker.cancel();
        assert!(picker.error.is_none());
    }

    #[test]
    fn a_disconnected_picker_reports_failure_and_allows_retry() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let (tx, rx) = bounded(1);
        let mut picker = FolderPicker {
            pending: Some((Id::new(("folder-picker", "open")), rx)),
            ..Default::default()
        };
        drop(tx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            picker.show(ui, "open", Path::new("."));
        });
        output.textures_delta.clear();
        assert!(picker.pending.is_none(), "failed worker must allow retry");
        assert!(picker.error.is_some(), "worker failure must be visible");
    }

    #[test]
    fn selecting_a_folder_leaves_the_caller_free_and_closing_discards_the_result() {
        let ctx = egui::Context::default();
        let mut picker = FolderPicker::default();
        let (started_tx, started_rx) = bounded(1);
        let (release_tx, release_rx) = bounded(1);
        picker.start(&ctx, Id::new("open"), move || {
            started_tx.send(()).expect("signal start");
            release_rx.recv().expect("release selection");
            Some(PathBuf::from("fixture"))
        });
        started_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("worker started without blocking caller");
        assert!(
            picker
                .pending
                .as_ref()
                .expect("pending picker")
                .1
                .try_recv()
                .is_err()
        );
        picker.begin_frame();
        picker.end_frame();
        assert!(
            picker.pending.is_none(),
            "closed field must not receive a late selection"
        );
        release_tx.send(()).expect("finish worker");
    }
}
