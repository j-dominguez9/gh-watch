//! gh-watch: know when GitHub issues you care about change. Works as a
//! command (`gh-watch`) or a gh extension (`gh watch`).
//!
//! It watches issues you're involved in (opened, assigned, mentioned,
//! commented on) and, optionally, issues with given labels, and reports
//! only real changes: closed, or commented on by someone other than you or
//! a bot.
//!
//! This file is the command line: each command, its flags, and which
//! module does it.
//! - `setup`: setup and config (what to watch), on top of `config`
//! - `notify`: changes and notify, on top of `state` (the clock) and
//!   `message` (the words)
//! - `desktop`, `app`, `schedule`: sending, the macOS app, launchd
//! - `update`: telling you when a newer gh-watch is out
//! - `doctor`: checks all of the above

mod app;
mod config;
mod desktop;
mod doctor;
mod message;
mod notify;
mod schedule;
mod setup;
mod state;
mod update;

use anyhow::{Context, Result, bail};
use chrono::Duration;
use clap::{Args, Parser, Subcommand};

use crate::state::State;

#[derive(Parser)]
#[command(
    version,
    about = "Know when GitHub issues you care about change: closed, or commented on by someone else"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Set everything up in one go: the config (with the repo you're in),
    /// the notifier app (macOS), notifications every 30 minutes, and the
    /// clock. Safe to run again: it only adds, and rebuilds the app and the
    /// schedule (do that after updating gh-watch).
    Setup(Watching),
    /// Show what's watched, or change it.
    Config {
        #[command(subcommand)]
        edit: Option<ConfigEdit>,
    },
    /// List what changed, e.g. `--since 1d`.
    Changes {
        /// How far back: 30m, 12h, 1d, 2w.
        #[arg(long, default_value = "1d", value_parser = parse_duration)]
        since: Duration,
        #[command(flatten)]
        instead: Instead,
    },
    /// Notify about what changed since the last time, one notification per
    /// issue. What the schedule runs; the first run only starts the clock.
    Notify {
        /// Notify about this window instead (e.g. 2d), leaving the clock
        /// alone: for trying it out. These are real notifications; add
        /// --dry-run to only print them.
        #[arg(long, value_parser = parse_duration)]
        since: Option<Duration>,
        /// Print what would be sent instead of sending it. Changes nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Check that notifications will work, with the fix for anything that
    /// won't: gh, the config, the app and its permission (macOS), and the
    /// schedule. Then send a test notification and say how it went out.
    Doctor {
        /// Skip the test notification.
        #[arg(long)]
        no_test: bool,
    },
    /// Build and install gh-watch's notifier app (macOS), so notifications
    /// show under its own name and icon. Needs Apple's command line tools.
    InstallApp,
    /// Run `notify` every 30 minutes (macOS, through launchd).
    Schedule {
        /// Stop and remove the schedule instead.
        #[arg(long)]
        remove: bool,
    },
}

/// What `setup` adds to the config.
#[derive(Args)]
struct Watching {
    /// A repo to watch, as owner/name (repeat for several). Default: the
    /// repo you're in.
    #[arg(long = "repo", short = 'R')]
    repos: Vec<String>,
    /// Also watch issues with this label (repeat for several).
    #[arg(long = "label", short = 'l')]
    labels: Vec<String>,
    /// Only watch labels, not issues you're involved in.
    #[arg(long)]
    no_involved: bool,
}

#[derive(Subcommand)]
enum ConfigEdit {
    /// Start watching these.
    Add(EditArgs),
    /// Stop watching these.
    Remove(EditArgs),
}

#[derive(Args)]
struct EditArgs {
    /// A repo, as owner/name (repeat for several).
    #[arg(long = "repo", short = 'R')]
    repos: Vec<String>,
    /// A label (repeat for several).
    #[arg(long = "label", short = 'l')]
    labels: Vec<String>,
    /// Issues you're involved in (opened, assigned, mentioned, commented).
    #[arg(long)]
    involved: bool,
}

/// What `changes` looks at instead of the config, this once.
#[derive(Args)]
struct Instead {
    /// Look at this repo instead of the config's (repeat for several).
    #[arg(long = "repo", short = 'R')]
    repos: Vec<String>,
    /// Look at issues with this label instead of the config's labels
    /// (repeat for several).
    #[arg(long = "label", short = 'l')]
    labels: Vec<String>,
    /// Leave out issues you're involved in.
    #[arg(long)]
    no_involved: bool,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Cmd::Setup(w) => setup::setup(&w.repos, &w.labels, w.no_involved),
        Cmd::Config { edit } => {
            let edit = edit.map(|edit| {
                let (remove, args) = match edit {
                    ConfigEdit::Add(args) => (false, args),
                    ConfigEdit::Remove(args) => (true, args),
                };
                setup::Edit {
                    remove,
                    repos: args.repos,
                    labels: args.labels,
                    involved: args.involved,
                }
            });
            setup::config_command(edit.as_ref())
        }
        Cmd::Changes { since, instead } => notify::changes_command(
            since,
            &notify::Instead {
                repos: instead.repos,
                labels: instead.labels,
                no_involved: instead.no_involved,
            },
        ),
        Cmd::Notify { since, dry_run } => notify::notify(since, dry_run),
        Cmd::Doctor { no_test } => {
            // Non-zero when something's wrong, for scripts.
            if doctor::run(State::load()?.last_notified, !no_test) > 0 {
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::InstallApp => {
            let path = app::install()?;
            println!(
                "Installed {}. Allow notifications if macOS asks.",
                path.display()
            );
            Ok(())
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
            Ok(())
        }
        Cmd::Schedule { remove: false } => {
            schedule::install()?;
            println!(
                "gh-watch notify now runs every 30 minutes (log: {}).",
                schedule::log_path()?.display()
            );
            Ok(())
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("30m").unwrap(), Duration::minutes(30));
        assert_eq!(parse_duration("2w").unwrap(), Duration::weeks(2));
        assert!(parse_duration("3y").is_err());
        assert!(parse_duration("100000000000w").is_err());
        assert!(parse_duration("d").is_err());
    }

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn flags_belong_to_the_commands_that_use_them() {
        // `--label` adds on setup and config, looks instead on changes...
        assert!(Cli::try_parse_from(["gh-watch", "setup", "--label", "x"]).is_ok());
        assert!(Cli::try_parse_from(["gh-watch", "config", "add", "--label", "x"]).is_ok());
        assert!(Cli::try_parse_from(["gh-watch", "changes", "--label", "x"]).is_ok());
        // ...and isn't accepted where it would do nothing.
        assert!(Cli::try_parse_from(["gh-watch", "notify", "--label", "x"]).is_err());
        assert!(Cli::try_parse_from(["gh-watch", "doctor", "--label", "x"]).is_err());
        assert!(Cli::try_parse_from(["gh-watch", "schedule", "--repo", "o/r"]).is_err());
    }
}
