//! `gh-watch install-app`: builds and installs gh-watch's own notifier app
//! (macOS), so notifications show under its name and cat icon.
//!
//! The app's source travels inside this binary, so it works however
//! gh-watch was installed (gh extension, cargo, a clone). It's compiled
//! here with Apple's command line tools and signed ad hoc, the same as
//! `macos/build.sh`. Built on this Mac rather than downloaded, it isn't
//! quarantined, so Gatekeeper lets it run without a Developer ID.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const MAIN_SWIFT: &str = include_str!("../macos/main.swift");
const MAKE_ICON_SWIFT: &str = include_str!("../macos/make-icon.swift");
const INFO_PLIST: &str = include_str!("../macos/Info.plist");

/// Where the app goes, and where gh-watch looks for it.
pub fn path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not find your home folder")?;
    Ok(home.join("Applications/gh-watch.app"))
}

/// Builds the app, replaces any installed one with it, and opens it once
/// so macOS asks for permission to notify. Returns where it is.
pub fn install() -> Result<PathBuf> {
    if !cfg!(target_os = "macos") {
        bail!("the gh-watch app is for macOS; on Linux, notifications use notify-send");
    }
    for tool in TOOLS {
        if !runs(tool) {
            bail!(
                "{tool} not found: install Apple's command line tools with `xcode-select --install`, then try again"
            );
        }
    }

    let work = std::env::temp_dir().join(format!("gh-watch-app-{}", std::process::id()));
    let result = build(&work).and_then(|built| put_in_place(&built));
    // Only now: the built app lives in `work` until it's been copied out.
    let _ = std::fs::remove_dir_all(&work);
    let target = result?;

    // A first notification, through Launch Services, so macOS shows the
    // permission prompt.
    run(Command::new("open").arg("-a").arg(&target).args([
        "--args",
        "--title",
        "gh-watch",
        "--message",
        "Notifications are on. Allow them if macOS asks.",
    ]))?;
    Ok(target)
}

/// Copies the built app to its place and signs it there, replacing any
/// installed one only once the new one is ready, so a failure part way
/// leaves the old app as it was. Returns where it is.
fn put_in_place(built: &Path) -> Result<PathBuf> {
    let target = path()?;
    let parent = target.parent().context("bad app path")?;
    std::fs::create_dir_all(parent)?;
    let staged = parent.join(".gh-watch.app.new");
    let _ = std::fs::remove_dir_all(&staged);
    copy_dir(built, &staged).context("could not copy the app into place")?;
    // macOS only lets signed apps send notifications; ad hoc is enough.
    // Signed where it'll live, so nothing changes the bundle afterwards
    // (moving it within the folder doesn't).
    if let Err(err) = run(Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(&staged))
    {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(err);
    }
    if target.exists() {
        std::fs::remove_dir_all(&target)
            .with_context(|| format!("could not replace {}", target.display()))?;
    }
    std::fs::rename(&staged, &target)
        .with_context(|| format!("could not move the app to {}", target.display()))?;
    Ok(target)
}

/// Builds gh-watch.app in `work` and returns its path there.
fn build(work: &Path) -> Result<PathBuf> {
    let app = work.join("gh-watch.app");
    let contents = app.join("Contents");
    std::fs::create_dir_all(contents.join("MacOS"))?;
    std::fs::create_dir_all(contents.join("Resources"))?;
    std::fs::write(work.join("main.swift"), MAIN_SWIFT)?;
    std::fs::write(work.join("make-icon.swift"), MAKE_ICON_SWIFT)?;
    std::fs::write(contents.join("Info.plist"), INFO_PLIST)?;

    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    };
    eprintln!("Building the gh-watch app...");
    run(Command::new("swiftc")
        .current_dir(work)
        .args(["-O", "-swift-version", "5", "-target"])
        .arg(format!("{arch}-apple-macos11"))
        .arg("main.swift")
        .arg("-o")
        .arg(contents.join("MacOS/gh-watch-notifier")))?;

    run(Command::new("swift")
        .current_dir(work)
        .args(["make-icon.swift", "AppIcon.iconset"]))?;
    run(Command::new("iconutil")
        .current_dir(work)
        .args(["-c", "icns", "AppIcon.iconset", "-o"])
        .arg(contents.join("Resources/AppIcon.icns")))?;

    // Notification sounds have to come from the app itself.
    std::fs::copy(
        "/System/Library/Sounds/Glass.aiff",
        contents.join("Resources/Glass.aiff"),
    )
    .context("could not copy the notification sound")?;

    Ok(app)
}

/// The command line tools building the app needs that aren't installed.
pub fn missing_tools() -> Vec<&'static str> {
    TOOLS.into_iter().filter(|tool| !runs(tool)).collect()
}

const TOOLS: [&str; 4] = ["swiftc", "swift", "iconutil", "codesign"];

/// Whether `tool` can be run at all.
fn runs(tool: &str) -> bool {
    Command::new("xcrun")
        .args(["--find", tool])
        .output()
        .is_ok_and(|o| o.status.success())
        || Command::new("which")
            .arg(tool)
            .output()
            .is_ok_and(|o| o.status.success())
}

fn run(command: &mut Command) -> Result<()> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command
        .output()
        .with_context(|| format!("could not run {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}
