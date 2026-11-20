//! gh-watch: know when GitHub issues you care about change. Works as a
//! command (`gh-watch`) or a gh extension (`gh watch`).
//!
//! It watches issues you're involved in (opened, assigned, mentioned,
//! commented on) and, optionally, issues with given labels, and reports
//! only real changes: closed, or commented on by someone other than you or
//! a bot.

mod app;
mod config;
mod desktop;
mod doctor;
mod schedule;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use clap::{Parser, Subcommand};
use gh_watch::changes::{self, Change, Watch};
use gh_watch::gh;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::desktop::Note;

/// At most this many notifications per run, the last a summary when
/// there are more.
const MAX_NOTIFICATIONS: usize = 5;
/// How much of a comment a line shows.
const PREVIEW_CHARS: usize = 160;

#[derive(Parser)]
#[command(
    version,
    about = "Know when GitHub issues you care about change: closed, or commented on by someone else"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
    /// Repo to watch, as owner/name (repeat for several). Default: the
    /// config's repos, else the repo you're in.
    #[arg(long = "repo", short = 'R', global = true)]
    repos: Vec<String>,
    /// Also watch issues with this label (repeat for several).
    #[arg(long = "label", short = 'l', global = true)]
    labels: Vec<String>,
    /// Leave out issues you're involved in (opened, assigned, mentioned,
    /// commented), so only labels are watched.
    #[arg(long, global = true)]
    no_involved: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// List what changed, e.g. `--since 1d` (also 30m, 12h, 2w).
    Changes {
        #[arg(long, default_value = "1d")]
        since: String,
    },
    /// Set everything up in one go: a config with the repo (the one you're
    /// in, or --repo), the notifier app (macOS), notifications every 30
    /// minutes, and the clock started.
    Setup,
    /// Build and install gh-watch's notifier app (macOS), so notifications
    /// show under its own name and icon. Needs Apple's command line tools.
    InstallApp,
    /// Show what's watched, or change it: `config add --label customer-acme`
    /// or `config remove --label customer-acme` (also `--repo owner/name`).
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// Check that notifications will work, with the fix for anything that
    /// won't: gh, the config, the app and its permission (macOS), and the
    /// schedule. Then send a test notification and say how it went out.
    Doctor {
        /// Skip the test notification.
        #[arg(long)]
        no_test: bool,
    },
    /// Run `notify` every 30 minutes (macOS, through launchd).
    Schedule {
        /// Stop and remove the schedule instead.
        #[arg(long)]
        remove: bool,
    },
    /// Notify about what changed since the last time (one notification per
    /// issue). For a schedule; the first run only starts the clock.
    Notify {
        /// Notify about this window instead (e.g. 2d), leaving the saved
        /// "last notified" time alone: for trying it out. These are real
        /// notifications, and the next scheduled run may send them again;
        /// add --dry-run to only print them.
        #[arg(long)]
        since: Option<String>,
        /// Print what would be sent instead of sending it. Changes nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Start watching the `--label`s and `--repo`s given.
    Add,
    /// Stop watching the `--label`s and `--repo`s given.
    Remove,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    // Commands that don't need to know what to watch.
    match cli.command {
        Cmd::InstallApp => {
            let path = app::install()?;
            println!(
                "Installed {}. Allow notifications if macOS asks.",
                path.display()
            );
            return Ok(());
        }
        Cmd::Config { ref action } => return config_command(&cli, action.as_ref()),
        Cmd::Doctor { no_test } => {
            // Non-zero when something's wrong, for scripts.
            if doctor::run(load_state()?.last_notified, !no_test) > 0 {
                std::process::exit(1);
            }
            return Ok(());
        }
        Cmd::Schedule { remove: true } => {
            let removed = schedule::remove()?;
            println!(
                "{}",
                if removed {
                    "Schedule removed."
                } else {
                    "There was no schedule."
                }
            );
            return Ok(());
        }
        Cmd::Schedule { remove: false } => {
            schedule::install()?;
            println!(
                "gh-watch notify now runs every 30 minutes (log: {}).",
                schedule::log_path()?.display()
            );
            return Ok(());
        }
        _ => {}
    }

    let config = Config::load()?;
    let repos = repos(&cli, &config)?;
    let watch = Watch {
        labels: if cli.labels.is_empty() {
            config.labels.clone()
        } else {
            cli.labels.clone()
        },
        involved: config.involved && !cli.no_involved,
    };
    if watch.labels.is_empty() && !watch.involved {
        bail!("nothing to watch: add labels, or leave issues you're involved in on");
    }

    match cli.command {
        Cmd::Changes { since } => {
            let since = ago(parse_duration(&since)?)?;
            let found = gather(&repos, &watch, since)?;
            print(&found, since);
            Ok(())
        }
        Cmd::Notify { since, dry_run } => {
            let window = since.as_deref().map(parse_duration).transpose()?;
            notify(&repos, &watch, window, dry_run)
        }
        Cmd::Setup => setup(&cli, &repos),
        Cmd::InstallApp | Cmd::Config { .. } | Cmd::Doctor { .. } | Cmd::Schedule { .. } => {
            unreachable!("handled above")
        }
    }
}

