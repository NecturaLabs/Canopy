//! Handing things to the operating system: web pages, folders, files in an editor, a terminal.
//!
//! Each kind of target has its own function, because each comes from a different source and
//! carries a different risk:
//!
//! - [`open_url`] takes web addresses only. API responses and the update feed reach it, so a
//!   server must not be able to make Canopy open a local file or a custom URL handler.
//! - [`open_folder`] shows a folder in the file manager.
//! - [`open_in_editor`] opens a file for editing. Files come from repositories, which can be
//!   anyone's, so a file is never handed to a launcher that might run it (Explorer runs `.cmd`,
//!   macOS `open` runs `.command`, desktop launchers run `.desktop` entries).
//!
//! Started programs are reaped on a background thread, so none is left as a zombie.
//!
//! It also asks the operating system for settings Canopy follows: reduced motion
//! ([`prefers_reduced_motion`]) and reduced transparency ([`prefers_reduced_transparency`]).

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Spawns `cmd` and waits for it on a background thread. The askpass port and token never reach
/// these programs, even when Canopy itself inherited them.
fn spawn(mut cmd: Command) -> std::io::Result<()> {
    for var in crate::askpass::CLIENT_ENV {
        cmd.env_remove(var);
    }
    let mut child = cmd.spawn()?;
    let _ = std::thread::Builder::new()
        .name("reap".into())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

/// Whether `url` may be opened in the browser: `https://`, or `http://` (a self-hosted GitLab
/// without TLS), with no control characters and something after the scheme.
pub fn web_address(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    rest.is_some_and(|r| !r.is_empty()) && !url.chars().any(char::is_control)
}

/// Opens a web page in the default browser.
#[cfg_attr(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    expect(
        clippy::disallowed_methods,
        reason = "starts the platform's URL opener, never git"
    )
)]
pub fn open_url(url: &str) -> Result<(), String> {
    if !web_address(url) {
        tracing::warn!("refused to open a non-web address");
        return Err("Only http:// and https:// addresses can be opened.".into());
    }
    #[cfg(target_os = "linux")]
    let cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "macos")]
    let cmd = {
        let mut c = Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "windows")]
    let cmd = {
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    return Err("unsupported platform".into());
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    spawn(cmd).map_err(|e| e.to_string())
}

/// Shows `dir` in the file manager.
#[cfg_attr(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    expect(
        clippy::disallowed_methods,
        reason = "starts the platform's file manager, never git"
    )
)]
pub fn open_folder(dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!("{} is not a folder", dir.display()));
    }
    #[cfg(target_os = "linux")]
    let program = "xdg-open";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    return Err("unsupported platform".into());
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        let mut cmd = Command::new(program);
        cmd.arg(dir);
        spawn(cmd).map_err(|e| e.to_string())
    }
}

/// The program and arguments of a configured command such as `code --wait`.
fn split_command(configured: &str) -> Option<(String, Vec<String>)> {
    let mut parts = configured.split_whitespace().map(str::to_owned);
    let program = parts.next()?;
    Some((program, parts.collect()))
}

/// Whether the desktop's default handler might run `file` instead of showing it.
#[cfg_attr(
    not(any(target_os = "linux", test)),
    expect(
        dead_code,
        reason = "only Linux opens files with the desktop's handler"
    )
)]
fn launchable(file: &Path) -> bool {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if matches!(ext.as_str(), "desktop" | "appimage" | "run" | "bin") {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(file).is_ok_and(|m| m.permissions().mode() & 0o111 != 0) {
            return true;
        }
    }
    false
}

