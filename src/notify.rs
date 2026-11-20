//! `gh watch changes` (list what changed) and `gh watch notify` (send it
//! as notifications, from the schedule).
//!
//! notify reports each change once, even when it looks at a window again:
//! the clock (`State::last_notified`) only moves once everything in the
//! window was read, and what was sent is remembered until the clock passes
//! it.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use gh_watch::changes::{self, Change, Watch};
use gh_watch::gh;

use crate::config::Config;
use crate::desktop::{self, Note};
use crate::message;
use crate::state::{self, Plan, State};
use crate::update;

/// At most this many notifications per run, the last a summary when
/// there are more.
const MAX_NOTIFICATIONS: usize = 5;
/// How long a repo that can't be read can keep the clock back. Past this,
/// it moves on without it, so a repo that stays broken (deleted, say)
/// can't hold everything back.
const HOLD_AT_MOST: Duration = Duration::hours(24);

/// What `changes` looks at instead of the config, for one run.
pub struct Instead {
    pub repos: Vec<String>,
    pub labels: Vec<String>,
    pub no_involved: bool,
}

/// `gh watch changes`: what changed in the last `window`. Uses the config,
/// or without one, the repo you're in; `instead` overrides either.
pub fn changes_command(window: Duration, instead: &Instead) -> Result<()> {
    let config = Config::read()?.unwrap_or_default();
    let repos = if !instead.repos.is_empty() {
        instead.repos.clone()
    } else if !config.repos.is_empty() {
        config.repos.clone()
    } else {
        vec![gh::current_repo().context(
            "no repo to look at: pass --repo owner/name, run this inside a repo, or run \
             `gh watch setup`",
        )?]
    };
    let watch = Watch {
        labels: if instead.labels.is_empty() {
            config.labels.clone()
        } else {
            instead.labels.clone()
        },
        involved: config.involved && !instead.no_involved,
    };
    if watch.labels.is_empty() && !watch.involved {
        bail!("nothing to look at: no labels, and not issues you're involved in");
    }
    let since = Utc::now()
        .checked_sub_signed(window)
        .context("that --since window is too long")?;
    let found = gather(&repos, &watch, since)?;
    print(&found.changes, since);
    Ok(())
}

/// What `gather` found.
struct Found {
    /// Every change, with its repo.
    changes: Vec<(String, Change)>,
    /// Some repo or issue couldn't be read, so changes may be missing.
    incomplete: bool,
}

/// Every change in `repos` since `since`. A repo or issue that can't be
/// read is reported and skipped, and the result marked incomplete.
fn gather(repos: &[String], watch: &Watch, since: DateTime<Utc>) -> Result<Found> {
    let me = gh::me()?;
    let mut found = Found {
        changes: Vec::new(),
        incomplete: false,
    };
    for repo in repos {
        let issues = match changes::watched(repo, watch, since) {
            Ok(issues) => issues,
            Err(err) => {
                eprintln!("Skipping {repo}: {err:#}");
                found.incomplete = true;
                continue;
            }
        };
        let (repo_changes, failed) = changes::changes(repo, &issues, &me, since);
        for (number, err) in failed {
            eprintln!("Skipping {repo}#{number}: {err:#}");
            found.incomplete = true;
        }
        found
            .changes
            .extend(repo_changes.into_iter().map(|c| (repo.clone(), c)));
    }
    Ok(found)
}

/// The changes in the terminal: by repo, each with its headline, what was
/// said, and the link.
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
        println!("      {}", message::headline(repo, c));
        let body = message::body(c);
        if body != c.issue.short_title() {
            println!("      {body}");
        }
        println!("      {}", c.url());
    }
}