/// `gh-watch setup`: each step says what it did, and a step that fails
/// (say, no command line tools for the app) doesn't stop the rest.
///
/// Run again, it adds to the config rather than replacing it: `gh watch
/// setup --label customer-acme` starts watching another label, keeping
/// everything already there.
fn setup(cli: &Cli, repos: &[String]) -> Result<()> {
    println!("Setting up gh-watch for {}:", repos.join(", "));

    // 1. Remember what to watch, so scheduled runs (which start outside
    //    any repo, with no flags) know.
    let config_path = Config::path()?;
    let existed = Config::exists()?;
    let mut config = Config::load()?;
    let mut changed = config.add(repos, &cli.labels);
    if cli.no_involved && config.involved {
        config.involved = false;
        changed = true;
    }
    if changed || !existed {
        config.save()?;
        let did = if existed { "updated" } else { "wrote" };
        println!("  ✓ Config: {did} {}", config_path.display());
    } else {
        println!("  ✓ Config: keeping your {}", config_path.display());
    }
    check_labels(&config, &cli.labels);

    // 2. The notifier app, on macOS.
    if cfg!(target_os = "macos") {
        match app::install() {
            Ok(path) => println!(
                "  ✓ App: installed {}. Allow notifications if macOS asks.",
                path.display()
            ),
            Err(err) => println!(
                "  ! App: not installed ({err:#}).\n    Notifications will still come, but \
                 without the cat icon, and clicking one won't open the issue (unless \
                 terminal-notifier is installed). Fix that first, then run \
                 `gh watch install-app`; `gh watch doctor` checks everything."
            ),
        }
    }

    // 3. Every 30 minutes.
    if cfg!(target_os = "macos") {
        schedule::install()?;
        println!(
            "  ✓ Schedule: notify runs every 30 minutes (log: {})",
            schedule::log_path()?.display()
        );
    } else {
        println!("  - Schedule: on Linux, run `gh-watch notify` from cron or a systemd timer.");
    }

    // 4. Start the clock, so the first scheduled run only reports what's new.
    let mut state = load_state()?;
    if state.last_notified.is_none() {
        state.last_notified = Some(Utc::now());
        save_state(&state)?;
    }
    println!(
        "  ✓ Watching {}: {}, from now on.",
        config.repos.join(", "),
        config.describe()
    );
    println!("\nTo see what notifications look like, try: gh watch notify --since 2d");
    Ok(())
}

/// `gh watch config`: shows what's watched, or adds or removes the
/// `--label`s and `--repo`s given. Changes apply from the next run.
fn config_command(cli: &Cli, action: Option<&ConfigAction>) -> Result<()> {
    let path = Config::path()?;
    let mut config = Config::load()?;
    match action {
        None => {}
        Some(action) => {
            if cli.labels.is_empty() && cli.repos.is_empty() {
                bail!(
                    "say what to change: --label <name> or --repo owner/name (repeat for several)"
                );
            }
            match action {
                ConfigAction::Add => {
                    if !config.add(&cli.repos, &cli.labels) {
                        println!("Already watching all of that.");
                    }
                    check_labels(&config, &cli.labels);
                }
                ConfigAction::Remove => {
                    for name in config.remove(&cli.repos, &cli.labels) {
                        println!("Wasn't watching {name:?}.");
                    }
                }
            }
            config.save()?;
        }
    }
    let repos = if config.repos.is_empty() {
        String::from("no repos yet (add one with `gh watch config add --repo owner/name`)")
    } else {
        config.repos.join(", ")
    };
    println!("Watching {repos}: {}.", config.describe());
    println!("Config: {}", path.display());
    Ok(())
}