/// Opens `file` for editing: with the editor configured in Settings when there is one,
/// otherwise with the platform's text editor (TextEdit's default on macOS, Notepad on Windows,
/// the desktop's handler on Linux unless it could run the file, in which case the folder opens).
#[expect(
    clippy::disallowed_methods,
    reason = "starts the configured or the platform's editor, never git"
)]
pub fn open_in_editor(file: &Path, configured: &str) -> Result<(), String> {
    if let Some((program, args)) = split_command(configured.trim()) {
        let mut cmd = Command::new(&program);
        cmd.args(args).arg(file);
        return spawn(cmd).map_err(|e| format!("{program}: {e}"));
    }
    #[cfg(target_os = "macos")]
    {
        // `-t` opens with the default text editor, never by running the file.
        let mut cmd = Command::new("open");
        cmd.arg("-t").arg(file);
        spawn(cmd).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("notepad");
        cmd.arg(file);
        spawn(cmd).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "linux")]
    {
        if launchable(file) {
            return match file.parent() {
                Some(dir) => open_folder(dir).and(Err(
                    "This file could be run rather than opened, so its folder was opened instead. \
                     Set an editor in Settings → Integrations to open it directly."
                        .into(),
                )),
                None => Err("No editor configured.".into()),
            };
        }
        let mut cmd = Command::new("xdg-open");
        cmd.arg(file);
        spawn(cmd).map_err(|e| e.to_string())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err("unsupported platform".into())
    }
}

/// Opens a terminal in `dir`: the configured command first, then the platform's usual ones.
#[expect(
    clippy::disallowed_methods,
    reason = "starts the configured or the platform's terminal, never git"
)]
pub fn open_terminal(dir: &Path, configured: &str) -> Result<(), String> {
    if let Some((program, args)) = split_command(configured.trim()) {
        let mut cmd = Command::new(&program);
        cmd.args(args).current_dir(dir);
        return spawn(cmd).map_err(|e| format!("{program}: {e}"));
    }
    #[cfg(target_os = "linux")]
    {
        let candidates: Vec<String> = std::env::var("TERMINAL")
            .ok()
            .into_iter()
            .chain(
                [
                    "x-terminal-emulator",
                    "kgx",
                    "gnome-terminal",
                    "konsole",
                    "alacritty",
                    "kitty",
                    "foot",
                    "wezterm",
                    "ghostty",
                    "xterm",
                ]
                .map(String::from),
            )
            .collect();
        for term in candidates {
            let mut cmd = Command::new(&term);
            cmd.current_dir(dir);
            if spawn(cmd).is_ok() {
                return Ok(());
            }
        }
        Err("No terminal emulator found. Set one in Settings → Integrations.".into())
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
        cmd.args(["-a", "Terminal"]).arg(dir);
        spawn(cmd).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "cmd"]).current_dir(dir);
        spawn(cmd).map_err(|e| e.to_string())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = dir;
        Err("unsupported platform".into())
    }
}

/// How long a settings query may take before Canopy gives up on it. PowerShell loading .NET on
/// a cold start is the slowest case.
const QUERY_TIMEOUT: Duration = Duration::from_secs(6);

/// Asks the operating system whether the user wants reduced motion. Blocks while a helper
/// program answers (up to a few seconds), so call it off the UI thread. `None` when the system
/// does not say.
#[cfg_attr(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    expect(
        clippy::disallowed_methods,
        reason = "asks the platform's settings tools, never git"
    )
)]
pub fn prefers_reduced_motion() -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        // The desktop portal works across GNOME, KDE and others; older GNOME only has gsettings.
        let mut portal = Command::new("gdbus");
        portal.args([
            "call",
            "--session",
            "--timeout",
            "2",
            "--dest",
            "org.freedesktop.portal.Desktop",
            "--object-path",
            "/org/freedesktop/portal/desktop",
            "--method",
            "org.freedesktop.portal.Settings.ReadOne",
            "org.freedesktop.appearance",
            "reduced-motion",
        ]);
        output_within(portal, QUERY_TIMEOUT)
            .and_then(|out| parse_portal(&out))
            .or_else(|| {
                let mut gsettings = Command::new("gsettings");
                gsettings.args(["get", "org.gnome.desktop.interface", "enable-animations"]);
                output_within(gsettings, QUERY_TIMEOUT).and_then(|out| parse_gsettings(&out))
            })
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("defaults");
        cmd.args(["read", "com.apple.universalaccess", "reduceMotion"]);
        output_within(cmd, QUERY_TIMEOUT).and_then(|out| parse_defaults(&out))
    }
    #[cfg(target_os = "windows")]
    {
        // "Show animations in Windows" (SPI_GETCLIENTAREAANIMATION), read through .NET so Canopy
        // needs no unsafe system call.
        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Add-Type -AssemblyName PresentationFramework; \
             [System.Windows.SystemParameters]::ClientAreaAnimation",
        ]);
        output_within(cmd, QUERY_TIMEOUT).and_then(|out| parse_windows(&out))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// Asks the operating system whether the user wants reduced transparency (macOS "Reduce