/// `gh watch notify`: notifies about changes since the last run, or over
/// `window` if given. Only a normal run (no window, not a dry run) moves
/// the clock or remembers what it sent.
pub fn notify(window: Option<Duration>, dry_run: bool) -> Result<()> {
    let config = Config::read()?.context("no config yet: run `gh watch setup` in a repo")?;
    config.check().with_context(|| {
        format!(
            "{} can't be used",
            Config::path().unwrap_or_default().display()
        )
    })?;
    let mut state = State::load()?;
    let now = Utc::now();
    let (since, trial) = match state::plan(state.last_notified, window, dry_run, now)? {
        Plan::NothingToCompare => {
            println!("First run: nothing to compare with yet. Try --since 2d.");
            return Ok(());
        }
        Plan::StartClock => {
            state.start(now);
            state.save()?;
            println!("First run: you'll be notified about changes from now on.");
            return Ok(());
        }
        Plan::Since { since, trial } => (since, trial),
    };

    let Found {
        changes: mut found,
        incomplete,
    } = gather(&config.repos, &config.watch(), since)?;
    // A window looked at again mustn't repeat what was sent.
    if !trial {
        found.retain(|(repo, c)| {
            state
                .sent
                .get(&sent_key(repo, c))
                .is_none_or(|sent| changed_at(c) > *sent)
        });
    }
    // A dry run lists each change as a "Would notify" line instead.
    if !dry_run || found.is_empty() {
        print(&found, since);
    }

    let notes = notes(&found, since);
    if dry_run {
        for (note, _) in &notes {
            println!(
                "Would notify: {} | {}",
                note.title,
                note.body.replace('\n', " / ")
            );
        }
        return Ok(());
    }

    let mut said_fallback = false;
    for (note, covers) in &notes {
        let way = match desktop::send(note) {
            Ok(way) => way,
            Err(err) => {
                // Keep what did go out, so the next run doesn't repeat it.
                if !trial {
                    state.save()?;
                }
                return Err(err);
            }
        };
        // Said once a run, so the log shows why there's no cat.
        if way.is_fallback() && !said_fallback {
            eprintln!(
                "Sent with {}: see `gh watch doctor` for why the app wasn't used.",
                way.describe()
            );
            said_fallback = true;
        }
        if !trial {
            for (repo, c) in *covers {
                state.sent.insert(sent_key(repo, c), changed_at(c));
            }
        }
    }
    if trial {
        return Ok(());
    }
    // Whatever couldn't be read hasn't been reported, so keep the clock
    // where it is and look at the same window again next time (what was
    // sent isn't repeated), up to HOLD_AT_MOST.
    let hold = incomplete && now - since < HOLD_AT_MOST;
    if hold {
        eprintln!(
            "Some of it couldn't be read, so the next run will look again from {}.",
            since.format("%b %-d %H:%M UTC")
        );
    } else {
        if incomplete {
            eprintln!(
                "Some of it still couldn't be read after {} hours; moving on without it.",
                HOLD_AT_MOST.num_hours()
            );
        }
        state.last_notified = Some(now);
    }
    // What's older than the clock can't come back, so it needn't be kept.
    let clock = state.last_notified.unwrap_or(now);
    state.sent.retain(|_, at| *at > clock);
    // Saved before looking for a newer gh-watch: that's a network call
    // that could hang, and mustn't cost what this run sent.
    state.save()?;
    // A newer minor release, once; remembered only once it's sent.
    if let Some((note, minor)) = update::note(&mut state, now)
        && desktop::send(&note).is_ok()
    {
        state.update_announced = Some(minor);
    }
    state.save()
}

/// One notification per change, each with the changes it covers (to
/// remember as sent); past MAX_NOTIFICATIONS, the rest in one summary.
fn notes(found: &[(String, Change)], since: DateTime<Utc>) -> Vec<(Note, &[(String, Change)])> {
    let mut notes: Vec<(Note, &[(String, Change)])> = found
        .iter()
        .enumerate()
        .map(|(i, (repo, c))| {
            let note = Note {
                title: message::headline(repo, c),
                subtitle: String::new(),
                body: message::body(c),
                open: c.url().to_string(),
                group: repo.clone(),
            };
            (note, std::slice::from_ref(&found[i]))
        })
        .collect();
    if notes.len() > MAX_NOTIFICATIONS {
        let rest = &found[MAX_NOTIFICATIONS - 1..];
        notes.truncate(MAX_NOTIFICATIONS - 1);
        let note = Note {
            title: format!("{} more updates", rest.len()),
            subtitle: String::new(),
            body: message::summary(rest),
            open: message::summary_link(rest, since),
            group: String::from("summary"),
        };
        notes.push((note, rest));
    }
    notes
}

/// "Arize-ai/arize#48765": how `State::sent` names an issue.
fn sent_key(repo: &str, c: &Change) -> String {
    format!("{repo}#{}", c.issue.number)
}

/// When the newest thing in a change happened: its last comment, or the
/// close.
fn changed_at(c: &Change) -> DateTime<Utc> {
    let commented = c.comments.last().map(|comment| comment.created_at);
    let closed = c.closed.then_some(c.issue.closed_at).flatten();
    commented
        .into_iter()
        .chain(closed)
        .max()
        .unwrap_or(c.issue.updated_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::tests::{REPO, change};

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn a_change_happened_at_its_newest_comment_or_close() {
        let mut c = change(true, None, &[("sally", "a")]);
        c.issue.closed_at = Some(at("2026-11-19T00:00:00Z"));
        // The comment is from the 18th, the close the 19th.
        assert_eq!(changed_at(&c), at("2026-11-19T00:00:00Z"));
        c.closed = false;
        assert_eq!(changed_at(&c), at("2026-11-18T00:00:00Z"));
        assert_eq!(sent_key(REPO, &c), "Arize-ai/arize#48765");
    }

    #[test]
    fn past_the_cap_the_rest_are_one_summary_covering_them_all() {
        let found: Vec<(String, Change)> = (0..7)
            .map(|_| (REPO.to_string(), change(true, None, &[])))
            .collect();
        let notes = notes(&found, at("2026-11-18T00:00:00Z"));
        assert_eq!(notes.len(), MAX_NOTIFICATIONS);
        assert!(notes[..4].iter().all(|(_, covers)| covers.len() == 1));
        let (summary, covers) = &notes[4];
        assert_eq!(summary.title, "3 more updates");
        assert_eq!(covers.len(), 3);
    }
}
