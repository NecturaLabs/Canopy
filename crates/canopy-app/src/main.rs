//! Canopy: a fast, beautiful, Linux-first Git client.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod action;
mod app;
mod askpass;
mod avatars;
mod branch_tree;
mod build_info;
mod crash_reports;
mod frame_stats;
mod highlight;
#[cfg(test)]
#[cfg(unix)]
mod hostile;
mod hosting;
mod housekeeping;
mod identity_worker;
mod keymap;
mod lfs;
mod logging;
mod platform;
mod read_limit;
mod repo_config;
mod request_view;
mod selfupdate;
mod settings;
mod settings_file;
mod settings_store;
mod storage;
mod system_apps;
mod system_theme;
mod tab;
mod trust;
mod ui;
mod updater;
mod views;
mod watcher;
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
        --theme <THEME>       Start with a theme: midnight, daylight, observatory or custom
        --view <VIEW>         Initial view: history, changes or requests
        --split               Show diffs side by side
        --overlay <NAME>      Open an overlay for screenshots: settings, privacy, cheatsheet,
                              palette, consent, consent-updated, crash-consent, legal, credential, host-key,
                              merge, identity, settings-git, settings-git-pull, settings-git-commits, settings-git-config,
                              clear-identity, update-installed, update-running, trust-prompt,
                              trust-banner, trust-safety-off, privacy-trust-off, diff-options,
                              settings-integrations, settings-integrations-custom, settings-accounts, settings-updates, settings-notifications,
                              settings-keyboard, settings-backup, settings-import-ignored,
                              settings-keyboard-capture, settings-keyboard-refused, settings-repository,
                              settings-repository-pull, settings-repository-diffs,
                              settings-search, settings-search-none, about, repository-insights, file-history, file-history-commit, lfs, lfs-patterns, lfs-locks, search-changes, clean-stale, remove-worktree, remove-worktree-dirty, remove-worktree-delete, forget-worktrees, worktrees, menu-worktree, clean-merged, update-from-base, flow-init, flow-start, flow-finish, force-delete, author-card, request-filters, request-sort, commit-expanded, commit-compact, amend-commit, toasts, force-push-stopped, conflict-notice, git-errors, git-errors-center, notifications, branch-selected,
                              long-message,
                              gallery, menu-branch, menu-commit, menu-file, menu-lfs-file, blame, open,
                              open-error, clone,
                              clone-running, clone-error, init, new-branch, new-worktree,
                              new-worktree-error, rename, add-submodule,
                              git-missing, remove-submodule, discard-submodule, new-tag,
                              stash, export-patch, apply-patch, add-remote, add-remote-refused, publish, new-request,
                              confirm, confirm-danger, confirm-host, convert-partial-clone, confirm-merge-risky,
                              force-push, force-push-new, diverged, pull-conflicts, pull-moved, whats-new, crash, update-available,
                              update-failed, terms, consent-reading, crash-reading, username,
                              password, approve or notice
        --size <WxH>          Initial window size [default: 1440x900]
        --zoom <FACTOR>       Initial interface zoom, from 0.8 to 1.6
        --style <STYLE>       Start with a style: standard or deeper
        --density <DENSITY>   Start with a density: comfortable or dense
        --request-density <D> Start the request list with comfortable or dense items
        --translucent <ON>    Translucent surfaces: on or off
        --contrast <ON>       High contrast: on or off
        --custom-theme <SPEC> The custom theme's colours, comma-separated: base=midnight|daylight,
                              accent=<hue 0-359>, intensity=<20-140>, tint=<0-100>,
                              tint-hue=<0-359> (with --theme custom)
        --motion <MOTION>     Motion: system, reduced or full
        --system <MODE>       Match system, as in a dark or light system: dark or light
        --gallery-page <N>    With --overlay gallery, scroll down N window heights
        --screenshot <FILE>   Render once the repository has loaded, save a PNG and exit
        --demo-requests <V>   With --screenshot, show invented pull requests (no network) in a
                              repository without a GitHub or GitLab remote: list, list-all,
                              list-closed, list-unchecked, list-overflow, list-more, list-search, filtered, no-match,
                              a request number such as 428, busy, loading, details-loading,
                              details-error, error, signed-out or empty;
                              prefix gitlab: for merge requests. Implies --view requests
    -h, --help                Print help
    -V, --version             Print version