/// transparency", Windows "Transparency effects" off). Blocks while a helper program answers, so
/// call it off the UI thread. `None` when the system does not say; Linux desktops have no
/// standard setting.
#[cfg_attr(
    any(target_os = "macos", target_os = "windows"),
    expect(
        clippy::disallowed_methods,
        reason = "asks the platform's settings tools, never git"
    )
)]
pub fn prefers_reduced_transparency() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("defaults");
        cmd.args(["read", "com.apple.universalaccess", "reduceTransparency"]);
        output_within(cmd, QUERY_TIMEOUT).and_then(|out| parse_defaults(&out))
    }
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("reg");
        cmd.args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            "EnableTransparency",
        ]);
        output_within(cmd, QUERY_TIMEOUT)
            .and_then(|out| parse_reg_dword(&out, "EnableTransparency"))
            .map(|enabled| enabled == 0)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// Runs `cmd` and returns its standard output when it exits successfully within `timeout`.
/// Output is read on its own thread, so a chatty program cannot fill the pipe and stall, and a
/// program that outlives the timeout is killed and reaped.
fn output_within(mut cmd: Command, timeout: Duration) -> Option<String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    // Ends when the pipe closes: when the program and anything it started have exited, or
    // after the kill below.
    let reader = std::thread::Builder::new()
        .name("query-output".into())
        .spawn(move || {
            let mut out = String::new();
            let ok = stdout.read_to_string(&mut out).is_ok();
            let _ = tx.send(ok.then_some(out));
        });
    if reader.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    if let Ok(out) = rx.recv_timeout(timeout) {
        let status = child.wait().ok()?;
        out.filter(|_| status.success())
    } else {
        let _ = child.kill();
        let _ = child.wait();
        tracing::debug!("system settings query timed out");
        None
    }
}

/// `(<uint32 1>,)` from the portal: 1 asks for reduced motion.
#[cfg_attr(
    not(any(target_os = "linux", test)),
    expect(
        dead_code,
        reason = "only the Linux query and the tests read this answer"
    )
)]
fn parse_portal(out: &str) -> Option<bool> {
    let value = out
        .trim()
        .trim_start_matches("(<")
        .trim_end_matches(">,)")
        .trim_start_matches("uint32")
        .trim();
    match value {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

/// `true` or `false` for `enable-animations`; disabled animations mean reduced motion.
#[cfg_attr(
    not(any(target_os = "linux", test)),
    expect(
        dead_code,
        reason = "only the Linux query and the tests read this answer"
    )
)]
fn parse_gsettings(out: &str) -> Option<bool> {
    match out.trim() {
        "true" => Some(false),
        "false" => Some(true),
        _ => None,
    }
}

/// `1` or `0` from `defaults read`.
#[cfg_attr(
    not(any(target_os = "macos", test)),
    expect(
        dead_code,
        reason = "only the macOS query and the tests read this answer"
    )
)]
fn parse_defaults(out: &str) -> Option<bool> {
    match out.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

/// `True` or `False` for client-area animation; animations off mean reduced motion.
#[cfg_attr(
    not(any(target_os = "windows", test)),
    expect(
        dead_code,
        reason = "only the Windows query and the tests read this answer"
    )
)]
fn parse_windows(out: &str) -> Option<bool> {
    match out.trim().to_ascii_lowercase().as_str() {
        "true" => Some(false),
        "false" => Some(true),
        _ => None,
    }
}

