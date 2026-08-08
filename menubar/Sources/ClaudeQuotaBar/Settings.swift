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

enum SettingsKey {
    static let pollInterval = "pollIntervalSeconds"
    static let barDisplayMode = "barDisplayMode"
    static let oauthFallbackEnabled = "oauthFallbackEnabled"
    static let showPercentageText = "showPercentageText"
}

enum PollInterval {
    /// Options offered in Settings, in seconds.
    static let choices: [Int] = [60, 300, 600, 1_800]

    static func label(_ seconds: Int) -> String {
        seconds < 3_600 ? "\(seconds / 60) min" : "\(seconds / 3_600) hr"
    }

    static let `default` = 300
}

enum Defaults {
    /// A status line snapshot older than this is treated as stale, which is what
    /// triggers an OAuth poll. Sized so an active session (which re-renders on
    /// every turn) essentially never triggers a network call.
    static let cacheStaleAfter: TimeInterval = 120

    /// Backoff ceiling after repeated 429s from the usage endpoint.
    static let maxBackoff: TimeInterval = 1_800
}
