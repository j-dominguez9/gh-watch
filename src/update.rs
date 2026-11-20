//! Telling you when a newer gh-watch is out: otherwise nobody upgrades.
//! `notify` looks at most once a day, and only a minor release (0.1.x to
//! 0.2.0) is worth a notification, once: bug-fix releases aren't worth
//! interrupting anyone for. `doctor` mentions any newer release. A failed
//! look (offline, say) is skipped without a word.

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use gh_watch::gh;

use crate::desktop::Note;
use crate::state::State;

/// How often `notify` looks.
const CHECK_EVERY: Duration = Duration::hours(24);

/// When gh-watch can't tell how it was installed.
const REINSTALL: &str = "reinstall gh-watch, then run `gh-watch setup`";

/// A version newer than this one.
pub struct Newer {
    /// "0.2.0".
    pub version: String,
    /// The release's page.
    pub url: String,
    /// More than bug fixes: the first or second number went up.
    pub minor: bool,
}

/// The newest release, if it's newer than this gh-watch.
pub fn newer() -> Result<Option<Newer>> {
    // Where releases come from: Cargo.toml's `repository`.
    let repo = env!("CARGO_PKG_REPOSITORY")
        .strip_prefix("https://github.com/")
        .context("gh-watch's repository isn't on GitHub")?;
    let release = gh::latest_release(repo)?;
    let version = release.tag.trim_start_matches('v').to_string();
    Ok(
        compare(&version, env!("CARGO_PKG_VERSION")).map(|minor| Newer {
            version,
            url: release.url,
            minor,
        }),
    )
}

/// The command that upgrades this install to `version` (then rebuilds the
/// app and the schedule), if gh-watch can tell how it was installed.
pub fn upgrade_command(version: &str) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let cargo_bin = std::env::var_os("CARGO_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".cargo")))
        .map(|cargo| cargo.join("bin"));
    command_for(&exe, cargo_bin.as_deref(), version)
}

fn command_for(
    exe: &std::path::Path,
    cargo_bin: Option<&std::path::Path>,
    version: &str,
) -> Option<String> {
    if exe.to_string_lossy().contains("/gh/extensions/") {
        Some(String::from(
            "gh extension upgrade gh-watch && gh watch setup",
        ))
    } else if cargo_bin.is_some_and(|bin| exe.starts_with(bin)) {
        // That release exactly (not whatever's on main), even though a
        // gh-watch is already installed.
        Some(format!(
            "cargo install --git {} --tag v{version} --force && gh-watch setup",
            env!("CARGO_PKG_REPOSITORY")
        ))
    } else {
        None
    }
}

/// How to upgrade to `version`, as a fix to show: "run `...`".
pub fn how_to_upgrade(version: &str) -> String {
    match upgrade_command(version) {
        Some(command) => format!("run `{command}`"),
        None => REINSTALL.to_string(),
    }
}

/// For `notify`: a notification about a newer minor release, when it's
/// time to look (once a day, counted from the last look that worked) and
/// it's one you haven't been told about. Returns it with the minor version
/// ("0.2"), to remember once it's sent.
pub fn note(state: &mut State, now: DateTime<Utc>) -> Option<(Note, String)> {
    if state
        .update_checked
        .is_some_and(|at| now - at < CHECK_EVERY)
    {
        return None;
    }
    let newer = newer().ok()?;
    state.update_checked = Some(now);
    let newer = newer.filter(|newer| newer.minor)?;
    let minor = minor_of(&newer.version)?;
    // Said once per minor release: not again for its bug-fix releases.
    let announced = state.update_announced.as_deref().and_then(minor_of);
    if announced.as_deref() == Some(minor.as_str()) {
        return None;
    }
    let body = match upgrade_command(&newer.version) {
        Some(command) => format!("Upgrade: `{command}`"),
        None => format!("{}{}.", REINSTALL[..1].to_uppercase(), &REINSTALL[1..]),
    };
    // Clicking opens the release, with what's new.
    let note = Note {
        title: String::from("gh-watch update available"),
        subtitle: String::new(),
        body,
        open: newer.url,
        group: String::from("gh-watch"),
    };
    Some((note, minor))
}

/// If `version` comes after `current` ("0.1.10" after "0.1.9"): whether
/// it's a minor release, its first two numbers higher ("0.2.0" after
/// "0.1.9"). None if it isn't newer, or isn't a plain version (a
/// prerelease, say).
fn compare(version: &str, current: &str) -> Option<bool> {
    let (version, current) = (numbers(version)?, numbers(current)?);
    (version > current).then(|| version[..2] > current[..2])
}

/// "0.2" for "0.2.1" (or "0.2").
fn minor_of(version: &str) -> Option<String> {
    let [major, minor, _] = numbers(version)?;
    Some(format!("{major}.{minor}"))
}

/// The three numbers of "0.2.1" ("0.2" counts as "0.2.0"); None for
/// anything else.
fn numbers(version: &str) -> Option<[u64; 3]> {
    let parts: Vec<u64> = version
        .split('.')
        .map(|part| part.parse().ok())
        .collect::<Option<_>>()?;
    match parts.as_slice() {
        [major, minor] => Some([*major, *minor, 0]),
        [major, minor, patch] => Some([*major, *minor, *patch]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_as_numbers() {
        // Bug-fix releases: newer, not minor.
        assert_eq!(compare("0.1.5", "0.1.4"), Some(false));
        assert_eq!(compare("0.1.10", "0.1.9"), Some(false));
        // Minor (or major) releases.
        assert_eq!(compare("0.2.0", "0.1.9"), Some(true));
        assert_eq!(compare("0.2.3", "0.1.4"), Some(true));
        assert_eq!(compare("1.0.0", "0.9.9"), Some(true));
        // Not newer.
        assert_eq!(compare("0.1.4", "0.1.4"), None);
        assert_eq!(compare("0.1.3", "0.1.4"), None);
        assert_eq!(compare("0.2.0-beta", "0.1.4"), None);
    }

    #[test]
    fn the_upgrade_command_fits_the_install() {
        use std::path::Path;
        let bin = Path::new("/Users/me/.cargo/bin");
        assert_eq!(
            command_for(
                Path::new("/Users/me/.local/share/gh/extensions/gh-watch/gh-watch"),
                Some(bin),
                "0.2.0"
            )
            .as_deref(),
            Some("gh extension upgrade gh-watch && gh watch setup")
        );
        assert!(
            command_for(
                Path::new("/Users/me/.cargo/bin/gh-watch"),
                Some(bin),
                "0.2.0"
            )
            .is_some_and(|c| c.starts_with("cargo install --git https://github.com/")
                && c.contains("--tag v0.2.0 --force"))
        );
        assert_eq!(
            command_for(Path::new("/tmp/build/gh-watch"), Some(bin), "0.2.0"),
            None
        );
    }

    #[test]
    fn versions_are_three_numbers() {
        assert_eq!(compare("0.1.5.0", "0.1.5"), None);
        assert_eq!(compare("0.2", "0.1.5"), Some(true));
        assert_eq!(minor_of("0.2.1").as_deref(), Some("0.2"));
        assert_eq!(minor_of("0.2").as_deref(), Some("0.2"));
    }

    #[test]
    fn looks_at_most_once_a_day() {
        let now = Utc::now();
        let mut state = State {
            update_checked: Some(now - Duration::hours(3)),
            ..State::default()
        };
        assert!(note(&mut state, now).is_none());
        // Not time yet, so nothing was looked up or changed.
        assert_eq!(state.update_checked, Some(now - Duration::hours(3)));
    }
}
