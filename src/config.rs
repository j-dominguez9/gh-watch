//! Settings in ~/.config/gh-watch/config.toml, all optional:
//!
//! ```toml
//! # Repos to watch, as owner/name. Default: the repo you run it in.
//! repos = ["owner/repo"]
//! # Also watch issues with these labels, in every repo above.
//! labels = ["needs-review"]
//! # Issues you're involved in: opened, assigned, mentioned, commented on.
//! involved = true
//! ```

use std::path::PathBuf;

use anyhow::{Context, Result};
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

    pub fn exists() -> Result<bool> {
        Ok(Self::path()?.exists())
    }

    /// Adds the repos and labels it doesn't have yet (labels compared
    /// ignoring case). Returns whether anything was added.
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

    /// Writes the config, replacing the file. It's written with the
    /// standard comments, so comments added by hand don't survive.
    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        std::fs::create_dir_all(path.parent().context("bad config path")?)?;
        std::fs::write(&path, self.to_toml())
            .with_context(|| format!("could not write {}", path.display()))
    }

    fn to_toml(&self) -> String {
        let list = |items: &[String]| {
            items
                .iter()
                .map(|item| toml::Value::from(item.as_str()).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let labels = if self.labels.is_empty() {
            String::from("# labels = [\"customer-acme\"]")
        } else {
            format!("labels = [{}]", list(&self.labels))
        };
        format!(
            "# Repos to watch, as owner/name.\nrepos = [{}]\n\
             # Also watch issues with these labels (in every repo above). Add more with\n\
             # `gh watch config add --label <name>`.\n{labels}\n\
             # Issues you're involved in: opened, assigned, mentioned, commented on.\n\
             involved = {}\n",
            list(&self.repos),
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

    /// The config, or the defaults if there's no file.
    pub fn load() -> Result<Config> {
        let path = Self::path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("could not parse {}", path.display()))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(err) => Err(err).context(format!("could not read {}", path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_keeps_whats_there_and_skips_repeats() {
        let mut config = Config {
            repos: vec![String::from("o/r")],
            labels: vec![String::from("customer-optum")],
            involved: true,
        };
        assert!(config.add(&[String::from("o/r")], &[String::from("customer-att")]));
        assert!(!config.add(&[], &[String::from("Customer-Optum")]));
        assert_eq!(config.repos, ["o/r"]);
        assert_eq!(config.labels, ["customer-optum", "customer-att"]);
    }

    #[test]
    fn removing_says_what_wasnt_there() {
        let mut config = Config {
            repos: vec![String::from("o/r")],
            labels: vec![String::from("customer-optum"), String::from("customer-att")],
            involved: true,
        };
        let missing = config.remove(&[], &[String::from("CUSTOMER-OPTUM"), String::from("nope")]);
        assert_eq!(missing, ["nope"]);
        assert_eq!(config.labels, ["customer-att"]);
        assert_eq!(config.repos, ["o/r"]);
    }

    #[test]
    fn saved_config_reads_back_the_same() {
        let config = Config {
            repos: vec![String::from("Arize-ai/arize")],
            labels: vec![String::from("customer-at&t"), String::from("say \"hi\"")],
            involved: false,
        };
        let back: Config = toml::from_str(&config.to_toml()).unwrap();
        assert_eq!(back.repos, config.repos);
        assert_eq!(back.labels, config.labels);
        assert!(!back.involved);
        // No labels yet: the example stays a comment.
        let empty = Config::default().to_toml();
        assert!(empty.contains("# labels = ["));
        assert!(toml::from_str::<Config>(&empty).unwrap().labels.is_empty());
    }

    #[test]
    fn says_what_it_watches() {
        let mut config = Config::default();
        assert_eq!(config.describe(), "issues you're involved in");
        config.labels = vec![String::from("customer-optum"), String::from("customer-att")];
        assert_eq!(
            config.describe(),
            "issues you're involved in, and labels customer-optum, customer-att"
        );
    }

    #[test]
    fn empty_config_watches_what_youre_involved_in() {
        let config: Config = toml::from_str("").unwrap();
        assert!(config.involved && config.repos.is_empty() && config.labels.is_empty());
    }

    #[test]
    fn parses_repos_and_labels() {
        let config: Config = toml::from_str(
            "repos = [\"acme/app\"]\nlabels = [\"customer-acme\"]\ninvolved = false",
        )
        .unwrap();
        assert_eq!(config.repos, ["acme/app"]);
        assert_eq!(config.labels, ["customer-acme"]);
        assert!(!config.involved);
    }
}
