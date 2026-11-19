//! gh-watch: what changed on the GitHub issues you care about, through the
//! `gh` CLI (so there's no token to set up).
//!
//! - [`gh`]: issues by label, issues you're involved in, comments, and who
//!   you are, as `gh` sees them.
//! - [`changes`]: which of those issues really changed since a moment:
//!   closed, or commented on by someone other than you or a bot.
//!
//! The `gh-watch` command (and `gh watch`, as a gh extension) is built on
//! these and adds notifications. Tools that do their own notifying can use
//! the library alone.

pub mod changes;
pub mod gh;
