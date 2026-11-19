#!/bin/sh
# Builds "gh-watch.app" (the app gh-watch's notifications come from) into
# macos/build. With --install, also copies it to ~/Applications, where gh-watch
# looks for it, and sends a test notification so macOS asks for permission.
set -eu

cd "$(dirname "$0")"
app="build/gh-watch.app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

# Built for this Mac only: you build it on the Mac that uses it.
swiftc -O -swift-version 5 -target "$(uname -m)-apple-macos11" \
    main.swift -o "$app/Contents/MacOS/gh-watch-notifier"

cp Info.plist "$app/Contents/Info.plist"
# The icon is drawn by make-icon.swift.
rm -rf build/AppIcon.iconset
swift make-icon.swift build/AppIcon.iconset
iconutil -c icns build/AppIcon.iconset -o "$app/Contents/Resources/AppIcon.icns"
rm -rf build/AppIcon.iconset
# Notification sounds have to come from the app itself.
cp /System/Library/Sounds/Glass.aiff "$app/Contents/Resources/Glass.aiff"

# macOS only lets signed apps send notifications. An ad-hoc signature is
# enough for your own Mac.
codesign --force --sign - "$app"
echo "Built $app"

if [ "${1:-}" = "--install" ]; then
    mkdir -p ~/Applications
    rm -rf ~/Applications/"gh-watch.app"
    cp -R "$app" ~/Applications/
    # Opened through Finder's machinery (not run directly) so macOS shows
    # the permission prompt.
    open -a ~/Applications/"gh-watch.app" --args \
        --title "gh-watch" --subtitle "Notifications are on" --message "You'll get gh-watch's notifications here."
    echo "Installed to ~/Applications. Allow notifications if macOS asks."
fi
