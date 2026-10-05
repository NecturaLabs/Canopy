//! Headless screenshots for `--screenshot` (tools/screenshot.sh).

use std::sync::Arc;

use super::*;

#[derive(Debug)]
pub(super) struct Screenshot {
    pub(super) path: PathBuf,
    pub(super) ready_frames: u32,
    pub(super) ready_since: Option<f64>,
    pub(super) requested: bool,
    pub(super) started: f64,
    /// A popup kept open for the picture: popups close when a frame does not show them.
    pub(super) popup: Option<egui::Id>,
}

impl CanopyApp {
    /// Makes the active tab show the `--demo-requests` scenario (see `demo_requests`), and the
    /// settings the requests view needs to draw it. Whether the picture may be taken yet.
    fn show_demo_requests(
        &mut self,
        ctx: &egui::Context,
        scenario: demo_requests::Scenario,
    ) -> bool {
        let now = chrono::Utc::now().timestamp();
        let shown = match self.tabs.get_mut(self.active) {
            Some(tab) => scenario.show(ctx, tab, now),
            // Nothing is open (the welcome screen): there is nothing to wait for.
            None => return true,
        };
        // The view draws only when pull and merge requests are on. That is switched on only once
        // the tab holds the demo's API thread, so the next frame has no reason to start a real
        // one; the pictures it would fetch from the host stay off.
        self.settings.hosting_api = shown;
        self.settings.download_avatars = false;
        shown
    }

    pub(super) fn handle_screenshot(&mut self, ctx: &egui::Context) {
        if self.screenshot.is_none() {
            return;
        }
        // `--demo-requests`: the active tab shows invented requests instead of loading them.
        let demo = demo_requests::chosen();
        let demo_shown = demo.is_none_or(|scenario| self.show_demo_requests(ctx, scenario));
        let Some(shot) = &mut self.screenshot else {
            return;
        };
        let now = ctx.input(|i| i.time);
        if shot.started == 0.0 {
            shot.started = now;
        }
        let ready = demo_shown
            && self.pending_opens == 0
            && !crate::ui::art::growing(ctx)
            && self.tabs.iter().all(|t| {
                // A demo scene may be a loading or an error state on purpose: it is final.
                let requests_ready = demo.is_some()
                    || t.view != View::Requests
                    || t.host.is_none()
                    || (!t.requests.loading
                        && !t.requests.details_loading
                        && t.requests.list.is_some());
                t.snap.is_some()
                    && t.running.is_none()
                    && (t.diff.is_some() || t.file.is_none())
                    && requests_ready
                    && !t.avatars.pending()
            });
        if ready {
            if let Some(id) = shot.popup {
                egui::Popup::open_id(ctx, id);
            }
            shot.ready_frames += 1;
            shot.ready_since.get_or_insert(now);
        } else {
            shot.ready_frames = 0;
            shot.ready_since = None;
        }
        let settled =
            shot.ready_frames > 30 && shot.ready_since.is_some_and(|since| now - since > 0.3);
        if !shot.requested && (settled || now - shot.started > 12.0) {
            shot.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(Arc::clone(image)),
                _ => None,
            })
        });
        if let Some(image) = image {
            match save_png(&shot.path, &image) {
                Ok(()) => tracing::info!(path = %shot.path.display(), "screenshot saved"),
                Err(e) => tracing::error!("screenshot failed: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn save_png(path: &std::path::Path, image: &egui::ColorImage) -> anyhow::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(
        std::io::BufWriter::new(file),
        u32::try_from(image.size[0])?,
        u32::try_from(image.size[1])?,
    );
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()?;
    let bytes: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(egui::Color32::to_srgba_unmultiplied)
        .collect();
    writer.write_image_data(&bytes)?;
    Ok(())
}
