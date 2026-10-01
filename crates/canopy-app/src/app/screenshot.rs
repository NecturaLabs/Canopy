//! Headless screenshots for `--screenshot` (tools/screenshot.sh).

use super::*;

#[derive(Debug)]
pub(super) struct Screenshot {
    pub(super) path: PathBuf,
    pub(super) ready_frames: u32,
    pub(super) ready_since: Option<f64>,
    pub(super) requested: bool,
    pub(super) started: f64,
}

impl CanopyApp {
    pub(super) fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &mut self.screenshot else {
            return;
        };
        let now = ctx.input(|i| i.time);
        if shot.started == 0.0 {
            shot.started = now;
        }
        let ready = self.pending_opens == 0
            && self.tabs.iter().all(|t| {
                let requests_ready = t.view != View::Requests
                    || t.host.is_none()
                    || (!t.requests.loading
                        && !t.requests.details_loading
                        && t.requests.list.is_some());
                t.snap.is_some()
                    && t.running.is_none()
                    && (t.diff.is_some() || t.file.is_none())
                    && requests_ready
            });
        if ready {
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
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
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
        .flat_map(|c| c.to_srgba_unmultiplied())
        .collect();
    writer.write_image_data(&bytes)?;
    Ok(())
}
