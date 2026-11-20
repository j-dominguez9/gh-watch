//! What to watch, in ~/.config/gh-watch/config.toml:
//!
//! ```toml
//! # Repos to watch, as owner/name.
//! repos = ["owner/repo"]
//! # Also watch issues with these labels (in every repo above).
//! labels = ["customer-acme"]
//! # Issues you're involved in: opened, assigned, mentioned, commented on.
//! involved = true
//! ```
//!
//! `gh watch setup` writes it and `gh watch config` changes it. Its one
//! rule, which `save` enforces so a broken config never reaches a
//! scheduled run: a repo, and something to watch in it.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use gh_watch::changes::Watch;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default = "yes")]
    pub involved: bool,
}

fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            repos: Vec::new(),
            labels: Vec::new(),
            involved: true,
        }
    }
}

impl Config {
    pub fn path() -> Result<PathBuf> {
        let home = dirs::home_dir().context("could not find your home folder")?;
        Ok(home.join(".config/gh-watch/config.toml"))
    }

    /// The saved config, or None if there isn't one yet.
    pub fn read() -> Result<Option<Config>> {
        let path = Self::path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text)
                .map(Some)
                .with_context(|| format!("could not parse {}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err).context(format!("could not read {}", path.display())),
        }
    }

    /// Whether it can be used: a repo, and something to watch in it.
    pub fn check(&self) -> Result<()> {
        if self.repos.is_empty() {
            bail!("no repo to watch");
        }
        if self.labels.is_empty() && !self.involved {
            bail!("nothing to watch (no labels, and not issues you're involved in)");
        }
        Ok(())
    }

    /// What to watch in each of `repos`.
    pub fn watch(&self) -> Watch {
        Watch {
            labels: self.labels.clone(),
            involved: self.involved,
        }
    }

    /// Adds the repos and labels it doesn't have yet (compared ignoring
    /// case). Returns whether anything was added.
    pub fn add(&mut self, repos: &[String], labels: &[String]) -> bool {
        let mut added = false;
        for repo in repos {
            if !self.repos.iter().any(|r| r.eq_ignore_ascii_case(repo)) {
                self.repos.push(repo.clone());
                added = true;
            }
        }
        for label in labels {
            if !self.labels.iter().any(|l| l.eq_ignore_ascii_case(label)) {
                self.labels.push(label.clone());
                added = true;
            }
        }
        added
    }

    /// Takes out the repos and labels given (ignoring case). Returns the
    /// ones that weren't there, to say so.
    pub fn remove(&mut self, repos: &[String], labels: &[String]) -> Vec<String> {
        let mut missing = Vec::new();
        for repo in repos {
            let before = self.repos.len();
            self.repos.retain(|r| !r.eq_ignore_ascii_case(repo));
            if self.repos.len() == before {
                missing.push(repo.clone());
            }
        }
        for label in labels {
            let before = self.labels.len();
            self.labels.retain(|l| !l.eq_ignore_ascii_case(label));
            if self.labels.len() == before {
                missing.push(label.clone());
            }
        }
        missing
    }

    /// Saves it, if it passes `check`. An existing file keeps its comments
    /// and layout; only the values change.
    pub fn save(&self) -> Result<()> {
        self.check()?;
        let path = Self::path()?;
        let text = match std::fs::read_to_string(&path) {
            Ok(old) => self.update(&old)?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => self.new_file(),
            Err(err) => return Err(err).context(format!("could not read {}", path.display())),
        };
        std::fs::create_dir_all(path.parent().context("bad config path")?)?;
        std::fs::write(&path, text).with_context(|| format!("could not write {}", path.display()))
    }

    /// `old` (the file as it is) with this config's values.
    fn update(&self, old: &str) -> Result<String> {
        let mut doc: toml_edit::DocumentMut = old.parse().with_context(|| {
            format!(
                "could not parse {}",
                Self::path().unwrap_or_default().display()
            )
        })?;
        doc["repos"] = toml_edit::value(array(&self.repos));
        // No labels and no `labels` line: leave it out (the new file's
        // example stays a comment) rather than writing `labels = []`.
        if !self.labels.is_empty() || doc.contains_key("labels") {
            doc["labels"] = toml_edit::value(array(&self.labels));
        }
        doc["involved"] = toml_edit::value(self.involved);
        Ok(doc.to_string())
    }

