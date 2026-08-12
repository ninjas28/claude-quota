import Foundation

/// The rolling windows Anthropic reports for a Claude.ai subscription.
///
/// `fiveHour` is the "session" window most people care about minute to minute;
/// the weekly ones are the ceilings. Model-scoped weekly windows only exist on
/// some plans, so every window is optional at runtime.
enum UsageWindowKind: String, CaseIterable, Identifiable {
    case fiveHour
    case sevenDay
    /// A weekly ceiling that applies to one model rather than the whole plan.
    /// The model it covers arrives at runtime, in `UsageWindow.scopeLabel`.
    case weeklyScoped

    var id: String { rawValue }

    /// Row labels, phrased the way Claude's usage screen phrases them. The
    /// weekly rows don't repeat "Weekly", the section header carries that.
    var label: String {
        switch self {
        case .fiveHour: return "Current session"
        case .sevenDay: return "All models"
        case .weeklyScoped: return "Scoped"
        }
    }

    /// Whether this window belongs under the "Weekly limits" heading.
    var isWeekly: Bool { self != .fiveHour }

    /// Order used when rendering the popover.
    static let displayOrder: [UsageWindowKind] = [.fiveHour, .sevenDay, .weeklyScoped]
}

struct UsageWindow: Equatable {
    /// 0...100. Both sources report this as a percentage, not a 0-1 fraction.
    /// (The legacy CLI's response headers use 0-1 — different source, different
    /// scale. `OAuthUsageClientTests` pins this down.)
    var usedPercentage: Double
    var resetsAt: Date?
    /// What this window is scoped to, when it isn't the whole plan — e.g. the
    /// display name of a single model. Nil for plan-wide windows.
    var scopeLabel: String?

    init(usedPercentage: Double, resetsAt: Date?, scopeLabel: String? = nil) {
        self.usedPercentage = usedPercentage
        self.resetsAt = resetsAt
        self.scopeLabel = scopeLabel
    }

    var clampedPercentage: Double { min(max(usedPercentage, 0), 100) }
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

    /// Scope is part of the identity: an account can report several scoped
    /// weekly windows, which share a `kind` but must not share a `ForEach` id.
    var id: String {
        guard let scope = window.scopeLabel else { return kind.rawValue }
        return "\(kind.rawValue)·\(scope)"
    }

    /// A scoped window is named after the model it covers ("Fable"); everything
    /// else uses its kind's label.
    var label: String { window.scopeLabel ?? kind.label }

    /// The line under the label. Claude's screen shows the countdown, or a
    /// "not used yet" note for a window still sitting at zero.
    func subtitle(now: Date) -> String? {
        if window.clampedPercentage == 0 {
            return "You haven't used \(window.scopeLabel ?? "this") yet"
        }
        guard let resetsAt = window.resetsAt else { return nil }
        guard resetsAt > now else { return "Reset" }
        return "Resets in \(RelativeTime.longCountdown(to: resetsAt, from: now))"
    }
}

struct UsageSnapshot: Equatable {
    /// An ordered list rather than a dictionary keyed by kind: the usage
    /// endpoint can report more than one `weekly_scoped` window (one per
    /// model), and those collide on `kind`.
    var windows: [UsageEntry]
    /// Whether the account has extra/overage usage enabled. Absent when unknown.
    var extraUsageEnabled: Bool?
    var capturedAt: Date
    var source: UsageSource
    /// Plan badge for the header ("Max (5x)"). Only the OAuth path knows it —
    /// it comes off the stored credential, not the usage payload.
    var plan: String?

    subscript(kind: UsageWindowKind) -> UsageWindow? {
        windows.first { $0.kind == kind }?.window
    }

    var age: TimeInterval { max(0, Date().timeIntervalSince(capturedAt)) }

    /// Windows that actually have data, in display order. Sorted on the index
    /// into `displayOrder` and the original position, so several windows of the
    /// same kind keep the order the server sent them in.
    var presentWindows: [UsageEntry] {
        windows.enumerated().sorted { left, right in
            let leftRank = UsageWindowKind.displayOrder.firstIndex(of: left.element.kind) ?? .max
            let rightRank = UsageWindowKind.displayOrder.firstIndex(of: right.element.kind) ?? .max
            if leftRank != rightRank { return leftRank < rightRank }
            return left.offset < right.offset
        }.map(\.element)
    }

    /// The two groups Claude's usage screen splits windows into.
    var sessionWindows: [UsageEntry] { presentWindows.filter { !$0.kind.isWeekly } }
    var weeklyWindows: [UsageEntry] { presentWindows.filter(\.kind.isWeekly) }

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