/// Warns about labels that don't exist in a watched repo: a typo would
/// otherwise quietly match nothing.
fn check_labels(config: &Config, labels: &[String]) {
    for label in labels {
        for repo in &config.repos {
            match gh::labels(repo, label) {
                Ok(found) if found.contains(&label.to_lowercase()) => {}
                Ok(_) => println!(
                    "  ! {repo} has no label {label:?}, so nothing will match it. \
                     See `gh label list -R {repo}`."
                ),
                Err(err) => println!("  ! Couldn't check label {label:?} in {repo} ({err:#})"),
            }
        }
    }
}

/// The repos to watch: --repo, else the config's, else the current one.
fn repos(cli: &Cli, config: &Config) -> Result<Vec<String>> {
    if !cli.repos.is_empty() {
        return Ok(cli.repos.clone());
    }
    if !config.repos.is_empty() {
        return Ok(config.repos.clone());
    }
    gh::current_repo().map(|r| vec![r]).with_context(|| {
        format!(
            "no repo to watch: pass --repo owner/name, run this inside a repo, or list repos in {}",
            Config::path()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        )
    })
}

/// Every change in `repos` since `since`, with its repo. A repo or issue
/// that can't be read is reported and skipped.
fn gather(repos: &[String], watch: &Watch, since: DateTime<Utc>) -> Result<Vec<(String, Change)>> {
    let me = gh::me()?;
    let mut found = Vec::new();
    for repo in repos {
        let issues = match changes::watched(repo, watch, since) {
            Ok(issues) => issues,
            Err(err) => {
                eprintln!("Skipping {repo}: {err:#}");
                continue;
            }
        };
        let (repo_changes, failed) = changes::changes(repo, &issues, &me, since);
        for (number, err) in failed {
            eprintln!("Skipping {repo}#{number}: {err:#}");
        }
        found.extend(repo_changes.into_iter().map(|c| (repo.clone(), c)));
    }
    Ok(found)
}

fn print(found: &[(String, Change)], since: DateTime<Utc>) {
    if found.is_empty() {
        println!(
            "Nothing changed since {}.",
            since.format("%b %-d %H:%M UTC")
        );
        return;
    }
    let mut last_repo = "";
    for (repo, c) in found {
        if repo != last_repo {
            println!("{repo}");
            last_repo = repo;
        }
        println!("  #{} {}", c.issue.number, c.issue.short_title());
        for line in what(c) {
            println!("      {line}");
        }
        println!("      {}", c.url());
    }
}

/// What happened, a line each: "Closed by PR #9", "sally: looks fixed".
fn what(c: &Change) -> Vec<String> {
    let mut lines = Vec::new();
    if c.closed {
        lines.push(match c.closed_by() {
            Some(pr) => format!("Closed by PR #{}", pr.number),
            None => String::from("Closed"),
        });
    }
    if let Some(last) = c.comments.last() {
        let others: Vec<&str> = c
            .commenters()
            .into_iter()
            .filter(|p| *p != last.user.login)
            .collect();
        let more = match (c.comments.len() - 1, others.is_empty()) {
            (0, _) => String::new(),
            (n, true) => format!(" (+{n} more)"),
            (n, false) => format!(" (+{n} more, from {})", others.join(", ")),
        };
        lines.push(format!(
            "{}: {}{more}",
            last.user.login,
            changes::preview(&last.body, PREVIEW_CHARS)
        ));
    }
    lines
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    last_notified: Option<DateTime<Utc>>,
}

fn state_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not find your home folder")?;
    Ok(home.join(".local/state/gh-watch/state.json"))
}

/// The saved state, or an empty one if there's none (or it's unreadable).
fn load_state() -> Result<State> {
    let path = state_path()?;
    Ok(std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default())
}

fn save_state(state: &State) -> Result<()> {
    let path = state_path()?;
    std::fs::create_dir_all(path.parent().context("bad state path")?)?;
    std::fs::write(&path, serde_json::to_vec_pretty(state)?)?;
    Ok(())
}