ENVIRONMENT:
    CANOPY_LOG    Log filter, e.g. debug or canopy_core=trace (falls back to RUST_LOG);
                  overrides Settings → Privacy → Log detail

FILES:
    Logs (one JSON-lines file per run) and crash reports are kept in Canopy's state folder
    (~/.local/state/canopy on Linux) and never leave your computer.";

#[expect(
    clippy::print_stdout,
    clippy::exit,
    reason = "--help and --version print to the terminal and quit before any window opens"
)]
fn parse_args() -> Result<(Launch, [f32; 2]), String> {
    let mut launch = Launch::default();
    let mut size = [1440.0, 900.0];
    let mut demo_requests = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy().into_owned();
        match text.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("canopy {}", build_info::label());
                std::process::exit(0);
            }
            "--light" => launch.theme = Some(ui::theme::ThemeId::Daylight),
            "--split" => launch.split = true,
            "--theme" => {
                let name = args
                    .next()
                    .ok_or("--theme needs a name")?
                    .to_string_lossy()
                    .into_owned();
                launch.theme = Some(
                    ui::theme::ThemeId::from_name(&name)
                        .ok_or_else(|| format!("unknown theme {name}; see --help"))?,
                );
            }
            "--style" => {
                let name = next_text(&mut args, "--style needs standard or deeper")?;
                launch.style = Some(
                    ui::theme::Style::from_name(&name).ok_or("--style takes standard or deeper")?,
                );
            }
            "--density" => {
                let name = next_text(&mut args, "--density needs comfortable or dense")?;
                launch.density = Some(
                    ui::theme::Density::from_name(&name)
                        .ok_or("--density takes comfortable or dense")?,
                );
            }
            "--request-density" => {
                let name = next_text(&mut args, "--request-density needs comfortable or dense")?;
                launch.request_density = Some(
                    request_view::RequestDensity::from_name(&name)
                        .ok_or("--request-density takes comfortable or dense")?,
                );
            }
            "--translucent" => {
                launch.translucent = Some(
                    match next_text(&mut args, "--translucent needs on or off")?.as_str() {
                        "on" => true,
                        "off" => false,
                        _ => return Err("--translucent takes on or off".into()),
                    },
                );
            }
            "--custom-theme" => {
                let spec = next_text(&mut args, "--custom-theme needs a list, see --help")?;
                launch.custom_theme = Some(parse_custom_theme(&spec)?);
            }
            "--contrast" => {
                launch.high_contrast = Some(
                    match next_text(&mut args, "--contrast needs on or off")?.as_str() {
                        "on" => true,
                        "off" => false,
                        _ => return Err("--contrast takes on or off".into()),
                    },
                );
            }
            "--system" => {
                launch.system_dark = Some(
                    match next_text(&mut args, "--system needs dark or light")?.as_str() {
                        "dark" => true,
                        "light" => false,
                        _ => return Err("--system takes dark or light".into()),
                    },
                );
            }
            "--motion" => {
                let name = next_text(&mut args, "--motion needs system, reduced or full")?;
                launch.motion = Some(
                    ui::motion::MotionPref::from_name(&name)
                        .ok_or("--motion takes system, reduced or full")?,
                );
            }
            "--glass-soak" if cfg!(debug_assertions) => {
                #[expect(
                    clippy::map_err_ignore,
                    reason = "the option's own message says what a valid value is"
                )]
                let frames = next_text(&mut args, "--glass-soak needs a number of frames")?
                    .parse()
                    .map_err(|_| "--glass-soak takes a whole number of frames")?;
                launch.glass_soak = Some(frames);
            }
            "--gallery-page" => {
                #[expect(
                    clippy::map_err_ignore,
                    reason = "the option's own message says what a valid value is"
                )]
                let page = next_text(&mut args, "--gallery-page needs a number")?
                    .parse()
                    .map_err(|_| "--gallery-page takes a whole number")?;
                launch.gallery_page = page;
            }
            "--overlay" => {
                let names = app::OVERLAYS.join(", ");
                let name = args
                    .next()
                    .ok_or_else(|| format!("--overlay needs one of {names}"))?
                    .to_string_lossy()
                    .into_owned();
                if !app::OVERLAYS.contains(&name.as_str()) {
                    return Err(format!("unknown overlay {name}; use one of {names}"));
                }
                launch.overlay = Some(name);
            }
            "--zoom" => {
                let value = args.next().ok_or("--zoom needs a factor from 0.8 to 1.6")?;
                launch.zoom = Some(parse_zoom(&value.to_string_lossy())?);
            }
            "--demo-requests" => {
                let spec = next_text(&mut args, "--demo-requests needs a scenario, see --help")?;
                if demo_requests.is_some() {
                    return Err("--demo-requests was given twice".into());
                }
                demo_requests = Some(app::demo_requests::Scenario::parse(&spec)?);
            }
            "--screenshot" => {
                launch.screenshot = Some(args.next().ok_or("--screenshot needs a file")?.into());
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
                #[expect(
                    clippy::map_err_ignore,
                    reason = "the option's own message says what a valid value is"
                )]
                let parsed = [
                    w.parse().map_err(|_| "bad width")?,
                    h.parse().map_err(|_| "bad height")?,
                ];
                size = parsed;
            }
            s if s.starts_with('-') => return Err(format!("unknown option {s}; see --help")),
            _ => launch.paths.push(PathBuf::from(arg)),
        }
    }
    if let Some(scenario) = demo_requests {
        app::demo_requests::enable(scenario, launch.screenshot.is_some())?;
        launch.view.get_or_insert(tab::View::Requests);
    }
    Ok((launch, size))
}

