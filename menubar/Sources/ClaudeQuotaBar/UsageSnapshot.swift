import Foundation

/// The rolling windows Anthropic reports for a Claude.ai subscription.
///
/// `fiveHour` is the "session" window most people care about minute to minute;
/// the `sevenDay*` windows are the weekly ceilings. The model-scoped weekly
/// windows only exist on some plans, so every window is optional at runtime.
enum UsageWindowKind: String, CaseIterable, Identifiable {
    case fiveHour
    case sevenDay
    case sevenDayOpus
    case sevenDaySonnet

    var id: String { rawValue }

    var label: String {
        switch self {
        case .fiveHour: return "Session"
        case .sevenDay: return "Weekly"
        case .sevenDayOpus: return "Weekly · Opus"
        case .sevenDaySonnet: return "Weekly · Sonnet"
        }
    }

    var shortLabel: String {
        switch self {
        case .fiveHour: return "5h"
        case .sevenDay: return "7d"
        case .sevenDayOpus: return "Opus"
        case .sevenDaySonnet: return "Sonnet"
        }
    }

    /// Order used when rendering the popover.
    static let displayOrder: [UsageWindowKind] = [.fiveHour, .sevenDay, .sevenDayOpus, .sevenDaySonnet]
}

struct UsageWindow: Equatable {
    /// 0...100. Anthropic reports this as a percentage, not a 0-1 fraction.
    var usedPercentage: Double
    var resetsAt: Date?

    var clampedPercentage: Double { min(max(usedPercentage, 0), 100) }
    var remainingPercentage: Double { max(0, 100 - clampedPercentage) }
}

/// Where a snapshot came from. Both sources report the same numbers; the
/// distinction matters only for explaining staleness to the user.
enum UsageSource: String, Equatable {
    /// Handed to us locally by Claude Code's status line. Free, no network call.
    case statusLine
    /// Polled from the OAuth usage endpoint. Free of quota, but a network call.
    case oauth

    var label: String {
        switch self {
        case .statusLine: return "Claude Code session"
        case .oauth: return "usage API"
        }
    }
}

/// One window paired with its kind.
///
/// A named struct rather than a tuple because Swift has no key paths into tuple
/// members, and `ForEach(_:id:)` needs one.
struct UsageEntry: Identifiable, Equatable {
    var kind: UsageWindowKind
    var window: UsageWindow

    var id: String { kind.rawValue }
}

struct UsageSnapshot: Equatable {
    var windows: [UsageWindowKind: UsageWindow]
    /// Whether the account has extra/overage usage enabled. Absent when unknown.
    var extraUsageEnabled: Bool?
    var capturedAt: Date
    var source: UsageSource

    subscript(kind: UsageWindowKind) -> UsageWindow? { windows[kind] }

    var age: TimeInterval { max(0, Date().timeIntervalSince(capturedAt)) }

    /// Windows that actually have data, in display order.
    var presentWindows: [UsageEntry] {
        UsageWindowKind.displayOrder.compactMap { kind in
            windows[kind].map { UsageEntry(kind: kind, window: $0) }
        }
    }

    /// The highest utilization across every reported window — what the menu bar
    /// shows in "worst window" mode, since that is the one that will bite first.
    var peak: UsageEntry? {
        presentWindows.max { $0.window.clampedPercentage < $1.window.clampedPercentage }
    }
}

/// A user-facing problem that stopped us from refreshing.
enum UsageError: Equatable {
    case noCredentials
    case credentialsExpired
    case rateLimited(retryAt: Date?)
    case notSubscribed
    case network(String)
    case noData

    var title: String {
        switch self {
        case .noCredentials: return "Claude Code not logged in"
        case .credentialsExpired: return "Login expired"
        case .rateLimited: return "Usage API rate limited"
        case .notSubscribed: return "No subscription usage"
        case .network: return "Couldn't reach the usage API"
        case .noData: return "No usage data yet"
        }
    }

    var detail: String {
        switch self {
        case .noCredentials:
            return "Run `claude` once and sign in, or turn off the usage API fallback in Settings."
        case .credentialsExpired:
            return "Open Claude Code to refresh the login. Claude Quota Bar never refreshes tokens itself."
        case .rateLimited(let retryAt):
            guard let retryAt else { return "Backing off before trying again." }
            return "Backing off until \(RelativeTime.clock(retryAt))."
        case .notSubscribed:
            return "Rate limit windows are only reported for Claude Pro and Max accounts."
        case .network(let message):
            return message
        case .noData:
            return "Start a Claude Code session, or enable the usage API fallback in Settings."
        }
    }
}
