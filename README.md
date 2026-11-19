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

As a gh extension, from a release's prebuilt binaries (macOS and Linux):

```sh
gh extension install j-dominguez9/gh-watch
gh watch changes
```

Or with Cargo:

```sh
cargo install --git https://github.com/j-dominguez9/gh-watch
gh-watch changes
```

Either way, `gh auth login` first if you haven't.

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
scheduled `notify` still picks up where it left off.

`notify` sends one notification per changed issue: its title, and the
newest comment in the commenter's words, or which PR closed it. Clicking
opens the comment or issue. Past five, the rest go into one summary. The
first run only starts the clock. On macOS, install
[terminal-notifier](https://github.com/julienXX/terminal-notifier)
(`brew install terminal-notifier`) for notifications you can click;
without it they still show, but clicking does nothing. On Linux it uses
`notify-send`.

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

## Notifications on a schedule (macOS)

To run `notify` every 30 minutes, with `repos` set in the config:

```sh
sed -e "s|GH_WATCH_PATH|$(command -v gh-watch)|" -e "s|HOME_DIR|$HOME|g" \
  contrib/gh-watch-notify.plist > ~/Library/LaunchAgents/com.gh-watch.notify.plist
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.gh-watch.notify.plist
```

For the extension, `command -v gh-watch` finds nothing: use the binary gh
installed, `~/.local/share/gh/extensions/gh-watch/gh-watch`.

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
