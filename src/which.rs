//! Finding a program: on PATH, then in folders given. Scheduled runs
//! (launchd) start with a bare PATH, so callers name the usual places too
//! (Homebrew's, Cargo's).

use std::path::PathBuf;

/// Where `name` is: the first match on PATH, then in `extra` folders.
pub fn which(name: &str, extra: &[PathBuf]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(extra.iter().cloned())
        .map(|dir| dir.join(name))
        .find(|program| program.is_file())
}

/// Homebrew's folders, Apple Silicon's and Intel's.
pub fn homebrew() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]
}

/// Cargo's bin folder: `$CARGO_HOME/bin`, else `~/.cargo/bin`.
pub fn cargo_bin() -> Option<PathBuf> {
    std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".cargo")))
        .map(|cargo| cargo.join("bin"))
}
