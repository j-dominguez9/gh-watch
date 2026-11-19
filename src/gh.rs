//! GitHub issues through the `gh` CLI: it's already logged in, so there's
//! no token to set up. Everything here runs `gh ... --json` (or `gh api`)
//! and parses the result.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// The fields asked of `gh issue list/view`, matching `Issue`.
const FIELDS: &str =
    "number,title,state,url,updatedAt,closedAt,labels,assignees,closedByPullRequestsReferences";
/// What `gh search issues` can return, matching `Issue` (no PR links).
const SEARCH_FIELDS: &str = "number,title,state,url,updatedAt,closedAt,labels,assignees";

/// More than any one label has open; gh's default of 30 would cut lists short.
const OPEN_LIMIT: &str = "500";
const CLOSED_LIMIT: &str = "100";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub number: u64,
    pub title: String,
    /// "OPEN" or "CLOSED" (search results say "open" or "closed").
    pub state: String,
    pub url: String,
    pub updated_at: DateTime<Utc>,
    /// Search results give open issues a placeholder date here, so check
    /// `is_open` first.
    pub closed_at: Option<DateTime<Utc>>,
    pub labels: Vec<Label>,
    pub assignees: Vec<Person>,
    /// PRs that close this issue when merged: still open while the issue
    /// is, merged once it's closed. Search results don't include them.
    #[serde(rename = "closedByPullRequestsReferences", default)]
    pub prs: Vec<PrRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Label {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Person {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrRef {
    pub number: u64,
    pub url: String,
}

impl Issue {
    pub fn is_open(&self) -> bool {
        self.state.eq_ignore_ascii_case("open")
    }

    pub fn has_label(&self, name: &str) -> bool {
        self.labels
            .iter()
            .any(|l| l.name.eq_ignore_ascii_case(name))
    }

    /// The title without a leading "[Something]" tag (and a ":" or "-"
    /// after it), when there's more to it than the tag.
    pub fn short_title(&self) -> &str {
        let title = self.title.trim_start();
        if title.starts_with('[')
            && let Some(end) = title.find(']')
            && let rest = title[end + 1..].trim_start_matches([' ', ':', '-'])
            && !rest.is_empty()
        {
            return rest;
        }
        title
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Comment {
    pub user: CommentAuthor,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub body: String,
    /// A link straight to the comment.
    #[serde(default)]
    pub html_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommentAuthor {
    pub login: String,
    #[serde(rename = "type")]
    pub kind: String,
}

impl Comment {
    pub fn is_bot(&self) -> bool {
        self.user.kind == "Bot" || self.user.login.ends_with("[bot]")
    }
}

/// Your GitHub login, as `gh` is logged in.
pub fn me() -> Result<String> {
    #[derive(Deserialize)]
    struct User {
        login: String,
    }
    let user: User = gh(&["api", "user"])?;
    Ok(user.login)
}

/// Every label in `repo` starting with `prefix`, lowercased.
pub fn labels(repo: &str, prefix: &str) -> Result<HashSet<String>> {
    #[derive(Deserialize)]
    struct Named {
        name: String,
    }
    let labels: Vec<Named> = gh(&[
        "label", "list", "-R", repo, "--search", prefix, "--limit", "2000", "--json", "name",
    ])?;
    Ok(labels
        .into_iter()
        .map(|l| l.name.to_lowercase())
        .filter(|name| name.starts_with(&prefix.to_lowercase()))
        .collect())
}

/// Issues with `label`: all open ones, and the ones closed on or after
/// `closed_since`.
pub fn with_label(repo: &str, label: &str, closed_since: NaiveDate) -> Result<Vec<Issue>> {
    let mut issues: Vec<Issue> = gh(&[
        "issue", "list", "-R", repo, "-l", label, "--state", "open", "--limit", OPEN_LIMIT,
        "--json", FIELDS,
    ])?;
    let closed: Vec<Issue> = gh(&[
        "issue",
        "list",
        "-R",
        repo,
        "-l",
        label,
        "--state",
        "closed",
        "--search",
        &format!("closed:>={closed_since}"),
        "--limit",
        CLOSED_LIMIT,
        "--json",
        FIELDS,
    ])?;
    issues.extend(closed);
    Ok(issues)
}

/// Issues with `label`, open or closed, updated since `since`.
pub fn updated_with_label(repo: &str, label: &str, since: DateTime<Utc>) -> Result<Vec<Issue>> {
    gh(&[
        "issue",
        "list",
        "-R",
        repo,
        "-l",
        label,
        "--state",
        "all",
        "--search",
        &format!("updated:>={}", timestamp(since)),
        "--limit",
        OPEN_LIMIT,
        "--json",
        FIELDS,
    ])
}

/// Issues in `repo` you're involved in (opened, assigned, mentioned or
/// commented on) that were updated since `since`.
pub fn involved(repo: &str, since: DateTime<Utc>) -> Result<Vec<Issue>> {
    let updated = format!(">={}", timestamp(since));
    gh(&[
        "search",
        "issues",
        "--repo",
        repo,
        "--involves",
        "@me",
        "--updated",
        &updated,
        "--limit",
        "100",
        "--json",
        SEARCH_FIELDS,
    ])
}

/// The repo of the current directory, as owner/name.
pub fn current_repo() -> Result<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Repo {
        name_with_owner: String,
    }
    let repo: Repo = gh(&["repo", "view", "--json", "nameWithOwner"])?;
    Ok(repo.name_with_owner)
}

/// One issue.
pub fn issue(repo: &str, number: u64) -> Result<Issue> {
    gh(&[
        "issue",
        "view",
        &number.to_string(),
        "-R",
        repo,
        "--json",
        FIELDS,
    ])
}

/// An issue's comments written after `since`, oldest first.
pub fn comments_since(repo: &str, number: u64, since: DateTime<Utc>) -> Result<Vec<Comment>> {
    // `since` here filters on when a comment was last *updated*, so edits
    // to old comments come back too: keep only ones written after it.
    let path = format!(
        "repos/{repo}/issues/{number}/comments?since={}&per_page=100",
        timestamp(since)
    );
    let comments: Vec<Comment> = gh(&["api", &path])?;
    Ok(comments
        .into_iter()
        .filter(|c| c.created_at > since)
        .collect())
}

fn timestamp(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Runs gh with `args` and parses its JSON output.
fn gh<T: DeserializeOwned>(args: &[&str]) -> Result<T> {
    let program = gh_path().context(
        "GitHub CLI not found: install it (https://cli.github.com) and run `gh auth login`",
    )?;
    let output = Command::new(program)
        .args(args)
        .output()
        .context("could not run gh")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("gh {} failed: {}", args[..2].join(" "), stderr.trim());
    }
    serde_json::from_slice(&output.stdout).context("could not parse gh's output")
}

/// Where gh is: the one running us (as a gh extension), on PATH, or in
/// Homebrew's folders (for scheduled runs, whose PATH doesn't include them).
fn gh_path() -> Option<PathBuf> {
    if let Some(gh) = std::env::var_os("GH_PATH").map(PathBuf::from)
        && gh.is_file()
    {
        return Some(gh);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ])
        .map(|dir| dir.join("gh"))
        .find(|program| program.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape `gh issue list --json` returns, trimmed.
    const SAMPLE: &str = r#"[{
        "number": 84848,
        "title": "[Feature Request] Add traces",
        "state": "OPEN",
        "url": "https://github.com/acme/app/issues/84848",
        "updatedAt": "2026-08-26T18:29:47Z",
        "closedAt": null,
        "labels": [{"id": "x", "name": "Customer", "description": "", "color": "AB8E2F"}],
        "assignees": [{"id": "u", "login": "sally", "name": ""}],
        "closedByPullRequestsReferences": [
            {"id": "p", "number": 92864, "url": "https://github.com/acme/app/pull/92864",
             "repository": {"name": "app"}}
        ]
    }]"#;

    fn sample() -> Issue {
        serde_json::from_str::<Vec<Issue>>(SAMPLE)
            .unwrap()
            .remove(0)
    }

    #[test]
    fn parses_gh_output() {
        let issue = sample();
        assert!(issue.is_open());
        assert!(issue.has_label("customer"));
        assert_eq!(issue.assignees[0].login, "sally");
        assert_eq!(issue.prs[0].number, 92864);
    }

    #[test]
    fn parses_search_results_without_pr_links() {
        let issues: Vec<Issue> = serde_json::from_str(
            r#"[{"number": 1, "title": "T", "state": "open", "url": "u",
                "updatedAt": "2026-11-11T20:21:48Z", "closedAt": "0001-01-01T00:00:00Z",
                "labels": [], "assignees": []}]"#,
        )
        .unwrap();
        assert!(issues[0].is_open());
        assert!(issues[0].prs.is_empty());
    }

    #[test]
    fn short_title_drops_the_tag() {
        let mut issue = sample();
        assert_eq!(issue.short_title(), "Add traces");
        issue.title = String::from("[Bug] : Dangling colon");
        assert_eq!(issue.short_title(), "Dangling colon");
        issue.title = String::from("[Only a tag]");
        assert_eq!(issue.short_title(), "[Only a tag]");
    }

    #[test]
    fn bots_are_spotted_by_type_or_name() {
        let comment = |login: &str, kind: &str| Comment {
            user: CommentAuthor {
                login: login.into(),
                kind: kind.into(),
            },
            created_at: Utc::now(),
            body: String::new(),
            html_url: String::new(),
        };
        assert!(comment("linear", "Bot").is_bot());
        assert!(comment("github-actions[bot]", "User").is_bot());
        assert!(!comment("sally", "User").is_bot());
    }
}