/// Notifies about changes since the last run, or over `window` if given.
/// Only a normal run (no window, not a dry run) moves the saved time.
fn notify(repos: &[String], watch: &Watch, window: Option<Duration>, dry_run: bool) -> Result<()> {
    let mut state = load_state()?;
    let now = Utc::now();

    let trial = window.is_some() || dry_run;
    let since = match (window, state.last_notified) {
        (Some(window), _) => ago(window)?,
        (None, Some(since)) => since,
        (None, None) if dry_run => {
            println!("First run: nothing to compare with yet. Try --since 2d.");
            return Ok(());
        }
        (None, None) => {
            state.last_notified = Some(now);
            save_state(&state)?;
            println!("First run: you'll be notified about changes from now on.");
            return Ok(());
        }
    };
    let found = gather(repos, watch, since)?;
    // A dry run lists each change as a "Would notify" line instead.
    if !dry_run || found.is_empty() {
        print(&found, since);
    }

    let mut notes: Vec<Note> = found
        .iter()
        .map(|(repo, c)| Note {
            // A short headline that's never cut off, saying who did what
            // where, and the long text in the body, which can wrap.
            title: headline(repo, c),
            subtitle: String::new(),
            body: content(c),
            open: c.url().to_string(),
            group: repo.clone(),
        })
        .collect();
    if notes.len() > MAX_NOTIFICATIONS {
        let rest = &found[MAX_NOTIFICATIONS - 1..];
        notes.truncate(MAX_NOTIFICATIONS - 1);
        notes.push(Note {
            title: format!("{} more updates", rest.len()),
            subtitle: String::new(),
            body: summary(rest),
            open: format!(
                "https://github.com/{}/issues?q=involves%3A%40me+updated%3A%3E%3D{}",
                found[0].0,
                since.format("%Y-%m-%d")
            ),
            group: String::from("summary"),
        });
    }
    let mut said_fallback = false;
    for note in &notes {
        if dry_run {
            println!(
                "Would notify: {} | {}",
                note.title,
                note.body.replace('\n', " / ")
            );
        } else {
            let way = desktop::send(note)?;
            // Said once a run, so the log shows why there's no cat.
            if way.is_fallback() && !said_fallback {
                eprintln!(
                    "Sent with {}: see `gh watch doctor` for why the app wasn't used.",
                    way.describe()
                );
                said_fallback = true;
            }
        }
    }
    if trial {
        return Ok(());
    }
    // Moved only after sending, so a failed run tries again next time.
    state.last_notified = Some(now);
    save_state(&state)
}

/// An issue as a notification names it: "arize (#48765)".
fn reference(repo: &str, c: &Change) -> String {
    format!("{} (#{})", short_repo(repo), c.issue.number)
}

/// A notification's first line: who did what, where. "sally commented on
/// arize (#48765)", "arize (#48765) closed by PR #94655".
fn headline(repo: &str, c: &Change) -> String {
    let issue = reference(repo, c);
    if c.closed {
        return match c.closed_by() {
            Some(pr) => format!("{issue} closed by PR #{}", pr.number),
            None => format!("{issue} closed"),
        };
    }
    let people = c.commenters();
    match (c.comments.len(), people.as_slice()) {
        (1, [who]) => format!("{who} commented on {issue}"),
        (n, [who]) => format!("{n} new comments on {issue} from {who}"),
        (n, many) => format!("{n} new comments on {issue} from {}", many.join(", ")),
    }
}

/// A notification's body: the newest comment in its writer's words (with
/// their name when there could be doubt), or for a close without
/// comments, the issue's full title.
fn content(c: &Change) -> String {
    let Some(last) = c.comments.last() else {
        return c.issue.short_title().to_string();
    };
    let text = changes::preview(&last.body, PREVIEW_CHARS);
    if c.comments.len() == 1 && !c.closed {
        text // the headline already says who
    } else {
        format!("{}: {text}", last.user.login)
    }
}

