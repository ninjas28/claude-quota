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

    /// A compact countdown: "4h 12m", "38m", "12s", or "now" once elapsed.
    static func countdown(to date: Date) -> String {
        let seconds = Int(date.timeIntervalSinceNow.rounded())
        guard seconds > 0 else { return "now" }
        if seconds < 60 { return "\(seconds)s" }
        let minutes = seconds / 60
        if minutes < 60 { return "\(minutes)m" }
        let hours = minutes / 60
        if hours < 24 {
            let remainder = minutes % 60
            return remainder == 0 ? "\(hours)h" : "\(hours)h \(remainder)m"
        }
        let days = hours / 24
        let remainder = hours % 24
        return remainder == 0 ? "\(days)d" : "\(days)d \(remainder)h"
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