/// The custom theme of a `--custom-theme` argument: `key=value` pairs over the defaults.
fn parse_custom_theme(spec: &str) -> Result<ui::custom_theme::CustomTheme, String> {
    let mut c = ui::custom_theme::CustomTheme::default();
    for pair in spec.split(',').filter(|p| !p.is_empty()) {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| format!("--custom-theme: {pair} is not key=value"))?;
        // Every range here fits a u8 where the field is one.
        let small = |v: u16| u8::try_from(v).unwrap_or(u8::MAX);
        let number = |range: std::ops::RangeInclusive<u16>| {
            value
                .parse::<u16>()
                .ok()
                .filter(|v| range.contains(v))
                .ok_or_else(|| {
                    format!(
                        "--custom-theme: {key} takes {}-{}",
                        range.start(),
                        range.end()
                    )
                })
        };
        match key {
            "base" => {
                c.light = match value {
                    "midnight" => false,
                    "daylight" => true,
                    _ => return Err("--custom-theme: base takes midnight or daylight".into()),
                };
            }
            "accent" => c.accent_hue = number(0..=359)?,
            "tint-hue" => c.tint_hue = number(0..=359)?,
            "intensity" => c.accent_intensity = small(number(20..=140)?),
            "tint" => c.tint = small(number(0..=100)?),
            _ => return Err(format!("--custom-theme: unknown key {key}; see --help")),
        }
    }
    Ok(c)
}

/// The next argument as text, or `missing` when there is none.
fn next_text(
    args: &mut impl Iterator<Item = std::ffi::OsString>,
    missing: &str,
) -> Result<String, String> {
    args.next()
        .map(|a| a.to_string_lossy().into_owned())
        .ok_or_else(|| missing.to_owned())
}

fn parse_zoom(value: &str) -> Result<f32, String> {
    value
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite() && (0.8..=1.6).contains(v))
        .ok_or_else(|| "--zoom needs a factor from 0.8 to 1.6".to_owned())
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

