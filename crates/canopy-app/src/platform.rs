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

use std::path::Path;
use std::process::Command;

/// Spawns `cmd` and waits for it on a background thread.
fn spawn(mut cmd: Command) -> std::io::Result<()> {
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
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
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
        assert!(!web_address("--help"));
        assert!(!web_address("https://a\nb"));
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
