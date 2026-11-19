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
