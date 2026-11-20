//! Deciding what to watch: `gh watch setup` (everything in one go, safe to
//! run again) and `gh watch config` (show it, or add and remove repos and
//! labels). Both only ever add to the config or take out what's named, and
//! warn about labels a repo doesn't have.

use anyhow::{Result, bail};
use chrono::Utc;
use gh_watch::gh;

use crate::config::Config;
use crate::state::State;
use crate::{app, schedule};

/// `gh watch setup`: the config (adding `repos`, else the repo you're in,
/// and `labels`), the notifier app (macOS), the schedule, and the clock.
/// Each step says what it did, and one that fails doesn't stop the rest.
pub fn setup(repos: &[String], labels: &[String], no_involved: bool) -> Result<()> {
    let existing = Config::read()?;
    let existed = existing.is_some();
    let mut config = existing.unwrap_or_default();
    // The repo you're in counts even when the config has others, so
    // running setup in a second repo adds it.
    let repos = if !repos.is_empty() {
        repos.to_vec()
    } else if let Ok(here) = gh::current_repo() {
        vec![here]
    } else if !config.repos.is_empty() {
        config.repos.clone()
    } else {
        bail!("no repo to watch: run this inside the repo, or pass --repo owner/name");
    };
    println!("Setting up gh-watch for {}:", repos.join(", "));

    // 1. Remember what to watch, so scheduled runs (which start outside
    //    any repo, with no flags) know.
    let config_path = Config::path()?;
    let mut changed = config.add(&repos, labels);
    if no_involved && config.involved {
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
    check_labels(&config, labels, &repos);

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
        match schedule::install() {
            Ok(()) => println!(
                "  ✓ Schedule: notify runs every 30 minutes (log: {})",
                schedule::log_path()?.display()
            ),
            Err(err) => println!(
                "  ! Schedule: not set up ({err:#}).\n    Nothing will check on its own \
                 until it is: try `gh watch schedule`, and `gh watch doctor` to see why."
            ),
        }
    } else {
        println!("  - Schedule: on Linux, run `gh-watch notify` from cron or a systemd timer.");
    }

    // 4. Start the clock, so the first scheduled run only reports what's new.
    let mut state = State::load()?;
    if state.start(Utc::now()) {
        state.save()?;
    }
    println!(
        "  ✓ Watching {}: {}, from now on.",
        config.repos.join(", "),
        config.describe()
    );
    println!("\nTo see what notifications look like, try: gh watch notify --since 2d");
    Ok(())
}

/// A change `gh watch config` was asked for.
pub struct Edit {
    /// Take these out, rather than add them.
    pub remove: bool,
    pub repos: Vec<String>,
    pub labels: Vec<String>,
    /// Issues you're involved in, too.
    pub involved: bool,
}

/// `gh watch config`: shows what's watched, after making `edit` if given.
/// Changes apply from the next run.
pub fn config_command(edit: Option<&Edit>) -> Result<()> {
    let path = Config::path()?;
    let existed = Config::read()?;
    if edit.is_none() && existed.is_none() {
        println!("No config yet: run `gh watch setup` in the repo to watch.");
        return Ok(());
    }
    let mut config = existed.unwrap_or_default();
    if let Some(edit) = edit {
        if edit.labels.is_empty() && edit.repos.is_empty() && !edit.involved {
            bail!(
                "say what to change: --label <name>, --repo owner/name (repeat for several), \
                 or --involved"
            );
        }
        let changed = if edit.remove {
            remove(&mut config, edit)
        } else {
            add(&mut config, edit)
        };
        // Only when something changed (saving rewrites the file), and never
        // a config that can't be used: scheduled runs would fail on it.
        if changed {
            if let Err(err) = config.check() {
                bail!(
                    "not saved: that would leave {err:#}, which scheduled runs would fail on. {}",
                    if edit.remove {
                        "Keep something, or stop gh-watch with `gh watch schedule --remove`."
                    } else {
                        "Add a repo too, with --repo owner/name."
                    }
                );
            }
            config.save()?;
        }
        if !edit.remove {
            check_labels(&config, &edit.labels, &edit.repos);
        }
    }
    let repos = if config.repos.is_empty() {
        String::from("no repos yet")
    } else {
        config.repos.join(", ")
    };
    println!("Watching {repos}: {}.", config.describe());
    println!("Config: {}", path.display());
    Ok(())
}

/// Returns whether anything changed.
fn add(config: &mut Config, edit: &Edit) -> bool {
    let mut changed = config.add(&edit.repos, &edit.labels);
    if edit.involved && !config.involved {
        config.involved = true;
        changed = true;
    }
    if !changed {
        println!("Already watching all of that.");
    }
    changed
}

/// Returns whether anything changed.
fn remove(config: &mut Config, edit: &Edit) -> bool {
    let missing = config.remove(&edit.repos, &edit.labels);
    for name in &missing {
        println!("Wasn't watching {name:?}.");
    }
    let mut changed = missing.len() < edit.repos.len() + edit.labels.len();
    if edit.involved && config.involved {
        config.involved = false;
        changed = true;
    }
    changed
}

/// Warns about watched labels a watched repo doesn't have: a typo would
/// otherwise quietly match nothing. Checks the `new_labels` in every repo,
/// and every label in the `new_repos`, with one label listing per repo.
fn check_labels(config: &Config, new_labels: &[String], new_repos: &[String]) {
    let is_in = |list: &[String], name: &str| list.iter().any(|x| x.eq_ignore_ascii_case(name));
    for repo in &config.repos {
        let new_repo = is_in(new_repos, repo);
        let check: Vec<&String> = config
            .labels
            .iter()
            .filter(|label| new_repo || is_in(new_labels, label))
            .collect();
        if check.is_empty() {
            continue;
        }
        match gh::all_labels(repo) {
            Ok(found) => {
                for label in check {
                    if !found.contains(&label.to_lowercase()) {
                        println!(
                            "  ! {repo} has no label {label:?}, so nothing will match it there. \
                             See `gh label list -R {repo}`."
                        );
                    }
                }
            }
            Err(err) => println!("  ! Couldn't check the labels in {repo} ({err:#})"),
        }
    }
}
