//! What changed on issues since a moment, leaving out what nobody needs a
//! ping about.
//!
//! GitHub calls an issue updated for anything: a relabel, a bot's comment,
//! your own comment. A change here is only one of:
//! - the issue was closed (and which PR closed it, if one did), or
//! - someone other than you, and not a bot, commented.

use std::collections::HashSet;

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::gh::{self, Comment, Issue, PrRef};

#[derive(Debug, Clone)]
pub struct Change {
    pub issue: Issue,
    /// Closed since the moment.
    pub closed: bool,
    /// Comments since the moment from other people, oldest first.
    pub comments: Vec<Comment>,
}

impl Change {
    /// The PR that closed the issue, if it was closed by one.
    pub fn closed_by(&self) -> Option<&PrRef> {
        self.closed.then(|| self.issue.prs.first()).flatten()
    }

    /// Where to look: the newest comment, else the issue.
    pub fn url(&self) -> &str {
        self.comments
            .last()
            .map(|c| c.html_url.as_str())
            .filter(|u| !u.is_empty())
            .unwrap_or(&self.issue.url)
    }

    /// Who commented, each once, in order.
    pub fn commenters(&self) -> Vec<&str> {
        let mut people: Vec<&str> = Vec::new();
        for c in &self.comments {
            if !people.contains(&c.user.login.as_str()) {
                people.push(&c.user.login);
            }
        }
        people
    }
}

/// The change on `issue` since `since`, given its comments written since
/// then, or None if nothing worth a ping happened. `me` is your login.
pub fn change(
    issue: &Issue,
    comments: &[Comment],
    me: &str,
    since: DateTime<Utc>,
) -> Option<Change> {
    let theirs: Vec<Comment> = comments
        .iter()
        .filter(|c| !c.is_bot() && !c.user.login.eq_ignore_ascii_case(me))
        .cloned()
        .collect();
    let closed = !issue.is_open() && issue.closed_at.is_some_and(|at| at > since);
    (closed || !theirs.is_empty()).then(|| Change {
        issue: issue.clone(),
        closed,
        comments: theirs,
    })
}

/// The changes on `issues` since `since`, fetching each one's comments.
/// Issues not updated since then are skipped without asking GitHub. An
/// issue whose comments can't be fetched is reported in the second list
/// rather than failing the rest.
pub fn changes(
    repo: &str,
    issues: &[Issue],
    me: &str,
    since: DateTime<Utc>,
) -> (Vec<Change>, Vec<(u64, anyhow::Error)>) {
    let mut found = Vec::new();
    let mut failed = Vec::new();
    for issue in issues.iter().filter(|i| i.updated_at > since) {
        match gh::comments_since(repo, issue.number, since) {
            Ok(comments) => {
                let Some(mut c) = change(issue, &comments, me, since) else {
                    continue;
                };
                // Search results leave out the PRs that close an issue, so
                // look a closed one up to say which PR it was.
                if c.closed
                    && c.issue.prs.is_empty()
                    && let Ok(full) = gh::issue(repo, issue.number)
                {
                    c.issue.prs = full.prs;
                }
                found.push(c);
            }
            Err(err) => failed.push((issue.number, err)),
        }
    }
    (found, failed)
}

/// What to watch in a repo.
#[derive(Debug, Clone, Default)]
pub struct Watch {
    /// Issues with any of these labels.
    pub labels: Vec<String>,
    /// Issues you're involved in: opened, assigned, mentioned, commented.
    pub involved: bool,
}

/// The issues `watch` covers in `repo` that were updated since `since`,
/// each once.
pub fn watched(repo: &str, watch: &Watch, since: DateTime<Utc>) -> Result<Vec<Issue>> {
    let mut lists = Vec::new();
    for label in &watch.labels {
        lists.push(gh::updated_with_label(repo, label, since)?);
    }
    if watch.involved {
        lists.push(gh::involved(repo, since)?);
    }
    Ok(unique(lists.into_iter().flatten()))
}

/// Each issue once (by number), first one kept.
pub fn unique(issues: impl IntoIterator<Item = Issue>) -> Vec<Issue> {
    let mut seen = HashSet::new();
    issues
        .into_iter()
        .filter(|i| seen.insert(i.number))
        .collect()
}

/// Text cut to fit a notification line: whitespace squeezed, then cut at
/// a word with "…" if it's longer than `max` characters.
pub fn preview(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::{CommentAuthor, Label};
    use chrono::TimeDelta;

    pub fn issue(number: u64, open: bool, closed_ago: Option<i64>) -> Issue {
        Issue {
            number,
            title: String::from("Slow exports"),
            state: String::from(if open { "OPEN" } else { "CLOSED" }),
            url: format!("https://github.com/acme/app/issues/{number}"),
            updated_at: Utc::now(),
            closed_at: closed_ago.map(|h| Utc::now() - TimeDelta::hours(h)),
            labels: vec![Label {
                name: String::from("customer"),
            }],
            assignees: Vec::new(),
            prs: Vec::new(),
        }
    }

    fn comment(login: &str, kind: &str) -> Comment {
        Comment {
            user: CommentAuthor {
                login: login.into(),
                kind: kind.into(),
            },
            created_at: Utc::now(),
            body: String::from("Looks fixed"),
            html_url: format!("https://github.com/acme/app/issues/7#{login}"),
        }
    }

    fn since() -> DateTime<Utc> {
        Utc::now() - TimeDelta::hours(2)
    }

    #[test]
    fn closing_counts_and_names_the_pr() {
        let mut i = issue(7, false, Some(1));
        i.prs.push(PrRef {
            number: 9,
            url: String::new(),
        });
        let c = change(&i, &[], "me", since()).unwrap();
        assert!(c.closed);
        assert_eq!(c.closed_by().map(|p| p.number), Some(9));
        assert_eq!(c.url(), "https://github.com/acme/app/issues/7");
        // Closed before the moment: old news.
        assert!(change(&issue(7, false, Some(5)), &[], "me", since()).is_none());
    }

    #[test]
    fn only_other_peoples_comments_count() {
        let i = issue(7, true, None);
        assert!(
            change(
                &i,
                &[comment("ME", "User"), comment("ci[bot]", "Bot")],
                "me",
                since()
            )
            .is_none()
        );
        let c = change(
            &i,
            &[
                comment("sally", "User"),
                comment("amir", "User"),
                comment("sally", "User"),
            ],
            "me",
            since(),
        )
        .unwrap();
        assert_eq!(c.commenters(), ["sally", "amir"]);
        assert_eq!(c.url(), "https://github.com/acme/app/issues/7#sally");
    }

    #[test]
    fn unique_keeps_the_first() {
        let mut a = issue(1, true, None);
        a.title = String::from("first");
        let issues = unique([a, issue(1, true, None), issue(2, true, None)]);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].title, "first");
    }

    #[test]
    fn previews_cut_at_a_word() {
        assert_eq!(preview("  two\n lines ", 50), "two lines");
        assert_eq!(preview("one two three", 9), "one two…");
    }
}
