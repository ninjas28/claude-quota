import AppKit
import SwiftUI

/// Shared colour ramp so the menu bar ring and the popover bars always agree.
enum UsageColor {
    static func level(_ percentage: Double) -> (nsColor: NSColor, color: Color) {
        switch percentage {
        case ..<50: return (.systemGreen, .green)
        case ..<80: return (.systemYellow, .yellow)
        case ..<95: return (.systemOrange, .orange)
        default: return (.systemRed, .red)
        }
    }

    static func nsColor(_ percentage: Double) -> NSColor { level(percentage).nsColor }
    static func color(_ percentage: Double) -> Color { level(percentage).color }
}

/// Draws the little ring shown in the menu bar.
///
/// Deliberately Core Graphics rather than a SwiftUI view: `MenuBarExtra` only
/// renders `Text` and `Image` reliably in its label, so we hand it a finished
/// bitmap. `isTemplate` stays off because the fill colour carries meaning.
enum UsageGaugeIcon {
    static func make(percentage: Double?, stale: Bool) -> NSImage {
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

            let color = UsageColor.nsColor(clamped)
            (stale ? color.withAlphaComponent(0.45) : color).setStroke()
            progress.stroke()
            return true
        }
        image.isTemplate = false
        return image
    }
}

extension UsageGaugeIcon {
    /// Ring plus percentage baked into one image.
    ///
    /// `MenuBarExtra` labels only render `Text` and `Image` dependably on
    /// macOS 13 — an `HStack` of both can silently drop an element — so the
    /// whole label is drawn once here and handed over as a single `Image`.
    static func makeStatusImage(percentage: Double?, stale: Bool, showText: Bool) -> NSImage {
        let ring = make(percentage: percentage, stale: stale)
        guard showText else { return ring }

        let text = percentage.map { "\(Int(min(max($0, 0), 100).rounded()))%" } ?? "--"
        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: 11, weight: .medium),
            .foregroundColor: stale ? NSColor.secondaryLabelColor : NSColor.labelColor
        ]
        let attributed = NSAttributedString(string: text, attributes: attributes)
        let textSize = attributed.size()

        let spacing: CGFloat = 3
        let totalSize = NSSize(
            width: ring.size.width + spacing + ceil(textSize.width),
            height: max(ring.size.height, ceil(textSize.height))
        )

        let composed = NSImage(size: totalSize, flipped: false) { rect in
            ring.draw(
                in: NSRect(
                    x: 0,
                    y: (rect.height - ring.size.height) / 2,
                    width: ring.size.width,
                    height: ring.size.height
                )
            )
            attributed.draw(
                at: NSPoint(
                    x: ring.size.width + spacing,
                    y: (rect.height - textSize.height) / 2
                )
            )
            return true
        }
        composed.isTemplate = false
        return composed
    }
}

/// Horizontal progress bar used in the popover rows.
struct UsageBar: View {
    var percentage: Double
    var stale: Bool

    var body: some View {
        GeometryReader { geometry in
            ZStack(alignment: .leading) {
                Capsule()
                    .fill(Color.primary.opacity(0.12))
                Capsule()
                    .fill(UsageColor.color(percentage).opacity(stale ? 0.45 : 1))
                    .frame(width: max(0, min(1, percentage / 100)) * geometry.size.width)
            }
        }
        .frame(height: 6)
        .accessibilityLabel(Text("\(Int(percentage.rounded())) percent used"))
    }
}
