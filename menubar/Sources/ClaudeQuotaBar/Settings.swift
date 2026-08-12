import Foundation

/// What the menu bar itself shows. The popover always lists every window.
enum BarDisplayMode: String, CaseIterable, Identifiable {
    case session
    case weekly
    case worst

    var id: String { rawValue }

    var label: String {
        switch self {
        case .session: return "Session (5h)"
        case .weekly: return "Weekly (7d)"
        case .worst: return "Whichever is highest"
        }
    }

    /// Falls back to the worst window when the preferred one isn't reported —
    /// better to show something true than an empty bar.
    func entry(from snapshot: UsageSnapshot) -> UsageEntry? {
        switch self {
        case .session:
            return snapshot[.fiveHour].map { UsageEntry(kind: .fiveHour, window: $0) } ?? snapshot.peak
        case .weekly:
            return snapshot[.sevenDay].map { UsageEntry(kind: .sevenDay, window: $0) } ?? snapshot.peak
        case .worst:
            return snapshot.peak
        }
    }
}

/// What shape the menu bar indicator takes.
enum MenuBarStyle: String, CaseIterable, Identifiable {
    case ring
    case bar

    var id: String { rawValue }

    var label: String {
        switch self {
        case .ring: return "Ring"
        case .bar: return "Bar"
        }
    }
}

/// How a percentage picks its colour.
///
/// Two honest positions, not a spectrum: match Claude's own usage panel, or
/// let the fill carry severity. The ring and the popover bars always agree —
/// a menu bar that says orange while the popover says blue is worse than
/// either on its own.
enum ColorTheme: String, CaseIterable, Identifiable {
    /// One flat blue at every level, like Claude's usage screen.
    case claude
    /// Green below 50%, yellow to 80%, orange to 95%, red above.
    case usage

    var id: String { rawValue }

    var label: String {
        switch self {
        case .claude: return "Claude blue"
        case .usage: return "Usage ramp"
        }
    }

    var detail: String {
        switch self {
        case .claude: return "Matches Claude's usage screen. Colour never signals how full you are."
        case .usage: return "Green under 50%, yellow to 80%, orange to 95%, red above."
        }
    }
}

enum SettingsKey {
    static let pollInterval = "pollIntervalSeconds"
    static let barDisplayMode = "barDisplayMode"
    static let oauthFallbackEnabled = "oauthFallbackEnabled"
    static let showPercentageText = "showPercentageText"
    static let menuBarStyle = "menuBarStyle"
    static let colorTheme = "colorTheme"
}

enum PollInterval {
    /// Options offered in Settings, in seconds.
    static let choices: [Int] = [60, 300, 600, 1_800]

    static func label(_ seconds: Int) -> String {
        seconds < 3_600 ? "\(seconds / 60) min" : "\(seconds / 3_600) hr"
    }

    static let `default` = 60
}

enum Defaults {
    /// A status line snapshot older than this is treated as stale, which is what
    /// triggers an OAuth poll. Sized so an active session (which re-renders on
    /// every turn) essentially never triggers a network call.
    static let cacheStaleAfter: TimeInterval = 120

    /// First backoff after a 429, doubling per consecutive rate limit. Fixed
    /// rather than derived from `pollInterval`, which at the 30-minute setting
    /// would land on the ceiling from the very first 429 and make the
    /// exponential meaningless.
    static let backoffBase: TimeInterval = 60

    /// Backoff ceiling after repeated 429s from the usage endpoint.
    static let maxBackoff: TimeInterval = 1_800
}
