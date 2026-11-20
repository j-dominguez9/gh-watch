//! What gh-watch remembers between runs, in
//! ~/.local/state/gh-watch/state.json: when it last notified (the clock),
//! and what it sent since then, so looking at a window again doesn't
//! repeat anything. Safe to delete: the next run just starts the clock.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
pub struct State {
    /// The clock: changes after this haven't been notified about yet.
    pub last_notified: Option<DateTime<Utc>>,
    /// For each issue notified about ("owner/repo#123"), when the newest
    /// change sent happened.
    #[serde(default)]
    pub sent: BTreeMap<String, DateTime<Utc>>,
}

fn path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not find your home folder")?;
    Ok(home.join(".local/state/gh-watch/state.json"))
}

impl State {
    /// The saved state, or an empty one if there's none (or it's unreadable:
    /// that only means starting the clock again).
    pub fn load() -> Result<State> {
        let path = path()?;
        Ok(std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default())
    }

    pub fn save(&self) -> Result<()> {
        let path = path()?;
        std::fs::create_dir_all(path.parent().context("bad state path")?)?;
        std::fs::write(&path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    /// Starts the clock if it hasn't started. Returns whether it did.
    pub fn start(&mut self, now: DateTime<Utc>) -> bool {
        let starting = self.last_notified.is_none();
        if starting {
            self.last_notified = Some(now);
        }
        starting
    }
}

/// What a `notify` run should do, from its flags and the clock.
#[derive(Debug, PartialEq)]
pub enum Plan {
    /// First run: start the clock, notify about nothing.
    StartClock,
    /// First run of a dry run: there's no clock to compare with.
    NothingToCompare,
    /// Look at changes since this time. `trial`: a look-back window or a
    /// dry run, which leaves the clock and the sent list alone.
    Since { since: DateTime<Utc>, trial: bool },
}

/// Decides what `notify` does: a `window` (like `--since 2d`) looks back
/// from now; otherwise it's since the clock.
pub fn plan(
    last_notified: Option<DateTime<Utc>>,
    window: Option<Duration>,
    dry_run: bool,
    now: DateTime<Utc>,
) -> Result<Plan> {
    let trial = window.is_some() || dry_run;
    Ok(match (window, last_notified) {
        (Some(window), _) => Plan::Since {
            since: now
                .checked_sub_signed(window)
                .context("that --since window is too long")?,
            trial,
        },
        (None, Some(since)) => Plan::Since { since, trial },
        (None, None) if dry_run => Plan::NothingToCompare,
        (None, None) => Plan::StartClock,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn plans() {
        let now = at("2026-11-20T12:00:00Z");
        let clock = Some(at("2026-11-20T11:30:00Z"));
        // A scheduled run: since the clock, for real.
        assert_eq!(
            plan(clock, None, false, now).unwrap(),
            Plan::Since {
                since: at("2026-11-20T11:30:00Z"),
                trial: false
            }
        );
        // Trying it out: a window from now, or a dry run, are trials.
        assert_eq!(
            plan(clock, Some(Duration::days(2)), false, now).unwrap(),
            Plan::Since {
                since: at("2026-11-18T12:00:00Z"),
                trial: true
            }
        );
        assert!(matches!(
            plan(clock, None, true, now).unwrap(),
            Plan::Since { trial: true, .. }
        ));
        // First runs.
        assert_eq!(plan(None, None, false, now).unwrap(), Plan::StartClock);
        assert_eq!(plan(None, None, true, now).unwrap(), Plan::NothingToCompare);
        assert!(plan(None, Some(Duration::MAX), false, now).is_err());
    }

    #[test]
    fn an_old_state_file_still_loads() {
        let state: State =
            serde_json::from_str(r#"{"last_notified": "2026-11-20T18:28:24Z"}"#).unwrap();
        assert!(state.sent.is_empty());
    }
}
