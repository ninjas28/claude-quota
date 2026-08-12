import XCTest
import SwiftUI
@testable import ClaudeQuotaBar

/// Renders the README images straight from the real views.
///
/// `ImageRenderer` rasterises offscreen, so this needs no Screen Recording
/// permission, no window, and no click — and because the data is fixed, the
/// images never leak whoever ran it their own usage numbers. Regenerate with:
///
///     SNAPSHOT_DIR=../docs swift test --filter SnapshotTests
///
/// Skipped by default so a normal `swift test` doesn't write files.
@MainActor
final class SnapshotTests: XCTestCase {



    func testWriteReadmeImages() throws {
        guard let target = ProcessInfo.processInfo.environment["SNAPSHOT_DIR"] else {
            throw XCTSkip("set SNAPSHOT_DIR to regenerate the README images")
        }
        let directory = URL(fileURLWithPath: target, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        try write(
            indicatorGrid(),
            to: directory.appendingPathComponent("indicators.png")
        )

        // Dark is where the low-contrast track bug hid — the light render
        // looked fine while a 2% row and a 0% row were the same picture on a
        // dark background. Render both from now on.
        try write(
            MenuContentView(model: .preview(snapshot: darkSnapshot(), theme: .usage)),
            to: directory.appendingPathComponent("popover-dark.png"),
            scheme: .dark
        )
    }

    /// Deliberately includes 2% and 0% side by side: the pair that has to stay
    /// visibly different.
    private func darkSnapshot() -> UsageSnapshot {
        let now = Date()
        return UsageSnapshot(
            windows: [
                UsageEntry(kind: .fiveHour, window: UsageWindow(
                    usedPercentage: 2,
                    resetsAt: now.addingTimeInterval(4 * 3600 + 52 * 60)
                )),
                UsageEntry(kind: .sevenDay, window: UsageWindow(
                    usedPercentage: 24,
                    resetsAt: now.addingTimeInterval(12 * 3600 + 22 * 60)
                )),
                UsageEntry(kind: .weeklyScoped, window: UsageWindow(
                    usedPercentage: 0,
                    resetsAt: nil,
                    scopeLabel: "Fable"
                ))
            ],
            extraUsageEnabled: false,
            capturedAt: now,
            source: .oauth,
            plan: "Max (5x)"
        )
    }

    /// Every indicator style and theme at three levels, on one strip.
    private func indicatorGrid() -> some View {
        let levels: [Double] = [15, 62, 93]
        return VStack(alignment: .leading, spacing: 12) {
            ForEach(MenuBarStyle.allCases) { style in
                ForEach(ColorTheme.allCases) { theme in
                    HStack(spacing: 18) {
                        Text("\(style.label) · \(theme.label)")
                            .font(.system(size: 11))
                            .frame(width: 130, alignment: .leading)
                        ForEach(levels, id: \.self) { level in
                            Image(nsImage: UsageGaugeIcon.makeStatusImage(
                                percentage: level,
                                stale: false,
                                showText: true,
                                style: style,
                                theme: theme
                            ))
                            .renderingMode(.original)
                        }
                    }
                }
            }
        }
        .padding(16)
    }

    private func write(_ view: some View, to url: URL, scheme: ColorScheme = .light) throws {
        // The popover normally sits on a material; without an explicit
        // background the PNG comes out transparent and reads as broken on
        // GitHub's white page. Fixed sRGB rather than `.windowBackgroundColor`
        // because NSColor resolves against the process appearance, not the
        // SwiftUI environment, so a dark render would land on a light ground.
        let background = scheme == .dark
            ? Color(.sRGB, red: 0.13, green: 0.13, blue: 0.14)
            : Color(.sRGB, red: 1, green: 1, blue: 1)

        let renderer = ImageRenderer(
            content: view
                .environment(\.colorScheme, scheme)
                .background(background)
        )
        renderer.scale = 2

        let image = try XCTUnwrap(renderer.nsImage, "ImageRenderer produced nothing for \(url.lastPathComponent)")
        let tiff = try XCTUnwrap(image.tiffRepresentation)
        let bitmap = try XCTUnwrap(NSBitmapImageRep(data: tiff))
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        try png.write(to: url)
    }
}
