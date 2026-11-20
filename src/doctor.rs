//! `gh watch doctor`: checks what notifications need, one line each, with
//! the fix for anything wrong, then sends a test notification and says how
//! it went out. Mostly for "I got a notification without the cat, and
//! clicking it did nothing": that's the app not being used.

use std::path::Path;
use std::process::Command;

use chrono::{DateTime, Local, TimeDelta, Utc};

use crate::config::Config;
use crate::desktop::{self, Note, Way};
use crate::{app, schedule};
use gh_watch::gh;

/// What's wrong, counted, so the end can say "all good" or not.
#[derive(Default)]
struct Report {
    problems: usize,
}

impl Report {
    fn ok(&self, what: &str, detail: &str) {
        println!("  ✓ {what}: {detail}");
    }

    fn problem(&mut self, what: &str, detail: &str, fix: &str) {
        self.problems += 1;
        println!("  ! {what}: {detail}\n      Fix: {fix}");
    }
}

/// A last check older than this, with the schedule on, means runs have
/// stopped or are failing (they're every 30 minutes, and a sleeping Mac
/// catches up when it wakes).
const STALE_AFTER: TimeDelta = TimeDelta::hours(3);

/// Runs the checks and returns how many problems it found.
pub fn run(last_notified: Option<DateTime<Utc>>, send_test: bool) -> usize {
    println!("Checking gh-watch:");
    let mut report = Report::default();

    match gh::me() {
        Ok(me) => report.ok("GitHub", &format!("gh is logged in as {me}")),
        Err(err) => report.problem(
            "GitHub",
            &format!("{err:#}"),
            "install gh (https://cli.github.com) and run `gh auth login`",
        ),
    }

    match Config::exists().and_then(|exists| exists.then(Config::load).transpose()) {
        Ok(Some(config)) if !config.repos.is_empty() => {
            report.ok("Config", &format!("watching {}", config.repos.join(", ")));
        }
        Ok(_) => report.problem(
            "Config",
            "no repos to watch yet",
            "run `gh watch setup` inside the repo to watch",
        ),
        Err(err) => report.problem(
            "Config",
            &format!("{err:#}"),
            "fix or delete the file, then run `gh watch setup`",
        ),
    }

    let mut app_ok = true;
    let mut scheduled = false;
    if cfg!(target_os = "macos") {
        app_ok = check_app(&mut report);
        check_schedule(&mut report);
        scheduled = schedule::is_installed();
    }

    match last_notified {
        Some(at) => {
            let when = at.with_timezone(&Local).format("%a %b %-d, %-I:%M %p");
            let ago = Utc::now() - at;
            if scheduled && ago > STALE_AFTER {
                report.problem(
                    "Last check",
                    &format!(
                        "{when}, {} ago, though the schedule should check every 30 minutes",
                        how_long(ago)
                    ),
                    "run `gh watch notify` to see what fails",
                );
            } else {
                report.ok("Last check", &when.to_string());
            }
        }
        None => report.problem(
            "Last check",
            "never: the clock hasn't started",
            "run `gh watch setup` (or `gh watch notify` once)",
        ),
    }

    if send_test {
        send_test_note(&mut report, app_ok);
    }

    if report.problems == 0 {
        println!("\nAll good.");
    } else {
        let things = if report.problems == 1 {
            "thing"
        } else {
            "things"
        };
        println!("\n{} {things} to fix, above.", report.problems);
    }
    report.problems
}

