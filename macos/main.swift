// gh-watch's notifier: a tiny app that shows gh-watch's notifications under
// its own name and icon, and opens the issue or comment when clicked.
//
//   gh-watch-notifier --title T [--subtitle S] --message M
//                    [--open URL-or-PATH] [--group NAME]
//
// --open takes a web link (opened in the browser) or a file path.
// --group keeps related notifications in one stack in Notification Center
// (gh-watch uses one per repo).
//
// Exit codes, so gh-watch knows when to fall back to something else:
//   0 shown, 1 bad arguments, 2 notifications not allowed, 3 other failure.
//
// Started without arguments (which is how macOS starts it when a
// notification is clicked), it waits briefly for the click, handles it and
// quits.

import AppKit
import UserNotifications

struct Options {
    var title = ""
    var subtitle = ""
    var message = ""
    var open: String?
    var group: String?
}

/// nil when there's nothing to show (no options of ours: macOS starting
/// the app for a click). Flags macOS itself may add at launch have a single
/// dash (-psn_0_12345, -NSDocumentRevisionsDebugMode YES) and are skipped,
/// so they can't make a click-launch fail.
func parseOptions(_ args: [String]) -> Options? {
    var ours = false
    var options = Options()
    var rest = args[...]
    while let flag = rest.popFirst() {
        if !flag.hasPrefix("--") {
            // A system flag, and its value if it has one.
            if flag.hasPrefix("-NS") || flag.hasPrefix("-Apple"),
                let next = rest.first, !next.hasPrefix("-")
            {
                rest.removeFirst()
            }
            continue
        }
        guard let value = rest.popFirst() else { fail(1, "\(flag) needs a value") }
        ours = true
        switch flag {
        case "--title": options.title = value
        case "--subtitle": options.subtitle = value
        case "--message": options.message = value
        case "--open": options.open = value
        case "--group": options.group = value
        default: fail(1, "unknown option \(flag)")
        }
    }
    if !ours { return nil }
    if options.title.isEmpty || options.message.isEmpty {
        fail(1, "--title and --message are required")
    }
    return options
}

func fail(_ code: Int32, _ message: String) -> Never {
    FileHandle.standardError.write(Data("gh-watch-notifier: \(message)\n".utf8))
    exit(code)
}

/// A web link as is; anything else as a file path.
func target(_ text: String) -> URL? {
    if text.hasPrefix("https://") || text.hasPrefix("http://") {
        return URL(string: text)
    }
    return URL(fileURLWithPath: text)
}

final class AppDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {
    let center = UNUserNotificationCenter.current()

    func applicationWillFinishLaunching(_ notification: Notification) {
        // Must be set before launching finishes, or a click that started
        // the app gets lost.
        center.delegate = self
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        // `--check` (for `gh watch doctor`): say whether notifications are
        // allowed, without posting one. Prints the answer; exits 0 if
        // banners can show, 2 if not (provisional permission only puts
        // them quietly in Notification Center).
        if CommandLine.arguments.dropFirst().first == "--check" {
            DispatchQueue.main.asyncAfter(deadline: .now() + 10) { fail(3, "timed out") }
            center.getNotificationSettings { settings in
                switch settings.authorizationStatus {
                case .authorized:
                    print("allowed")
                    exit(0)
                case .provisional:
                    print("delivered quietly")
                case .denied:
                    print("turned off")
                case .notDetermined:
                    print("not asked yet")
                default:
                    print("unknown")
                }
                exit(2)
            }
            return
        }
        if let options = parseOptions(Array(CommandLine.arguments.dropFirst())) {
            // The first time, this waits for you to answer the permission
            // prompt. Don't hang forever if nobody's there to answer.
            DispatchQueue.main.asyncAfter(deadline: .now() + 60) {
                fail(3, "timed out")
            }
            show(options)
        } else {
            // Started for a click: it arrives right after launch.
            DispatchQueue.main.asyncAfter(deadline: .now() + 10) { exit(0) }
        }
    }

    func show(_ options: Options) {
        center.requestAuthorization(options: [.alert, .sound]) { granted, error in
            if let error { fail(2, "not allowed: \(error.localizedDescription)") }
            if !granted { fail(2, "notifications are turned off in System Settings") }

            let content = UNMutableNotificationContent()
            content.title = options.title
            content.subtitle = options.subtitle
            content.body = options.message
            content.sound = UNNotificationSound(named: UNNotificationSoundName("Glass.aiff"))
            if let group = options.group {
                content.threadIdentifier = group
            }
            if let open = options.open {
                content.userInfo = ["open": open]
            }
            let request = UNNotificationRequest(
                identifier: UUID().uuidString, content: content, trigger: nil)
            self.center.add(request) { error in
                if let error { fail(3, error.localizedDescription) }
                exit(0)
            }
        }
    }

    // Shown even if this app happens to be the one in front (an install
    // opens it), which macOS would otherwise keep to Notification Center.
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler:
            @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .list, .sound])
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        if response.actionIdentifier == UNNotificationDefaultActionIdentifier,
            let open = response.notification.request.content.userInfo["open"] as? String,
            let url = target(open)
        {
            NSWorkspace.shared.open(url)
        }
        completionHandler()
        exit(0)
    }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
