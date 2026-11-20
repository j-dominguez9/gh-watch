//! Desktop notifications. On macOS: gh-watch's own app (`macos/`, so they
//! show under its name and icon) if it's installed, else terminal-notifier,
//! else plain notifications through osascript (which can't open anything
//! when clicked). On Linux: notify-send.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

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

/// Set once the notifier app fails in a way that would happen again (not
/// allowed, timed out, crashed, couldn't run), so the rest of this run's
/// notifications go straight to the next way: if it's waiting on a
/// permission nobody answers, each try would wait out its timeout again.
/// Bad arguments (exit 1) don't set it: that's about one note, not the app.
static APP_FAILED: AtomicBool = AtomicBool::new(false);

/// Why the app last failed to send, for `gh watch doctor`.
static APP_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Why the app failed the last time it was tried, if it did.
pub fn app_error() -> Option<String> {
    APP_ERROR.lock().ok().and_then(|e| e.clone())
}

/// How a notification went out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Way {
    App,
    TerminalNotifier,
    Osascript,
    NotifySend,
}

impl Way {
    pub fn describe(self) -> &'static str {
        match self {
            Way::App => "the gh-watch app",
            Way::TerminalNotifier => "terminal-notifier (no cat icon, but clicks open the issue)",
            Way::Osascript => "plain macOS notifications (no cat icon, and clicks open nothing)",
            Way::NotifySend => "notify-send",
        }
    }

    /// A fallback on macOS, missing what the app gives.
    pub fn is_fallback(self) -> bool {
        matches!(self, Way::TerminalNotifier | Way::Osascript)
    }
}

pub fn send(note: &Note) -> Result<Way> {
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
        return Ok(Way::NotifySend);
    }

    if !APP_FAILED.load(Ordering::Relaxed)
        && let Some(program) = app()
    {
        let output = Command::new(program)
            .args(["--title", &note.title, "--subtitle", &note.subtitle])
            .args([
                "--message",
                &note.body,
                "--open",
                &note.open,
                "--group",
                &note.group,
            ])
            .output();
        let (error, again) = match output {
            Ok(out) if out.status.success() => {
                if let Ok(mut e) = APP_ERROR.lock() {
                    *e = None;
                }
                return Ok(Way::App);
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let error = if stderr.is_empty() {
                    format!("{}", out.status)
                } else {
                    format!("{}: {stderr}", out.status)
                };
                // Exit 1 is bad arguments, about this note only.
                (error, out.status.code() != Some(1))
            }
            Err(err) => (format!("could not run it: {err}"), true),
        };
        eprintln!("gh-watch app failed ({error}), trying the next way");
        if again {
            APP_FAILED.store(true, Ordering::Relaxed);
        }
        if let Ok(mut e) = APP_ERROR.lock() {
            *e = Some(error);
        }
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
            Ok(status) if status.success() => return Ok(Way::TerminalNotifier),
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
    Ok(Way::Osascript)
}

/// The gh-watch app's program, if it's installed (`macos/build.sh --install`).
pub fn app() -> Option<PathBuf> {
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
pub fn terminal_notifier() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ])
        .map(|dir| dir.join("terminal-notifier"))
        .find(|program| program.is_file())
}