/// The summary's body: the rest, by repo. "arize: #93337, #95251".
fn summary(rest: &[(String, Change)]) -> String {
    let mut by_repo: Vec<(&str, Vec<String>)> = Vec::new();
    for (repo, c) in rest {
        let number = format!("#{}", c.issue.number);
        match by_repo.iter_mut().find(|(r, _)| *r == short_repo(repo)) {
            Some((_, numbers)) => numbers.push(number),
            None => by_repo.push((short_repo(repo), vec![number])),
        }
    }
    by_repo
        .iter()
        .map(|(repo, numbers)| format!("{repo}: {}", numbers.join(", ")))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// "Arize-ai/arize" -> "arize", to keep titles short.
fn short_repo(repo: &str) -> &str {
    repo.rsplit('/').next().unwrap_or(repo)
}

/// "30m", "12h", "1d", "2w".
fn parse_duration(text: &str) -> Result<Duration> {
    let text = text.trim();
    let digits = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (n, unit) = text.split_at(digits);
    let bad = || format!("bad duration {text:?}: try 30m, 12h, 1d or 2w");
    let n: i64 = n.parse().with_context(bad)?;
    let duration = match unit {
        "m" => Duration::try_minutes(n),
        "h" => Duration::try_hours(n),
        "d" => Duration::try_days(n),
        "w" => Duration::try_weeks(n),
        _ => bail!(bad()),
    };
    duration.with_context(|| format!("duration {text:?} is too long"))
}

/// The time `window` ago, or an error if that's before the calendar starts.
fn ago(window: Duration) -> Result<DateTime<Utc>> {
    Utc::now()
        .checked_sub_signed(window)
        .context("that --since window is too long")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("30m").unwrap(), Duration::minutes(30));
        assert_eq!(parse_duration("2w").unwrap(), Duration::weeks(2));
        assert!(parse_duration("3y").is_err());
        assert!(parse_duration("100000000000w").is_err());
        assert!(ago(Duration::MAX).is_err());
        assert!(parse_duration("d").is_err());
    }

    fn change(closed: bool, pr: Option<u64>, comments: &[(&str, &str)]) -> Change {
        let mut issue: gh_watch::gh::Issue = serde_json::from_str(
            r#"{"number": 48765, "title": "[Bug] Max limit counts spans", "state": "OPEN",
                "url": "u", "updatedAt": "2026-11-18T00:00:00Z", "closedAt": null,
                "labels": [], "assignees": []}"#,
        )
        .unwrap();
        if let Some(n) = pr {
            issue.prs.push(gh_watch::gh::PrRef {
                number: n,
                url: String::new(),
            });
        }
        let comments = comments
            .iter()
            .map(|(who, body)| {
                serde_json::from_value(serde_json::json!({
                    "user": {"login": who, "type": "User"},
                    "created_at": "2026-11-18T00:00:00Z", "body": body, "html_url": ""
                }))
                .unwrap()
            })
            .collect();
        Change {
            issue,
            closed,
            comments,
        }
    }

    const REPO: &str = "Arize-ai/arize";

    #[test]
    fn closes_name_the_pr_and_show_the_title() {
        let c = change(true, Some(94655), &[]);
        assert_eq!(headline(REPO, &c), "arize (#48765) closed by PR #94655");
        assert_eq!(content(&c), "Max limit counts spans");
        assert_eq!(
            headline(REPO, &change(true, None, &[])),
            "arize (#48765) closed"
        );
    }

    #[test]
    fn one_comment_names_its_writer_in_the_headline() {
        let c = change(false, None, &[("sally", "Looks  fixed\nnow")]);
        assert_eq!(headline(REPO, &c), "sally commented on arize (#48765)");
        assert_eq!(content(&c), "Looks fixed now");
    }

    #[test]
    fn several_comments_are_counted_and_the_newest_shown() {
        let c = change(
            false,
            None,
            &[("sally", "a"), ("amir", "b"), ("sally", "c")],
        );
        assert_eq!(
            headline(REPO, &c),
            "3 new comments on arize (#48765) from sally, amir"
        );
        assert_eq!(content(&c), "sally: c");
        let c = change(false, None, &[("sally", "a"), ("sally", "b")]);
        assert_eq!(
            headline(REPO, &c),
            "2 new comments on arize (#48765) from sally"
        );
    }

    #[test]
    fn a_close_with_comments_shows_the_comment() {
        let c = change(true, Some(9), &[("sally", "Shipped")]);
        assert_eq!(headline(REPO, &c), "arize (#48765) closed by PR #9");
        assert_eq!(content(&c), "sally: Shipped");
    }

    #[test]
    fn summary_groups_numbers_by_repo() {
        let rest = vec![
            (REPO.to_string(), change(true, None, &[])),
            ("acme/web".to_string(), change(true, None, &[])),
        ];
        assert_eq!(summary(&rest), "arize: #48765 · web: #48765");
    }

    #[test]
    fn short_repo_names() {
        assert_eq!(short_repo("Arize-ai/arize"), "arize");
        assert_eq!(short_repo("solo"), "solo");
    }
}
