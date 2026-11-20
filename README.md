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

As a gh extension (prebuilt binaries for macOS and Linux):

```sh
gh extension install j-dominguez9/gh-watch
```

Or with Cargo: `cargo install --git https://github.com/j-dominguez9/gh-watch`
(then it's `gh-watch` rather than `gh watch`).

Then, from inside the repo you want to watch (or pass `--repo owner/name`):

```sh
gh watch setup
```

That does everything in one go:

1. writes a config with that repo, so scheduled runs know what to watch;
2. on macOS, builds and installs gh-watch's notifier app, so notifications
   show under its own name and cat icon (macOS asks once for permission);
3. on macOS, runs `notify` every 30 minutes;
4. starts the clock, so you only hear about changes from now on.

To see what a notification looks like right away: `gh watch notify --since 2d`.

You need `gh auth login` done. The app needs Apple's command line tools
(`xcode-select --install`), which you likely have already since they come
with git. Without them, setup carries on and notifications use
[terminal-notifier](https://github.com/julienXX/terminal-notifier) if it's
installed, or plain macOS notifications.

## Use

```sh
gh watch changes                    # what changed in the last day, in the repo you're in
gh watch changes --since 3h         # also 30m, 2d, 1w
gh watch -R owner/repo changes      # another repo (repeat -R for several)
gh watch -l needs-review changes    # also issues with this label
gh watch notify                     # notify about changes since the last run
gh watch notify --since 2d          # try it: notify about the last 2 days
gh watch notify --since 2d --dry-run   # or just print what it would send
```

`--since` and `--dry-run` leave the saved "last notified" time alone, so a
scheduled `notify` still picks up where it left off. That also means a plain
`--since` run's notifications can come again on the next scheduled run;
add `--dry-run` to only print them.

`notify` sends one notification per changed issue: its title, and the
newest comment in the commenter's words, or which PR closed it. Clicking
opens the comment or issue. Past five, the rest go into one summary. The
first run only starts the clock. On Linux it uses `notify-send`.

Clicking a notification opens the comment or issue. On Linux they come
from `notify-send`.

Each step of `setup` also works on its own: `gh watch install-app`
rebuilds the app (say, after updating gh-watch), `gh watch schedule`
reinstalls the schedule, and `gh watch schedule --remove` stops it.

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
Notifications → gh-watch). After updating gh-watch, run `gh watch
install-app` and `gh watch schedule` again.

## Config

Optional, in `~/.config/gh-watch/config.toml`, so you don't need flags:

```toml
# Repos to watch, as owner/name. Default: the repo you run it in.
repos = ["owner/repo"]
# Also watch issues with these labels, in every repo above.
labels = ["needs-review"]
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

Tag a version (`git tag v0.1.0 && git push --tags`). The release workflow
builds `gh-watch-darwin-arm64`, `gh-watch-darwin-amd64` and
`gh-watch-linux-amd64` and attaches them to a GitHub release, which is
what `gh extension install` downloads.
