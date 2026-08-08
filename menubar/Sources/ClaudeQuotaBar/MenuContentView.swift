import AppKit
import SwiftUI

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
                VStack(alignment: .leading, spacing: 14) {
                    ForEach(snapshot.presentWindows) { entry in
                        WindowRow(kind: entry.kind, window: entry.window, stale: isStale, now: now)
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 14)
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
        .frame(width: 288)
        .onReceive(clock) { now = $0 }
        .onAppear { model.refresh() }
    }

    private var header: some View {
        HStack {
            Text("Claude Usage")
                .font(.system(size: 13, weight: .semibold))
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
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
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
        .padding(.horizontal, 14)
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
        .padding(.horizontal, 14)
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
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }
}

private struct WindowRow: View {
    var kind: UsageWindowKind
    var window: UsageWindow
    var stale: Bool
    /// Passed in so the countdown re-renders on the model's clock tick.
    var now: Date

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(alignment: .firstTextBaseline) {
                Text(kind.label)
                    .font(.system(size: 11, weight: .medium))
                Spacer()
                Text("\(Int(window.clampedPercentage.rounded()))%")
                    .font(.system(size: 11, weight: .semibold).monospacedDigit())
                    .foregroundStyle(UsageColor.color(window.clampedPercentage))
            }

            UsageBar(percentage: window.clampedPercentage, stale: stale)

            if let resetsAt = window.resetsAt {
                Text(resetsAt > now
                     ? "Resets in \(RelativeTime.countdown(to: resetsAt)) · \(RelativeTime.clock(resetsAt))"
                     : "Reset")
                    .font(.system(size: 10))
                    .foregroundStyle(.secondary)
            }
        }
    }
}