    /// A first config file, with a comment for each setting.
    fn new_file(&self) -> String {
        let labels = if self.labels.is_empty() {
            String::from("# labels = [\"customer-acme\"]")
        } else {
            format!("labels = {}", array(&self.labels))
        };
        format!(
            "# Repos to watch, as owner/name.\nrepos = {}\n\
             # Also watch issues with these labels (in every repo above). Add more with\n\
             # `gh watch config add --label <name>`.\n{labels}\n\
             # Issues you're involved in: opened, assigned, mentioned, commented on.\n\
             involved = {}\n",
            array(&self.repos),
            self.involved
        )
    }

    /// "issues you're involved in, and labels customer-acme, bug".
    pub fn describe(&self) -> String {
        let labels = match self.labels.as_slice() {
            [] => None,
            [one] => Some(format!("label {one}")),
            many => Some(format!("labels {}", many.join(", "))),
        };
        match (self.involved, labels) {
            (true, Some(labels)) => format!("issues you're involved in, and {labels}"),
            (true, None) => String::from("issues you're involved in"),
            (false, Some(labels)) => labels,
            (false, None) => String::from("nothing"),
        }
    }
}

fn array(items: &[String]) -> toml_edit::Array {
    items.iter().map(String::as_str).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(repos: &[&str], labels: &[&str], involved: bool) -> Config {
        Config {
            repos: repos.iter().map(|s| s.to_string()).collect(),
            labels: labels.iter().map(|s| s.to_string()).collect(),
            involved,
        }
    }

    #[test]
    fn adding_keeps_whats_there_and_skips_repeats() {
        let mut c = config(&["o/r"], &["customer-optum"], true);
        assert!(c.add(&[String::from("o/r")], &[String::from("customer-att")]));
        assert!(!c.add(&[], &[String::from("Customer-Optum")]));
        assert_eq!(c.repos, ["o/r"]);
        assert_eq!(c.labels, ["customer-optum", "customer-att"]);
    }

    #[test]
    fn removing_says_what_wasnt_there() {
        let mut c = config(&["o/r"], &["customer-optum", "customer-att"], true);
        let missing = c.remove(&[], &[String::from("CUSTOMER-OPTUM"), String::from("nope")]);
        assert_eq!(missing, ["nope"]);
        assert_eq!(c.labels, ["customer-att"]);
    }

    #[test]
    fn a_usable_config_has_a_repo_and_something_to_watch() {
        assert!(config(&["o/r"], &[], true).check().is_ok());
        assert!(config(&["o/r"], &["x"], false).check().is_ok());
        assert!(config(&[], &["x"], true).check().is_err());
        assert!(config(&["o/r"], &[], false).check().is_err());
    }

    #[test]
    fn a_new_file_reads_back_the_same() {
        let c = config(&["Arize-ai/arize"], &["customer-at&t", "say \"hi\""], false);
        let back: Config = toml::from_str(&c.new_file()).unwrap();
        assert_eq!(back.repos, c.repos);
        assert_eq!(back.labels, c.labels);
        assert!(!back.involved);
        // No labels yet: the example stays a comment.
        let empty = config(&["o/r"], &[], true).new_file();
        assert!(empty.contains("# labels = ["));
        assert!(toml::from_str::<Config>(&empty).unwrap().labels.is_empty());
    }

    #[test]
    fn updating_keeps_comments_and_changes_only_values() {
        let old = "# my note\nrepos = [\"o/r\"] # the main one\n\n# who cares\ninvolved = true\n";
        let c = config(&["o/r", "o/two"], &["bug"], false);
        let new = c.update(old).unwrap();
        assert!(new.contains("# my note"));
        assert!(new.contains("# who cares"));
        let back: Config = toml::from_str(&new).unwrap();
        assert_eq!(back.repos, ["o/r", "o/two"]);
        assert_eq!(back.labels, ["bug"]);
        assert!(!back.involved);
        // No labels, and none in the file: still no `labels` line.
        let new = config(&["o/r"], &[], true).update(old).unwrap();
        assert!(!new.contains("labels"));
    }

    #[test]
    fn says_what_it_watches() {
        let mut c = config(&["o/r"], &[], true);
        assert_eq!(c.describe(), "issues you're involved in");
        c.labels = vec![String::from("a"), String::from("b")];
        assert_eq!(c.describe(), "issues you're involved in, and labels a, b");
    }

    #[test]
    fn an_empty_file_watches_what_youre_involved_in() {
        let c: Config = toml::from_str("").unwrap();
        assert!(c.involved && c.repos.is_empty() && c.labels.is_empty());
    }
}