/// A screenshot must use the isolated environment prepared by tools/screenshot.sh.
fn validate_screenshot_launch(launch: &Launch, isolated: bool) -> Result<(), &'static str> {
    if launch.screenshot.is_some() && !isolated {
        return Err("--screenshot requires tools/screenshot.sh (isolated home and data folders)");
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    frame_stats::mark_process_start();
    // Started by git or ssh to ask for a credential: answer through the running app and exit,
    // before touching logs or windows.
    if askpass::is_client() {
        std::process::exit(askpass::run_client());
    }
    #[expect(
        clippy::print_stderr,
        reason = "a bad command line is reported on the terminal that gave it, before any window"
    )]
    let (launch, size) = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("canopy: {e}");
            std::process::exit(2);
        }
    };

    // Reject direct screenshot runs before creating a log or opening the user's desktop.
    // The screenshot wrapper supplies an isolated home and explicitly opts in.
    let isolated = std::env::var("CANOPY_SCREENSHOT_ISOLATED").is_ok_and(|v| v == "1");
    validate_screenshot_launch(&launch, isolated).map_err(anyhow::Error::msg)?;

    // Normal detail until Settings load; their log setting takes over afterwards.
    ui::install_host();
    // Folders first: the log is written into one of them.
    storage::prepare();
    logging::init(logging::LogLevel::Normal);
    storage::log_setup();
    canopy_hosting::set_build(build_info::NUMBER);

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Canopy")
        .with_app_id("io.necturalabs.Canopy")
        .with_inner_size(size);
    if let Some(icon) = icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        persist_window: launch.screenshot.is_none() && launch.glass_soak.is_none(),
        // The window's place and size live beside the settings, in Canopy's own folder.
        persistence_path: storage::layout().map(storage::Layout::window_file),
        // Gradients and glows band without it; spelled out so a changed default cannot bring
        // the bands back.
        dithering: true,
        ..Default::default()
    };
    let result = eframe::run_native(
        "canopy",
        options,
        Box::new(move |cc| Ok(Box::new(CanopyApp::new(cc, launch)))),
    )
    .map_err(|e| anyhow::anyhow!("could not start the window: {e}"));
    // Sends any queued crash report (if reporting is on).
    crash_reports::shutdown();
    selfupdate::run_requested_restart();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_requires_explicit_isolation_but_normal_launch_does_not() {
        let mut launch = Launch::default();
        validate_screenshot_launch(&launch, false).expect("normal launch needs no isolation");
        launch.screenshot = Some(PathBuf::from("capture.png"));
        assert!(validate_screenshot_launch(&launch, false).is_err());
        validate_screenshot_launch(&launch, true).expect("isolated screenshots are allowed");
    }

    #[test]
    fn custom_theme_specs_set_their_keys_over_the_defaults() {
        use ui::custom_theme::CustomTheme;
        assert_eq!(parse_custom_theme(""), Ok(CustomTheme::default()));
        assert_eq!(
            parse_custom_theme("base=daylight,accent=150,intensity=140,tint-hue=28,tint=40"),
            Ok(CustomTheme {
                light: true,
                accent_hue: 150,
                accent_intensity: 140,
                tint_hue: 28,
                tint: 40,
            })
        );
        // A later value for the same key wins, like repeated flags.
        assert_eq!(
            parse_custom_theme("accent=10,accent=20").map(|c| c.accent_hue),
            Ok(20)
        );
        for bad in [
            "accent=360",
            "intensity=19",
            "intensity=141",
            "tint=101",
            "tint-hue=-1",
            "base=paper",
            "Base=daylight",
            "colour=red",
            "accent",
            "accent=ten",
        ] {
            assert!(parse_custom_theme(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn initial_zoom_requires_a_finite_supported_factor() {
        assert_eq!(parse_zoom("0.8"), Ok(0.8));
        assert_eq!(parse_zoom("1.6"), Ok(1.6));
        for input in ["NaN", "inf", "0", "1.7", "100%", ""] {
            assert!(parse_zoom(input).is_err(), "{input}");
        }
    }

    #[test]
    fn help_lists_every_overlay() {
        let options = HELP
            .split("--overlay <NAME>")
            .nth(1)
            .and_then(|rest| rest.split("--size").next())
            .unwrap_or_default();
        let listed: Vec<&str> = options
            .split([',', ' ', '\n'])
            .map(str::trim)
            .filter(|w| !w.is_empty() && *w != "or")
            .skip_while(|w| *w != "settings")
            .collect();
        assert_eq!(listed, app::OVERLAYS);
    }
}
