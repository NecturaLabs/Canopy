//! Canopy: a fast, beautiful, Linux-first Git client.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod highlight;
mod hosting;
mod keymap;
mod logging;
mod settings;
mod tab;
mod ui;
mod updater;
mod views;
mod worker;

use std::path::PathBuf;

use app::{CanopyApp, Launch};

const HELP: &str = "\
Canopy — a fast, beautiful, Linux-first Git client

USAGE:
    canopy [OPTIONS] [PATH]...

ARGS:
    [PATH]...    Repositories to open (any folder inside one works)

OPTIONS:
        --light               Start with the Daylight theme
        --theme <THEME>       Start with a theme: midnight, daylight, aurora, ember, neon,
                              forest or paper
        --view <VIEW>         Initial view: history, changes or requests
        --split               Show diffs side by side
        --overlay <NAME>      Open settings, cheatsheet or palette (for screenshots)
        --size <WxH>          Initial window size [default: 1440x900]
        --screenshot <FILE>   Render once the repository has loaded, save a PNG and exit
    -h, --help                Print help
    -V, --version             Print version

ENVIRONMENT:
    CANOPY_LOG    Log filter, e.g. debug or canopy_core=trace (falls back to RUST_LOG)

FILES:
    Logs and crash reports are kept in Canopy's data folder (~/.local/share/canopy on Linux)
    and never leave your computer.";

fn parse_args() -> Result<(Launch, [f32; 2]), String> {
    let mut launch = Launch::default();
    let mut size = [1440.0, 900.0];
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy().into_owned();
        match text.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("canopy {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--light" => launch.theme = Some(ui::theme::ThemeId::Daylight),
            "--split" => launch.split = true,
            "--theme" => {
                let name = args
                    .next()
                    .ok_or("--theme needs a name")?
                    .to_string_lossy()
                    .to_lowercase();
                launch.theme = Some(
                    ui::theme::ThemeId::ALL
                        .into_iter()
                        .find(|t| t.name().to_lowercase() == name)
                        .ok_or_else(|| format!("unknown theme {name}; see --help"))?,
                );
            }
            "--overlay" => {
                launch.overlay = Some(
                    args.next()
                        .ok_or("--overlay needs settings, cheatsheet or palette")?
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            "--screenshot" => {
                launch.screenshot = Some(args.next().ok_or("--screenshot needs a file")?.into())
            }
            "--view" => {
                launch.view = Some(
                    match args
                        .next()
                        .map(|v| v.to_string_lossy().into_owned())
                        .as_deref()
                    {
                        Some("history") => tab::View::History,
                        Some("changes") => tab::View::Changes,
                        Some("requests") => tab::View::Requests,
                        _ => return Err("--view takes history, changes or requests".into()),
                    },
                );
            }
            "--size" => {
                let v = args
                    .next()
                    .ok_or("--size needs WxH")?
                    .to_string_lossy()
                    .into_owned();
                let (w, h) = v
                    .split_once('x')
                    .ok_or("--size takes WxH, e.g. 1600x1000")?;
                size = [
                    w.parse().map_err(|_| "bad width")?,
                    h.parse().map_err(|_| "bad height")?,
                ];
            }
            s if s.starts_with('-') => return Err(format!("unknown option {s}; see --help")),
            _ => launch.paths.push(PathBuf::from(arg)),
        }
    }
    Ok((launch, size))
}

fn icon() -> Option<egui::IconData> {
    let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!(
        "../assets/icon-256.png"
    )));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    Some(egui::IconData {
        rgba: buf,
        width: info.width,
        height: info.height,
    })
}

fn main() -> anyhow::Result<()> {
    let filter = std::env::var("CANOPY_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .unwrap_or_else(|_| "warn,canopy_app=info,canopy_core=info".into());
    logging::init(&filter);

    let (launch, size) = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("canopy: {e}");
            std::process::exit(2);
        }
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Canopy")
        .with_app_id("io.necturalabs.Canopy")
        .with_inner_size(size)
        .with_min_inner_size([900.0, 560.0]);
    if let Some(icon) = icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        persist_window: launch.screenshot.is_none(),
        ..Default::default()
    };
    eframe::run_native(
        "canopy",
        options,
        Box::new(move |cc| Ok(Box::new(CanopyApp::new(cc, launch)))),
    )
    .map_err(|e| anyhow::anyhow!("could not start the window: {e}"))
}