/// The value of `name` in `reg query` output: `    EnableTransparency    REG_DWORD    0x1`.
#[cfg_attr(
    not(any(target_os = "windows", test)),
    expect(
        dead_code,
        reason = "only the Windows query and the tests read this answer"
    )
)]
fn parse_reg_dword(out: &str, name: &str) -> Option<u32> {
    out.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        if words.next()? != name || words.next()? != "REG_DWORD" {
            return None;
        }
        let value = words.next()?;
        u32::from_str_radix(value.strip_prefix("0x")?, 16).ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_addresses_open_in_the_browser() {
        assert!(web_address("https://github.com/o/r/pull/1"));
        assert!(web_address("HTTPS://gitlab.com/o/r"));
        assert!(web_address("http://gitlab.internal/group/repo"));
        assert!(!web_address("https://"));
        assert!(!web_address("file:///etc/passwd"));
        assert!(!web_address("/home/someone/run.sh"));
        assert!(!web_address("C:\\Windows\\evil.cmd"));
        assert!(!web_address("javascript:alert(1)"));
        assert!(!web_address("vscode://open?x"));
        assert!(!web_address(" https://github.com/o/r"));
        assert!(!web_address("ssh://git@github.com/o/r.git"));
        assert!(!web_address("https://github.com/\u{7f}"));
        assert!(!web_address("--help"));
        assert!(!web_address("https://a\nb"));
    }

    #[test]
    fn system_answers_are_read() {
        assert_eq!(parse_portal("(<uint32 1>,)\n"), Some(true));
        assert_eq!(parse_portal("(<uint32 0>,)"), Some(false));
        assert_eq!(parse_portal("Error: no such key"), None);
        assert_eq!(parse_gsettings("false\n"), Some(true));
        assert_eq!(parse_gsettings("true"), Some(false));
        assert_eq!(parse_defaults("1\n"), Some(true));
        assert_eq!(
            parse_defaults("The domain/default pair does not exist"),
            None
        );
        assert_eq!(parse_windows("False\r\n"), Some(true));
        assert_eq!(parse_windows("True"), Some(false));
        let reg = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\r\n    EnableTransparency    REG_DWORD    0x0\r\n\r\n";
        assert_eq!(parse_reg_dword(reg, "EnableTransparency"), Some(0));
        assert_eq!(
            parse_reg_dword(
                "    EnableTransparency    REG_DWORD    0x1",
                "EnableTransparency"
            ),
            Some(1)
        );
        assert_eq!(
            parse_reg_dword(
                "    EnableTransparency    REG_SZ    yes",
                "EnableTransparency"
            ),
            None
        );
        assert_eq!(
            parse_reg_dword(
                "ERROR: The system was unable to find the specified registry key",
                "EnableTransparency"
            ),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test runs sh to stand in for a settings tool"
    )]
    fn slow_or_failing_queries_give_up() {
        let mut ok = Command::new("sh");
        ok.args(["-c", "echo hello"]);
        assert_eq!(
            output_within(ok, Duration::from_secs(5)).as_deref(),
            Some("hello\n")
        );

        let mut fails = Command::new("sh");
        fails.args(["-c", "echo partial; exit 3"]);
        assert_eq!(output_within(fails, Duration::from_secs(5)), None);

        let started = std::time::Instant::now();
        let mut slow = Command::new("sh");
        slow.args(["-c", "sleep 30"]);
        assert_eq!(output_within(slow, Duration::from_millis(200)), None);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the slow query was not killed"
        );

        // Lots of output must not stall the program on a full pipe.
        let mut chatty = Command::new("sh");
        chatty.args(["-c", "yes canopy | head -c 1000000"]);
        let out = output_within(chatty, Duration::from_secs(10));
        assert_eq!(out.map(|o| o.len()), Some(1_000_000));
    }

    #[test]
    fn configured_commands_split_into_program_and_arguments() {
        assert_eq!(
            split_command("code --wait"),
            Some(("code".into(), vec!["--wait".into()]))
        );
        assert_eq!(split_command("   "), None);
    }

    #[test]
    fn files_that_could_run_are_recognised() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let text = dir.path().join("notes.md");
        std::fs::write(&text, "x")?;
        assert!(!launchable(&text));
        assert!(launchable(&dir.path().join("app.desktop")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = dir.path().join("setup");
            std::fs::write(&script, "#!/bin/sh")?;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
            assert!(launchable(&script));
        }
        Ok(())
    }
}
