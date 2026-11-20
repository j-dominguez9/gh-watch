//! How a change is put into words, for notifications and the terminal
//! alike: a headline saying who did what where ("sally commented on arize
//! (#48765)"), and a body with what they said.

use chrono::{DateTime, Utc};
use gh_watch::changes::{self, Change};

/// How much of a comment the body shows.
const PREVIEW_CHARS: usize = 160;

/// Who did what, where: "sally commented on arize (#48765)", "arize
/// (#48765) closed by PR #94655". Short, so a notification never cuts it.
pub fn headline(repo: &str, c: &Change) -> String {
    let issue = format!("{} (#{})", short_repo(repo), c.issue.number);
    if c.closed {
        return match c.closed_by() {
            Some(pr) => format!("{issue} closed by PR #{}", pr.number),
            None => format!("{issue} closed"),
        };
    }
    let people = c.commenters();
    match (c.comments.len(), people.as_slice()) {
        (1, [who]) => format!("{who} commented on {issue}"),
        (n, [who]) => format!("{n} new comments on {issue} from {who}"),
        (n, many) => format!("{n} new comments on {issue} from {}", many.join(", ")),
    }
}

/// What was said: the newest comment in its writer's words (with their
/// name when the headline doesn't make it clear), or for a close without
/// comments, the issue's title.
pub fn body(c: &Change) -> String {
    let Some(last) = c.comments.last() else {
        return c.issue.short_title().to_string();
    };
    let text = changes::preview(&last.body, PREVIEW_CHARS);
    if c.comments.len() == 1 && !c.closed {
        text // the headline already says who
    } else {
        format!("{}: {text}", last.user.login)
    }
}

/// The rest, when there are too many to notify about one by one, by
/// repo: "arize: #93337, #95251 · web: #12".
pub fn summary(rest: &[(String, Change)]) -> String {
    let mut by_repo: Vec<(&str, Vec<String>)> = Vec::new();
    for (repo, c) in rest {
        let number = format!("#{}", c.issue.number);
        match by_repo.iter_mut().find(|(r, _)| *r == short_repo(repo)) {
            Some((_, numbers)) => numbers.push(number),
            None => by_repo.push((short_repo(repo), vec![number])),
        }
    }
    by_repo
        .iter()
        .map(|(repo, numbers)| format!("{repo}: {}", numbers.join(", ")))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Where the summary goes: GitHub's issue search for those repos, updated
/// since `since`, newest first. That shows more than the changes counted
/// (anything updated), but has every one of them, which no search can
/// narrow to just those issues.
pub fn summary_link(rest: &[(String, Change)], since: DateTime<Utc>) -> String {
    let mut repos: Vec<&str> = Vec::new();
    for (repo, _) in rest {
        if !repos.contains(&repo.as_str()) {
            repos.push(repo);
        }
    }
    let mut query: Vec<String> = repos.iter().map(|r| format!("repo:{r}")).collect();
    query.push(String::from("is:issue"));
    query.push(format!("updated:>={}", since.format("%Y-%m-%d")));
    query.push(String::from("sort:updated-desc"));
    let q: String = query
        .join(" ")
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' => {
                (b as char).to_string()
            }
            b' ' => String::from("+"),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("https://github.com/issues?q={q}")
}

/// "Arize-ai/arize" -> "arize", to keep headlines short.
fn short_repo(repo: &str) -> &str {
    repo.rsplit('/').next().unwrap_or(repo)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn change(closed: bool, pr: Option<u64>, comments: &[(&str, &str)]) -> Change {
        let mut issue: gh_watch::gh::Issue = serde_json::from_str(
            r#"{"number": 48765, "title": "[Bug] Max limit counts spans", "state": "OPEN",
                "url": "u", "updatedAt": "2026-11-18T00:00:00Z", "closedAt": null,
                "labels": [], "assignees": []}"#,
        )
        .unwrap();
        if let Some(n) = pr {
            issue.prs.push(gh_watch::gh::PrRef {
                number: n,
                url: String::new(),
            });
        }
        let comments = comments
            .iter()
            .map(|(who, body)| {
                serde_json::from_value(serde_json::json!({
                    "user": {"login": who, "type": "User"},
                    "created_at": "2026-11-18T00:00:00Z", "body": body, "html_url": ""
                }))
                .unwrap()
            })
            .collect();
        Change {
            issue,
            closed,
            comments,
        }
    }

    pub const REPO: &str = "Arize-ai/arize";

    #[test]
    fn closes_name_the_pr_and_show_the_title() {
        let c = change(true, Some(94655), &[]);
        assert_eq!(headline(REPO, &c), "arize (#48765) closed by PR #94655");
        assert_eq!(body(&c), "Max limit counts spans");
        assert_eq!(
            headline(REPO, &change(true, None, &[])),
            "arize (#48765) closed"
        );
    }

    #[test]
    fn one_comment_names_its_writer_in_the_headline() {
        let c = change(false, None, &[("sally", "Looks  fixed\nnow")]);
        assert_eq!(headline(REPO, &c), "sally commented on arize (#48765)");
        assert_eq!(body(&c), "Looks fixed now");
    }

    #[test]
    fn several_comments_are_counted_and_the_newest_shown() {
        let c = change(
            false,
            None,
            &[("sally", "a"), ("amir", "b"), ("sally", "c")],
        );
        assert_eq!(
            headline(REPO, &c),
            "3 new comments on arize (#48765) from sally, amir"
        );
        assert_eq!(body(&c), "sally: c");
        let c = change(false, None, &[("sally", "a"), ("sally", "b")]);
        assert_eq!(
            headline(REPO, &c),
            "2 new comments on arize (#48765) from sally"
        );
    }

    #[test]
    fn a_close_with_comments_shows_the_comment() {
        let c = change(true, Some(9), &[("sally", "Shipped")]);
        assert_eq!(headline(REPO, &c), "arize (#48765) closed by PR #9");
        assert_eq!(body(&c), "sally: Shipped");
    }

    #[test]
    fn summary_groups_numbers_by_repo() {
        let rest = vec![
            (REPO.to_string(), change(true, None, &[])),
            ("acme/web".to_string(), change(true, None, &[])),
        ];
        assert_eq!(summary(&rest), "arize: #48765 · web: #48765");
    }

    #[test]
    fn the_summary_links_to_every_repo_updated_since() {
        let rest = vec![
            (REPO.to_string(), change(true, None, &[])),
            (REPO.to_string(), change(true, None, &[])),
            ("acme/web".to_string(), change(true, None, &[])),
        ];
        let since = DateTime::parse_from_rfc3339("2026-11-18T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            summary_link(&rest, since),
            "https://github.com/issues?q=repo%3AArize-ai/arize+repo%3Aacme/web+is%3Aissue\
             +updated%3A%3E%3D2026-11-18+sort%3Aupdated-desc"
        );
    }

    #[test]
    fn short_repo_names() {
        assert_eq!(short_repo("Arize-ai/arize"), "arize");
        assert_eq!(short_repo("solo"), "solo");
    }
}
