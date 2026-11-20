//! Desktop notifications. On macOS: gh-watch's own app (`macos/`, so they
//! show under its name and icon) if it's installed, else terminal-notifier,
//! else plain notifications through osascript (which can't open anything
//! when clicked). On Linux: notify-send.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

pub struct Note {
    pub title: String,
    pub subtitle: String,
    pub body: String,
    /// What a click opens.
    pub open: String,
    /// Notifications with the same group stack together in Notification
    /// Center (one per repo).
    pub group: String,
}

/// Set once the notifier app fails, so the rest of this run's
/// notifications go straight to the next way: if it's waiting on a
/// permission nobody answers, each try would wait out its timeout again.
static APP_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn send(note: &Note) -> Result<()> {
    if cfg!(target_os = "linux") {
        let status = Command::new("notify-send")
            .args(["--app-name", "gh-watch"])
            .arg(format!("{}: {}", note.title, note.subtitle))
            .arg(&note.body)
            .status()
            .context("could not run notify-send")?;
        if !status.success() {
            bail!("notify-send failed ({status})");
        }
        return Ok(());
    }

    if !APP_FAILED.load(std::sync::atomic::Ordering::Relaxed)
        && let Some(program) = app()
    {
        let status = Command::new(program)
            .args(["--title", &note.title, "--subtitle", &note.subtitle])
            .args([
                "--message",
                &note.body,
                "--open",
                &note.open,
                "--group",
                &note.group,
            ])
            .status();
        match status {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => eprintln!("gh-watch app failed ({status}), trying the next way"),
            Err(err) => eprintln!("could not run the gh-watch app ({err}), trying the next way"),
        }
        APP_FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    if let Some(program) = terminal_notifier() {
        // terminal-notifier treats a message starting with "-" or "[" as
        // special, so escape it.
        let body = if note.body.starts_with(['-', '[']) {
            format!("\\{}", note.body)
        } else {
            note.body.clone()
        };
        let status = Command::new(program)
            .args(["-title", &note.title, "-subtitle", &note.subtitle])
            .args(["-message", &body, "-open", &note.open, "-group", &note.open])
            .args(["-sound", "Glass"])
            .status();
        match status {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => eprintln!("terminal-notifier failed ({status}), using osascript"),
            Err(err) => eprintln!("could not run terminal-notifier ({err}), using osascript"),
        }
    }

    let status = Command::new("osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 1 of argv) with title (item 2 of argv) \
             subtitle (item 3 of argv) sound name \"Glass\"",
            "-e",
            "end run",
            &note.body,
            &note.title,
            &note.subtitle,
        ])
        .status()
        .context("could not run osascript")?;
    if !status.success() {
        bail!("osascript failed ({status})");
    }
    Ok(())
}

/// The gh-watch app's program, if it's installed (`macos/build.sh --install`).
fn app() -> Option<PathBuf> {
    let program = "gh-watch.app/Contents/MacOS/gh-watch-notifier";
    dirs::home_dir()
        .map(|home| home.join("Applications"))
        .into_iter()
        .chain([PathBuf::from("/Applications")])
        .map(|dir| dir.join(program))
        .find(|program| program.is_file())
}

/// Where terminal-notifier is, if installed. Homebrew's folders are checked
/// by name since scheduled runs don't have them on PATH.
fn terminal_notifier() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ])
        .map(|dir| dir.join("terminal-notifier"))
        .find(|program| program.is_file())
}
