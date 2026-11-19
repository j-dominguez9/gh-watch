//! gh-watch: know when GitHub issues you care about change. Works as a
//! command (`gh-watch`) or a gh extension (`gh watch`).
//!
//! It watches issues you're involved in (opened, assigned, mentioned,
//! commented on) and, optionally, issues with given labels, and reports
//! only real changes: closed, or commented on by someone other than you or
//! a bot.

mod config;
mod desktop;

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

fn main() -> Result<()> {
    let cli = Cli::parse();
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

/// Notifies about changes since the last run, or over `window` if given.
/// Only a normal run (no window, not a dry run) moves the saved time.
fn notify(repos: &[String], watch: &Watch, window: Option<Duration>, dry_run: bool) -> Result<()> {
    let path = state_path()?;
    let mut state: State = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let now = Utc::now();
    let save = |state: &State| -> Result<()> {
        std::fs::create_dir_all(path.parent().context("bad state path")?)?;
        std::fs::write(&path, serde_json::to_vec_pretty(state)?)?;
        Ok(())
    };

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
            save(&state)?;
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
            title: format!("{} #{}", short_repo(repo), c.issue.number),
            subtitle: c.issue.short_title().to_string(),
            body: what(c).join("\n"),
            open: c.url().to_string(),
        })
        .collect();
    if notes.len() > MAX_NOTIFICATIONS {
        let rest = notes.split_off(MAX_NOTIFICATIONS - 1);
        let repo = &found[0].0;
        notes.push(Note {
            title: String::from("gh-watch"),
            subtitle: format!("{} more updates", rest.len()),
            body: rest
                .iter()
                .map(|n| n.title.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            open: format!(
                "https://github.com/{repo}/issues?q=involves%3A%40me+updated%3A%3E%3D{}",
                since.format("%Y-%m-%d")
            ),
        });
    }
    for note in &notes {
        if dry_run {
            println!(
                "Would notify: {} | {} | {}",
                note.title,
                note.subtitle,
                note.body.replace('\n', " / ")
            );
        } else {
            desktop::send(note)?;
        }
    }
    if trial {
        return Ok(());
    }
    // Moved only after sending, so a failed run tries again next time.
    state.last_notified = Some(now);
    save(&state)
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

    #[test]
    fn short_repo_names() {
        assert_eq!(short_repo("Arize-ai/arize"), "arize");
        assert_eq!(short_repo("solo"), "solo");
    }
}
