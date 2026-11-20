//! Telling you when a newer gh-watch is out: otherwise nobody upgrades.
//! `notify` looks at most once a day, and only a minor release (0.1.x to
//! 0.2.0) is worth a notification, once: bug-fix releases aren't worth
//! interrupting anyone for. `doctor` mentions any newer release. A failed
//! look (offline, say) is skipped without a word.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use gh_watch::gh;

use crate::desktop::Note;
use crate::state::State;

/// How often `notify` looks.
const CHECK_EVERY: Duration = Duration::hours(24);

/// When gh-watch can't tell how it was installed.
/// (Not `setup` after: that would also add the repo you're in.)
const REINSTALL: &str =
    "reinstall gh-watch, then run `gh-watch install-app` and `gh-watch schedule`";

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

/// How this gh-watch was installed, which decides how it upgrades.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Install {
    /// `gh extension install`: `gh extension upgrade` replaces it.
    Extension,
    /// `cargo install`: rebuilt from the release's tag.
    Cargo,
    /// Anything else (a build folder, say): it can't upgrade itself.
    Unknown,
}

impl Install {
    /// This one, from where it runs.
    pub fn this() -> Install {
        std::env::current_exe().map_or(Install::Unknown, |exe| Install::at(&exe))
    }

    /// The install a gh-watch at `exe` is.
    pub fn at(exe: &Path) -> Install {
        Install::of(exe, gh_watch::which::cargo_bin().as_deref())
    }

    fn of(exe: &Path, cargo_bin: Option<&Path>) -> Install {
        if exe.to_string_lossy().contains("/gh/extensions/") {
            Install::Extension
        } else if cargo_bin.is_some_and(|bin| exe.starts_with(bin)) {
            Install::Cargo
        } else {
            Install::Unknown
        }
    }
}

/// The command that upgrades this install, if it can upgrade itself.
pub fn upgrade_command() -> Option<&'static str> {
    match Install::this() {
        Install::Extension => Some("gh watch upgrade"),
        Install::Cargo => Some("gh-watch upgrade"),
        Install::Unknown => None,
    }
}

/// How to upgrade, as a fix to show: "run `...`".
pub fn how_to_upgrade() -> String {
    match upgrade_command() {
        Some(command) => format!("run `{command}`"),
        None => REINSTALL.to_string(),
    }
}

/// `gh watch upgrade`: gets the newest release, then has it rebuild the
/// app and reload the schedule. Not `setup`, which would also add the repo
/// you happen to be in to the config.
pub fn upgrade() -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let Some(newer) = newer()? else {
        println!("gh-watch {current} is the latest.");
        return Ok(());
    };
    let exe = std::env::current_exe().context("could not tell where gh-watch is")?;
    println!("Upgrading gh-watch {current} to {}...", newer.version);
    match Install::at(&exe) {
        Install::Extension => {
            run(Command::new(gh::program()?).args(["extension", "upgrade", "gh-watch"]))
                .with_context(|| {
                    // gh won't upgrade an extension pinned to a version, or
                    // one installed from a local folder.
                    let repo =
                        env!("CARGO_PKG_REPOSITORY").trim_start_matches("https://github.com/");
                    format!(
                        "gh couldn't upgrade it (see above). If it was installed pinned to a \
                         version or from a folder, reinstall it: `gh extension remove gh-watch \
                         && gh extension install {repo}`, then `gh watch install-app` and \
                         `gh watch schedule`"
                    )
                })?
        }
        Install::Cargo => {
            println!("(Building it from source takes a minute or two.)");
            let cargo = gh_watch::which::which(
                "cargo",
                &gh_watch::which::cargo_bin().into_iter().collect::<Vec<_>>(),
            )
            .context("cargo not found: is Rust installed (rustup)?")?;
            run(Command::new(cargo).args([
                "install",
                "--git",
                env!("CARGO_PKG_REPOSITORY"),
                "--tag",
                &format!("v{}", newer.version),
                "--force",
            ]))?
        }
        Install::Unknown => bail!(
            "this gh-watch wasn't installed as a gh extension or with cargo, so it can't \
             upgrade itself: {REINSTALL}"
        ),
    }

    // The new binary is where the old one was.
    let out = Command::new(&exe)
        .arg("--version")
        .output()
        .with_context(|| format!("could not run the new gh-watch at {}", exe.display()))?;
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if version != format!("gh-watch {}", newer.version) {
        bail!(
            "the upgrade ran, but this is still {version:?} rather than gh-watch {}; try \
             again, or {REINSTALL}",
            newer.version
        );
    }

    // Its own commands rebuild what was there: this (old) code would build
    // the old app. Only what was installed (a schedule you removed stays
    // removed), and a step that fails doesn't stop the other, like setup.
    // Both commands exist in every version, so this works across them.
    let mut warnings = 0;
    if cfg!(target_os = "macos") {
        let steps = [
            (crate::desktop::app().is_some(), "install-app", "the app"),
            (crate::schedule::is_installed(), "schedule", "the schedule"),
        ];
        for (installed, command, what) in steps {
            if installed && let Err(err) = run(Command::new(&exe).arg(command)) {
                println!("  ! Couldn't rebuild {what} ({err:#}): run `gh watch {command}`.");
                warnings += 1;
            }
        }
    }
    println!(
        "Upgraded to gh-watch {}{}. What's new: {}",
        newer.version,
        if warnings > 0 {
            " (with the warnings above)"
        } else {
            ""
        },
        newer.url
    );
    Ok(())
}

/// Runs `command`, showing its output, and fails if it does.
fn run(command: &mut Command) -> Result<()> {
    let program = command.get_program().to_string_lossy().into_owned();
    let status = command
        .status()
        .with_context(|| format!("could not run {program}"))?;
    if !status.success() {
        bail!("{program} failed ({status})");
    }
    Ok(())
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
    let body = match upgrade_command() {
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
    fn tells_how_it_was_installed() {
        let bin = Path::new("/Users/me/.cargo/bin");
        assert_eq!(
            Install::of(
                Path::new("/Users/me/.local/share/gh/extensions/gh-watch/gh-watch"),
                Some(bin)
            ),
            Install::Extension
        );
        assert_eq!(
            Install::of(Path::new("/Users/me/.cargo/bin/gh-watch"), Some(bin)),
            Install::Cargo
        );
        assert_eq!(
            Install::of(Path::new("/tmp/build/gh-watch"), Some(bin)),
            Install::Unknown
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
