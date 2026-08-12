import AppKit
import SwiftUI

/// Shared colour rules so the menu bar indicator and the popover bars always
/// agree. Every colour in the app comes from here.
enum UsageColor {
    /// Claude's usage-screen blue. Fixed rather than `controlAccentColor`: it
    /// is meant to look like Claude, not like whatever accent the user picked.
    /// Saturated enough to hold up on both light and dark menu bars.
    static let claudeBlue = NSColor(srgbRed: 0.243, green: 0.388, blue: 0.867, alpha: 1)

    static func nsColor(_ percentage: Double, theme: ColorTheme) -> NSColor {
        switch theme {
        case .claude:
            return claudeBlue
        case .usage:
            switch percentage {
            case ..<50: return .systemGreen
            case ..<80: return .systemYellow
            case ..<95: return .systemOrange
            default: return .systemRed
            }
        }
    }

    static func color(_ percentage: Double, theme: ColorTheme) -> Color {
        Color(nsColor: nsColor(percentage, theme: theme))
    }

    /// Neutral track for a window nothing has touched.
    static let emptyTrack = Color.primary.opacity(0.13)

    /// The unfilled remainder of a bar.
    ///
    /// Tinted with the fill colour, dropping to neutral grey when the window is
    /// untouched — that contrast is what makes an unused row read as "nothing
    /// here" rather than "something, very small".
    ///
    /// Both themes tint. An earlier version left the usage ramp's track a flat
    /// `primary.opacity(0.12)`, which put it within a hair of the empty grey:
    /// on a dark background a 2% row and a 0% row were the same picture.
    static func trackColor(_ percentage: Double, theme: ColorTheme) -> Color {
        guard percentage > 0 else { return emptyTrack }
        return color(percentage, theme: theme).opacity(0.24)
    }
}

/// Draws the menu bar indicator.
///
/// Deliberately Core Graphics rather than a SwiftUI view: `MenuBarExtra` only
/// renders `Text` and `Image` reliably in its label, so we hand it a finished
/// bitmap. `isTemplate` stays off because the fill colour carries meaning.
enum UsageGaugeIcon {
    static func make(
        percentage: Double?,
        stale: Bool,
        style: MenuBarStyle,
        theme: ColorTheme
    ) -> NSImage {
        switch style {
        case .ring: return ring(percentage: percentage, stale: stale, theme: theme)
        case .bar: return bar(percentage: percentage, stale: stale, theme: theme)
        }
    }

    private static func fill(_ clamped: Double, stale: Bool, theme: ColorTheme) -> NSColor {
        let color = UsageColor.nsColor(clamped, theme: theme)
        return stale ? color.withAlphaComponent(0.45) : color
    }

    private static func ring(percentage: Double?, stale: Bool, theme: ColorTheme) -> NSImage {
        let size = NSSize(width: 16, height: 16)
        let image = NSImage(size: size, flipped: false) { rect in
            let center = CGPoint(x: rect.midX, y: rect.midY)
            let radius: CGFloat = 6.5
            let lineWidth: CGFloat = 2.5

            let track = NSBezierPath()
            track.appendArc(withCenter: center, radius: radius, startAngle: 0, endAngle: 360)
            track.lineWidth = lineWidth
            NSColor.tertiaryLabelColor.setStroke()
            track.stroke()

            guard let percentage else {
                // Unknown state: a dash inside an empty ring.
                let dash = NSBezierPath()
                dash.move(to: CGPoint(x: center.x - 3, y: center.y))
                dash.line(to: CGPoint(x: center.x + 3, y: center.y))
                dash.lineWidth = 1.5
                NSColor.tertiaryLabelColor.setStroke()
                dash.stroke()
                return true
            }

            let clamped = min(max(percentage, 0), 100)
            guard clamped > 0 else { return true }

            // Start at 12 o'clock and sweep clockwise.
            let start: CGFloat = 90
            let end = start - CGFloat(clamped / 100) * 360

            let progress = NSBezierPath()
            progress.appendArc(
                withCenter: center,
                radius: radius,
                startAngle: start,
                endAngle: end,
                clockwise: true
            )
            progress.lineWidth = lineWidth
            progress.lineCapStyle = .round
            fill(clamped, stale: stale, theme: theme).setStroke()
            progress.stroke()
            return true
        }
        image.isTemplate = false
        return image
    }

