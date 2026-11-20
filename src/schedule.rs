//! `gh-watch schedule`: runs `gh-watch notify` every 30 minutes (macOS,
//! through launchd), with the gh-watch that's running now, so it's the
//! right binary however it was installed.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

const LABEL: &str = "com.gh-watch.notify";
/// Every 30 minutes, in seconds.
const INTERVAL: u32 = 1800;

fn plist_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not find your home folder")?;
    Ok(home.join(format!("Library/LaunchAgents/{LABEL}.plist")))
}

pub fn log_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not find your home folder")?;
    Ok(home.join(".local/state/gh-watch/notify.log"))
}

/// Installs (or replaces) the job and starts it.
pub fn install() -> Result<()> {
    macos_only()?;
    let program = std::env::current_exe().context("could not tell where gh-watch is")?;
    let log = log_path()?;
    std::fs::create_dir_all(log.parent().context("bad log path")?)?;
    let plist = plist_path()?;
    std::fs::create_dir_all(plist.parent().context("bad LaunchAgents path")?)?;
    std::fs::write(
        &plist,
        plist_xml(&program.display().to_string(), &log.display().to_string()),
    )
    .with_context(|| format!("could not write {}", plist.display()))?;

    // Replace a job that's already loaded (it may point at an older binary).
    let domain = domain()?;
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("{domain}/{LABEL}")])
        .output();
    let output = Command::new("launchctl")
        .args(["bootstrap", &domain])
        .arg(&plist)
        .output()
        .context("could not run launchctl")?;
    if !output.status.success() {
        bail!(
            "launchctl couldn't start the job: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Stops and removes the job, if there is one.
pub fn remove() -> Result<bool> {
    macos_only()?;
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("{}/{LABEL}", domain()?)])
        .output();
    let plist = plist_path()?;
    if !plist.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&plist)
        .with_context(|| format!("could not remove {}", plist.display()))?;
    Ok(true)
}

/// The job as launchd sees it, from `launchctl print`.
#[derive(Debug, Default, PartialEq)]
pub struct JobStatus {
    /// The gh-watch it runs.
    pub program: Option<String>,
    /// "0", or "1"; None if it hasn't exited since it was loaded.
    pub last_exit: Option<String>,
    /// "Killed: 9", if the last run was killed by a signal (launchd shows
    /// this instead of an exit code then).
    pub last_signal: Option<String>,
    /// launchd couldn't start it at all (say, the binary is gone or was
    /// replaced): `gh watch schedule` fixes that.
    pub spawn_failed: bool,
}

/// Whether the job's file is there (it may still not be loaded).
pub fn is_installed() -> bool {
    plist_path().is_ok_and(|p| p.exists())
}

/// The loaded job's status, or None if launchd doesn't have it.
pub fn status() -> Option<JobStatus> {
    let output = Command::new("launchctl")
        .arg("print")
        .arg(format!("{}/{LABEL}", domain().ok()?))
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| parse_status(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_status(text: &str) -> JobStatus {
    let mut status = JobStatus::default();
    for line in text.lines().map(str::trim) {
        if let Some(program) = line.strip_prefix("program = ") {
            status.program = Some(program.to_string());
        } else if let Some(code) = line.strip_prefix("last exit code = ") {
            // launchd reports a run it couldn't start (binary missing or
            // not executable) as EX_CONFIG; gh-watch never exits 78.
            if code.starts_with("78:") {
                status.spawn_failed = true;
            } else if code != "(never exited)" {
                status.last_exit = Some(code.to_string());
            }
        } else if let Some(signal) = line.strip_prefix("last terminating signal = ") {
            status.last_signal = Some(signal.to_string());
        } else if line.contains("spawn failed") {
            status.spawn_failed = true;
        }
    }
    status
}

fn macos_only() -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!(
            "scheduling is for macOS (launchd); on Linux, run `gh-watch notify` from cron or a systemd timer"
        );
    }
    Ok(())
}

/// The user's launchd domain: "gui/501".
fn domain() -> Result<String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("could not run id")?;
    Ok(format!(
        "gui/{}",
        String::from_utf8_lossy(&output.stdout).trim()
    ))
}

fn plist_xml(program: &str, log: &str) -> String {
    let (program, log) = (xml_escape(program), xml_escape(log));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Written by `gh-watch schedule`; `gh-watch schedule --remove` takes it out. -->
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{program}</string>
        <string>notify</string>
    </array>
    <!-- Every 30 minutes while the Mac is awake. A failed run doesn't count
         as notified, so the next one catches up. -->
    <key>StartInterval</key>
    <integer>{INTERVAL}</integer>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from real `launchctl print` output (macOS 27).
    #[test]
    fn reads_launchctl_print() {
        let fine = "gui/501/com.gh-watch.notify = {\n\tstate = not running\n\
                    \tprogram = /Users/me/.cargo/bin/gh-watch\n\tlast exit code = 0\n\
                    \tjob state = exited\n}";
        assert_eq!(
            parse_status(fine),
            JobStatus {
                program: Some(String::from("/Users/me/.cargo/bin/gh-watch")),
                last_exit: Some(String::from("0")),
                ..JobStatus::default()
            }
        );

        // The binary is gone or can't be run.
        let spawn = parse_status("\tstate = spawn scheduled\n\tlast exit code = 78: EX_CONFIG\n");
        assert!(spawn.spawn_failed);
        assert_eq!(spawn.last_exit, None);

        let killed = parse_status("\tstate = not running\n\tlast terminating signal = Killed: 9\n");
        assert_eq!(killed.last_signal.as_deref(), Some("Killed: 9"));
        assert_eq!(killed.last_exit, None);
        assert!(!killed.spawn_failed);

        let fresh = parse_status("\tlast exit code = (never exited)\n");
        assert_eq!(fresh, JobStatus::default());
    }

    #[test]
    fn plist_runs_notify_with_the_given_binary() {
        let xml = plist_xml("/Users/me/bin/gh-watch", "/tmp/log & more");
        assert!(
            xml.contains(
                "<string>/Users/me/bin/gh-watch</string>\n        <string>notify</string>"
            )
        );
        assert!(xml.contains("<integer>1800</integer>"));
        assert!(xml.contains("/tmp/log &amp; more"));
    }
}
