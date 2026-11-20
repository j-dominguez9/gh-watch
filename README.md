# gh-watch

Know when the GitHub issues you care about change, without GitHub's
notification firehose.

It watches issues you're involved in (opened, assigned, mentioned or
commented on) and, if you like, issues with certain labels. It only tells
you about real changes:

- **closed**, and by which PR, or
- **commented on by someone else**: not you, not a bot.

Relabels, your own comments and bot noise don't count.

It talks to GitHub through the [`gh` CLI](https://cli.github.com), so
there's no token to set up: if `gh` works, gh-watch works.

## Install

You need the [GitHub CLI](https://cli.github.com), logged in (`brew
install gh`, then `gh auth login`). On macOS, the notifier app also needs
Apple's command line tools (`xcode-select --install`); you likely have
them already, since they come with git.

```sh
gh extension install j-dominguez9/gh-watch
gh watch setup --repo owner/name    # or run `gh watch setup` inside the repo
gh watch doctor                     # checks it all, and sends a test notification
```

`setup` does everything in one go:

1. writes a config with that repo, so scheduled runs know what to watch;
2. on macOS, builds and installs gh-watch's notifier app, so notifications
   show under its own name and cat icon (macOS asks once for permission);
3. on macOS, runs `notify` every 30 minutes;
4. starts the clock, so you only hear about changes from now on.

Add `--label customer-acme` (repeat for several) to also watch issues
with those labels. To see real notifications right away: `gh watch notify
--since 2d`.

Without the command line tools, setup carries on, and notifications come
without the cat icon or clickable links (through
[terminal-notifier](https://github.com/julienXX/terminal-notifier) if it's
installed, or plain macOS notifications); `gh watch doctor` says how to
fix it.

Or with Cargo, from source: `cargo install --git
https://github.com/j-dominguez9/gh-watch`, then `gh-watch setup`. It's
`gh-watch` rather than `gh watch` everywhere then.

Built and tested for Apple Silicon Macs; there are also builds for Intel
Macs and Linux (where notifications use `notify-send`), less tried.

## Use

```sh
gh watch changes                    # what changed in the last day, in what you watch
gh watch changes --since 3h         # also 30m, 2d, 1w
gh watch changes -R owner/repo      # look at another repo instead, this once
gh watch changes -l needs-review    # or at issues with this label instead
gh watch notify                     # notify about changes since the last run
gh watch notify --since 2d          # try it: notify about the last 2 days
gh watch notify --since 2d --dry-run   # or just print what it would send
```

`--since` and `--dry-run` leave the saved "last notified" time alone, so a
scheduled `notify` still picks up where it left off. That also means a plain
`--since` run's notifications can come again on the next scheduled run;
add `--dry-run` to only print them. To change what's watched for good, use
`gh watch config` (below): `notify` always uses the config.

If a repo or issue can't be read (no network, say), `notify` keeps the
clock where it was and looks at the same window again next time, without
repeating what it already sent, for up to a day.

`notify` sends one notification per changed issue. The headline says who
did what where ("sally commented on arize (#48765)", "arize (#48765)
closed by PR #94655"), and the body has the newest comment, or the issue's
title for a close. Clicking opens the comment or issue. Past five, the rest
go into one summary that opens a GitHub search. The first run only starts
the clock. On Linux it uses `notify-send`, which can't open anything.

Each step of `setup` also works on its own: `gh watch install-app`
rebuilds the app, `gh watch schedule` reinstalls the schedule, and `gh
watch schedule --remove` stops it. Running `setup` again does all of it
and is safe to repeat.

## Not working?

```sh
gh watch doctor
```

checks each thing notifications need, with the fix for anything that's
wrong: gh and its login, the config, the app and whether macOS lets it
notify, and the schedule (including a run that failed, or a schedule
pointing at an old gh-watch). Then it sends a test notification and says
how it went out.

A notification without the cat icon, which does nothing when clicked,
came without the app (plain macOS notifications): `doctor` says why.
Usually the app couldn't be built (`xcode-select --install`, then `gh watch
install-app`) or notifications for it are off (System Settings →
Notifications → gh-watch).

## Upgrading

When a new minor version is out (0.2.0, say, but not bug-fix releases like
0.1.6), `notify` tells you once, with the command; `gh watch doctor`
mentions any newer release. To upgrade:

```sh
gh extension upgrade gh-watch && gh watch setup
```

Running `setup` again keeps your config and where it left off, and
rebuilds the app and the schedule from the new version. With Cargo, it's
`cargo install --git https://github.com/j-dominguez9/gh-watch --tag
v<version> --force && gh-watch setup`.

## Uninstalling

```sh
gh watch schedule --remove
gh extension remove gh-watch
rm -rf ~/Applications/gh-watch.app ~/.config/gh-watch ~/.local/state/gh-watch
```

## Config

`setup` writes it: the repo, and issues you're involved in. To watch more,
or less:

```sh
gh watch config                               # what's watched, and where the file is
gh watch config add --label customer-acme     # also issues with this label
gh watch config add --repo owner/other        # another repo
gh watch config remove --label customer-acme
gh watch config remove --involved             # only labels, not issues you're in
```

It won't save a config that watches nothing (no repo, or no labels and not
issues you're involved in), since every scheduled run would fail on it.
Running `gh watch setup` in another repo adds that repo.

A label that doesn't exist in a watched repo gets a warning, since a typo
would otherwise match nothing. First time, `gh watch setup --label
customer-acme` does both at once.

It's `~/.config/gh-watch/config.toml`, which you can also edit by hand
(`config add` and `remove` keep your comments and layout):

```toml
# Repos to watch, as owner/name.
repos = ["owner/repo"]
# Also watch issues with these labels, in every repo above.
labels = ["customer-acme"]
# Issues you're involved in: opened, assigned, mentioned, commented on.
involved = true
```

## As a library

The `gh_watch` crate has what the command is built on, for tools that do
their own notifying:

- `gh_watch::gh`: issues by label, issues you're involved in, comments,
  and your login, through `gh`.
- `gh_watch::changes`: which of those issues really changed since a
  moment (`changes::changes`), and the issues a `Watch` covers
  (`changes::watched`).

## Releasing

Bump `version` in `Cargo.toml`, merge, then tag it (`git tag v0.1.0 &&
git push --tags`). The release workflow
builds `gh-watch-darwin-arm64`, `gh-watch-darwin-amd64` and
`gh-watch-linux-amd64` and attaches them to a GitHub release, which is
what `gh extension install` downloads.
