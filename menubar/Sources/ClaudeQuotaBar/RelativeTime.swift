import Foundation

/// Small date formatting helpers shared by the popover and error strings.
enum RelativeTime {
    private static let clockFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "h:mm a"
        return formatter
    }()

    private static let dayClockFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "EEE h:mm a"
        return formatter
    }()

    /// "3:45 PM", or "Sat 3:45 PM" once the date is more than a day out.
    static func clock(_ date: Date) -> String {
        let interval = date.timeIntervalSinceNow
        return interval < 86_400 ? clockFormatter.string(from: date) : dayClockFormatter.string(from: date)
    }

    /// A spelled-out countdown: "3 hr 19 min", "45 min", "2 days 4 hr".
    /// Phrased the way Claude's usage screen phrases the same thing.
    static func longCountdown(to date: Date, from now: Date = Date()) -> String {
        let seconds = Int(date.timeIntervalSince(now).rounded())
        guard seconds > 0 else { return "now" }
        if seconds < 60 { return "\(seconds) sec" }

        let minutes = seconds / 60
        if minutes < 60 { return "\(minutes) min" }

        let hours = minutes / 60
        if hours < 24 {
            let remainder = minutes % 60
            return remainder == 0 ? "\(hours) hr" : "\(hours) hr \(remainder) min"
        }

        let days = hours / 24
        let remainder = hours % 24
        let dayLabel = days == 1 ? "1 day" : "\(days) days"
        return remainder == 0 ? dayLabel : "\(dayLabel) \(remainder) hr"
    }

    /// How long ago a snapshot was captured: "just now", "4m ago", "2h ago".
    static func age(_ interval: TimeInterval) -> String {
        let seconds = Int(interval.rounded())
        if seconds < 45 { return "just now" }
        if seconds < 3_600 { return "\(max(1, seconds / 60))m ago" }
        if seconds < 86_400 { return "\(seconds / 3_600)h ago" }
        return "\(seconds / 86_400)d ago"
    }
}
