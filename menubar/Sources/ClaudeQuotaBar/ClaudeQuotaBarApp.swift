import AppKit
import SwiftUI

@main
struct ClaudeQuotaBarApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    /// Shared so the delegate can start polling at launch, before the popover
    /// has ever been opened.
    @StateObject private var model = UsageModel.shared

    var body: some Scene {
        MenuBarExtra {
            MenuContentView(model: model)
        } label: {
            // .original keeps the colour ramp; the default would tint the
            // whole ring with the menu bar's label colour.
            Image(nsImage: labelImage)
                .renderingMode(.original)
                .accessibilityLabel(accessibilityLabel)
        }
        .menuBarExtraStyle(.window)
    }

    @MainActor
    private var displayed: UsageEntry? {
        model.snapshot.flatMap { model.barDisplayMode.entry(from: $0) }
    }

    /// Dim the ring once data is old enough to mislead — roughly two missed
    /// refreshes.
    @MainActor
    private var isStale: Bool {
        guard let snapshot = model.snapshot else { return true }
        return snapshot.age > TimeInterval(model.pollInterval) * 2
    }

    @MainActor
    private var labelImage: NSImage {
        UsageGaugeIcon.makeStatusImage(
            percentage: displayed?.window.clampedPercentage,
            stale: isStale,
            showText: model.showPercentageText
        )
    }

    @MainActor
    private var accessibilityLabel: String {
        guard let displayed else { return "Claude usage unavailable" }
        return "Claude \(displayed.kind.label) usage \(Int(displayed.window.clampedPercentage.rounded())) percent"
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Menu bar only — no Dock tile, no main window. Also set via
        // LSUIElement in Info.plist so the Dock never flashes at launch.
        NSApp.setActivationPolicy(.accessory)
        Task { @MainActor in UsageModel.shared.start() }
    }

    func applicationWillTerminate(_ notification: Notification) {
        Task { @MainActor in UsageModel.shared.stop() }
    }
}
