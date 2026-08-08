import AppKit
import SwiftUI

/// Hosts Settings in a real `NSWindow`.
///
/// A SwiftUI `.sheet` presented from a `MenuBarExtra` has no reliable presenting
/// window — the extra lives in a transient panel that dismisses as soon as focus
/// leaves it, which takes the sheet with it. Owning the window directly avoids
/// the whole problem, and lets an `LSUIElement` app raise it properly.
@MainActor
final class SettingsWindowController: NSObject, NSWindowDelegate {
    static let shared = SettingsWindowController()

    private var window: NSWindow?

    func show(model: UsageModel) {
        if let window {
            window.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return
        }

        let hosting = NSHostingController(
            rootView: SettingsView(model: model) { [weak self] in self?.close() }
        )
        let window = NSWindow(contentViewController: hosting)
        window.title = "Claude Quota Bar Settings"
        window.styleMask = [.titled, .closable]
        // The controller owns the lifetime; without this, closing deallocates
        // the window and reopening crashes.
        window.isReleasedWhenClosed = false
        window.delegate = self
        window.center()

        self.window = window
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    func close() {
        window?.close()
    }

    nonisolated func windowWillClose(_ notification: Notification) {
        // Release the reference so the next Settings… builds a fresh window.
        Task { @MainActor in self.window = nil }
    }
}