/// "5 days", "7 hours".
fn how_long(span: TimeDelta) -> String {
    let (n, unit) = if span.num_days() >= 1 {
        (span.num_days(), "day")
    } else {
        (span.num_hours(), "hour")
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// The app: there, and allowed to notify. Returns whether it all checked out.
fn check_app(report: &mut Report) -> bool {
    let before = report.problems;
    let Some(program) = desktop::app() else {
        let missing = app::missing_tools();
        let fix = if missing.is_empty() {
            String::from("run `gh watch install-app`")
        } else {
            format!(
                "install Apple's command line tools (`xcode-select --install`; missing: {}), \
                 then run `gh watch install-app`",
                missing.join(", ")
            )
        };
        report.problem(
            "App",
            "not installed, so notifications have no cat icon and clicks may open nothing",
            &fix,
        );
        return false;
    };
    let app = program
        .ancestors()
        .nth(3)
        .unwrap_or(&program)
        .display()
        .to_string();
    report.ok("App", &format!("installed at {app}"));

    // Ask the app itself; it says without posting anything.
    let output = Command::new(&program).arg("--check").output();
    match output {
        Ok(out) if out.status.success() => {
            report.ok("Permission", "macOS allows gh-watch to notify");
        }
        Ok(out) if out.status.code() == Some(2) => {
            let answer = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let fix = if answer == "not asked yet" {
                "run `gh watch install-app`, and click Allow when macOS asks"
            } else if answer == "delivered quietly" {
                "System Settings → Notifications → gh-watch → set the alert style to Banners"
            } else {
                "System Settings → Notifications → gh-watch → turn on Allow Notifications"
            };
            report.problem(
                "Permission",
                &format!("notifications for gh-watch are {answer}"),
                fix,
            );
        }
        // An app from before `--check` existed refuses the option.
        Ok(out) if out.status.code() == Some(1) => report.problem(
            "Permission",
            "the app is too old to say",
            "run `gh watch install-app` to update it, then `gh watch doctor` again",
        ),
        Ok(out) => report.problem(
            "Permission",
            &format!("couldn't tell ({}{})", out.status, stderr_note(&out.stderr)),
            "run `gh watch install-app`",
        ),
        Err(err) => report.problem(
            "Permission",
            &format!("couldn't run the app ({err})"),
            "run `gh watch install-app`",
        ),
    }

    if Command::new("codesign")
        .args(["--verify", "--deep"])
        .arg(&app)
        .output()
        .is_ok_and(|o| !o.status.success())
    {
        report.problem(
            "Signature",
            "the app's signature doesn't check out, so macOS may refuse it",
            "run `gh watch install-app`",
        );
    }
    report.problems == before
}

/// The launchd job: there, running this gh-watch, and its last run fine.
fn check_schedule(report: &mut Report) {
    let fix = "run `gh watch schedule`";
    if !schedule::is_installed() {
        report.problem(
            "Schedule",
            "not set up, so nothing checks every 30 minutes",
            fix,
        );
        return;
    }
    let Some(job) = schedule::status() else {
        report.problem("Schedule", "set up, but not loaded in launchd", fix);
        return;
    };
    if job.spawn_failed {
        report.problem(
            "Schedule",
            "launchd couldn't start it (often after gh-watch was reinstalled)",
            fix,
        );
        return;
    }
    let this = std::env::current_exe().ok();
    let runs_this = match (&job.program, &this) {
        (Some(program), Some(this)) => same_file(Path::new(program), this),
        _ => true,
    };
    if !runs_this {
        report.problem(
            "Schedule",
            &format!(
                "runs {}, not this gh-watch",
                job.program.as_deref().unwrap_or("?")
            ),
            fix,
        );
        return;
    }
    let log = schedule::log_path()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    match (job.last_exit.as_deref(), job.last_signal.as_deref()) {
        (Some("0"), _) => report.ok("Schedule", "every 30 minutes; the last run went fine"),
        (Some(code), _) => report.problem(
            "Schedule",
            &format!("the last run failed (exit {code})"),
            &format!("see the end of {log}"),
        ),
        (None, Some(signal)) => report.problem(
            "Schedule",
            &format!("the last run was killed ({signal})"),
            &format!("see the end of {log}, and run `gh watch notify` to try it"),
        ),
        (None, None) => report.ok("Schedule", "every 30 minutes (no run since it was loaded)"),
    }
}

/// Sends one notification and says which way it went out. `app_ok`: the
/// app checks above all passed.
fn send_test_note(report: &mut Report, app_ok: bool) {
    println!("\nSending a test notification...");
    let note = Note {
        title: String::from("gh-watch"),
        subtitle: String::new(),
        body: String::from("Test from `gh watch doctor`. Click to open gh-watch's page."),
        open: String::from("https://github.com/j-dominguez9/gh-watch"),
        group: String::from("gh-watch"),
    };
    match desktop::send(&note) {
        Ok(Way::App) | Ok(Way::NotifySend) => {
            report.ok(
                "Test",
                "sent: you should see it now, and a click opens gh-watch's page",
            );
        }
        Ok(way) => match desktop::app_error().filter(|_| app_ok) {
            // The app looked fine but failed when it was used.
            Some(error) => report.problem(
                "Test",
                &format!(
                    "sent with {}, because the app failed: {error}",
                    way.describe()
                ),
                "run `gh watch install-app` and click Allow if macOS asks, then \
                 `gh watch doctor` again",
            ),
            None => report.problem(
                "Test",
                &format!("sent with {}", way.describe()),
                "fix the app lines above",
            ),
        },
        Err(err) => report.problem(
            "Test",
            &format!("couldn't send it ({err:#})"),
            "fix the lines above",
        ),
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn stderr_note(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr).trim().to_string();
    if text.is_empty() {
        text
    } else {
        format!(": {text}")
    }
}
