import AppKit
import SwiftUI

/// Laid out like Claude's own usage screen: a label column with a countdown
/// beneath it, a capsule bar, and "N% used" on the right, grouped into the
/// session window and the weekly ones.
struct MenuContentView: View {
    @ObservedObject var model: UsageModel
    /// Local to the popover so countdowns only tick while it's actually open.
    @State private var now = Date()
    private let clock = Timer.publish(every: 1, on: .main, in: .common).autoconnect()

    private var isStale: Bool {
        guard let snapshot = model.snapshot else { return false }
        return snapshot.age > TimeInterval(model.pollInterval) * 2
    }

    /// The empty state already says everything `.noData` would, so showing both
    /// prints the same sentence twice.
    private var visibleError: UsageError? {
        guard let error = model.lastError else { return nil }
        if model.snapshot == nil && error == .noData { return nil }
        return error
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider()

            if let snapshot = model.snapshot {
                VStack(alignment: .leading, spacing: 18) {
                    ForEach(snapshot.sessionWindows) { entry in
                        WindowRow(entry: entry, stale: isStale, theme: model.colorTheme, now: now)
                    }

                    if !snapshot.weeklyWindows.isEmpty {
                        VStack(alignment: .leading, spacing: 14) {
                            Text("Weekly limits")
                                .font(.system(size: 12, weight: .semibold))
                            ForEach(snapshot.weeklyWindows) { entry in
                                WindowRow(
                                    entry: entry,
                                    stale: isStale,
                                    theme: model.colorTheme,
                                    now: now
                                )
                            }
                        }
                    }

                    // Only shown when we actually know: the status line payload
                    // doesn't report it, so this stays hidden on cached data.
                    if let extraUsage = snapshot.extraUsageEnabled {
                        HStack(spacing: 5) {
                            Image(systemName: extraUsage ? "plus.circle.fill" : "minus.circle")
                                .font(.system(size: 9))
                            Text(extraUsage
                                 ? "Extra usage on — work continues past 100%"
                                 : "Extra usage off — requests stop at 100%")
                        }
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 16)
            } else {
                emptyState
            }

            if let error = visibleError {
                Divider()
                errorBanner(error)
            }

            Divider()
            footer
        }
        .frame(width: 420)
        .onReceive(clock) { now = $0 }
        .onAppear { model.refresh() }
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text("Plan usage limits")
                .font(.system(size: 13, weight: .semibold))
            if let plan = model.snapshot?.plan {
                Text(plan)
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button { model.refreshNow() } label: {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 11, weight: .semibold))
            }
            .buttonStyle(.plain)
            .disabled(model.isRefreshing)
            .opacity(model.isRefreshing ? 0.4 : 1)
            .help("Refresh now")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 11)
    }

    private var emptyState: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("No usage data yet")
                .font(.system(size: 12, weight: .medium))
            Text("Start a Claude Code session, or enable the usage API fallback in Settings.")
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    private func errorBanner(_ error: UsageError) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 11))
                .foregroundStyle(.orange)
            VStack(alignment: .leading, spacing: 2) {
                Text(error.title)
                    .font(.system(size: 11, weight: .medium))
                Text(error.detail)
                    .font(.system(size: 10))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    private var footer: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let snapshot = model.snapshot {
                HStack(spacing: 4) {
                    Circle()
                        .fill(isStale ? Color.secondary : Color.green)
                        .frame(width: 5, height: 5)
                    Text("\(RelativeTime.age(snapshot.age)) · \(snapshot.source.label)")
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                    if let modelName = model.context?.model, snapshot.source == .statusLine {
                        Text("· \(modelName)")
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                    }
                }
            }

            HStack {
                Button("Settings…") { SettingsWindowController.shared.show(model: model) }
                    .buttonStyle(.plain)
                    .font(.system(size: 11))
                Spacer()
                Button("Quit") { NSApplication.shared.terminate(nil) }
                    .buttonStyle(.plain)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }
}

private struct WindowRow: View {
    var entry: UsageEntry
    var stale: Bool
    var theme: ColorTheme
    /// Passed in so the countdown re-renders on the popover's clock tick.
    var now: Date

    /// Fixed columns so every bar starts and ends on the same x, however long
    /// the labels are — the alignment is most of what makes Claude's version
    /// read as a table rather than a list.
    private enum Column {
        /// Wide enough for "You haven't used Fable yet" on one line. Letting
        /// that wrap makes one row taller than the others and the table stops
        /// reading as a table.
        static let label: CGFloat = 152
        static let percentage: CGFloat = 64
    }

    var body: some View {
        HStack(alignment: .center, spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text(entry.label)
                    .font(.system(size: 12))
                if let subtitle = entry.subtitle(now: now) {
                    Text(subtitle)
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            .frame(width: Column.label, alignment: .leading)

            UsageBar(
                percentage: entry.window.clampedPercentage,
                stale: stale,
                theme: theme
            )

            Text("\(Int(entry.window.clampedPercentage.rounded()))% used")
                .font(.system(size: 11).monospacedDigit())
                .foregroundStyle(.secondary)
                .frame(width: Column.percentage, alignment: .trailing)
        }
    }
}