    /// The capsule from Claude's usage screen, shrunk to menu bar size.
    private static func bar(percentage: Double?, stale: Bool, theme: ColorTheme) -> NSImage {
        let size = NSSize(width: 24, height: 16)
        let height: CGFloat = 6

        let image = NSImage(size: size, flipped: false) { rect in
            let track = NSRect(
                x: 0,
                y: (rect.height - height) / 2,
                width: rect.width,
                height: height
            )
            let radius = height / 2

            let clamped = percentage.map { min(max($0, 0), 100) }
            let trackColor: NSColor = {
                guard let clamped, clamped > 0 else { return .tertiaryLabelColor }
                switch theme {
                case .claude: return UsageColor.nsColor(clamped, theme: theme).withAlphaComponent(0.28)
                case .usage: return .tertiaryLabelColor
                }
            }()
            NSBezierPath(roundedRect: track, xRadius: radius, yRadius: radius).fill(
                with: trackColor
            )

            guard let clamped, clamped > 0 else { return true }

            // Never let a non-zero reading round away to an invisible sliver —
            // a bar that looks empty at 2% is worse than one that overstates it.
            let width = max(height, track.width * CGFloat(clamped / 100))
            let filled = NSRect(x: track.minX, y: track.minY, width: width, height: height)
            NSBezierPath(roundedRect: filled, xRadius: radius, yRadius: radius).fill(
                with: fill(clamped, stale: stale, theme: theme)
            )
            return true
        }
        image.isTemplate = false
        return image
    }
}

private extension NSBezierPath {
    func fill(with color: NSColor) {
        color.setFill()
        fill()
    }
}

extension UsageGaugeIcon {
    /// Indicator plus percentage baked into one image.
    ///
    /// `MenuBarExtra` labels only render `Text` and `Image` dependably on
    /// macOS 13 — an `HStack` of both can silently drop an element — so the
    /// whole label is drawn once here and handed over as a single `Image`.
    static func makeStatusImage(
        percentage: Double?,
        stale: Bool,
        showText: Bool,
        style: MenuBarStyle,
        theme: ColorTheme
    ) -> NSImage {
        let indicator = make(percentage: percentage, stale: stale, style: style, theme: theme)
        guard showText else { return indicator }

        let text = percentage.map { "\(Int(min(max($0, 0), 100).rounded()))%" } ?? "--"
        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: 11, weight: .medium),
            .foregroundColor: stale ? NSColor.secondaryLabelColor : NSColor.labelColor
        ]
        let attributed = NSAttributedString(string: text, attributes: attributes)
        let textSize = attributed.size()

        let spacing: CGFloat = 3
        let totalSize = NSSize(
            width: indicator.size.width + spacing + ceil(textSize.width),
            height: max(indicator.size.height, ceil(textSize.height))
        )

        let composed = NSImage(size: totalSize, flipped: false) { rect in
            indicator.draw(
                in: NSRect(
                    x: 0,
                    y: (rect.height - indicator.size.height) / 2,
                    width: indicator.size.width,
                    height: indicator.size.height
                )
            )
            attributed.draw(
                at: NSPoint(
                    x: indicator.size.width + spacing,
                    y: (rect.height - textSize.height) / 2
                )
            )
            return true
        }
        composed.isTemplate = false
        return composed
    }
}

/// The capsule bar used in the popover rows.
struct UsageBar: View {
    var percentage: Double
    var stale: Bool
    var theme: ColorTheme

    var body: some View {
        GeometryReader { geometry in
            ZStack(alignment: .leading) {
                Capsule()
                    .fill(UsageColor.trackColor(percentage, theme: theme))
                if percentage > 0 {
                    Capsule()
                        .fill(UsageColor.color(percentage, theme: theme).opacity(stale ? 0.45 : 1))
                        .frame(
                            width: max(
                                Layout.barHeight,
                                min(1, percentage / 100) * geometry.size.width
                            )
                        )
                }
            }
        }
        .frame(height: Layout.barHeight)
        .accessibilityLabel(Text("\(Int(percentage.rounded())) percent used"))
    }

    enum Layout {
        static let barHeight: CGFloat = 7
    }
}
