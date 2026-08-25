import XCTest
import SwiftUI
@testable import ClaudeQuotaBar

/// The label, grouping and colour rules behind the popover. All pure functions
/// over a snapshot, so they're worth pinning even though the SwiftUI layout
/// around them isn't directly testable.
final class PresentationTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_700_000_000)

    private func entry(
        _ kind: UsageWindowKind,
        _ percentage: Double,
        scope: String? = nil,
        resetsIn: TimeInterval? = nil
    ) -> UsageEntry {
        UsageEntry(
            kind: kind,
            window: UsageWindow(
                usedPercentage: percentage,
                resetsAt: resetsIn.map { now.addingTimeInterval($0) },
                scopeLabel: scope
            )
        )
    }

    private func snapshot(_ entries: [UsageEntry]) -> UsageSnapshot {
        UsageSnapshot(windows: entries, extraUsageEnabled: nil, capturedAt: now, source: .oauth)
    }

    // MARK: - Grouping

    func testSessionAndWeeklyAreSplitTheWayClaudeSplitsThem() {
        let all = snapshot([
            entry(.fiveHour, 15, resetsIn: 3600),
            entry(.sevenDay, 18, resetsIn: 68_340),
            entry(.weeklyScoped, 0, scope: "Fable")
        ])
        XCTAssertEqual(all.sessionWindows.map(\.label), ["Current session"])
        XCTAssertEqual(all.weeklyWindows.map(\.label), ["All models", "Fable"])
    }

    func testOnlyTheSessionWindowIsNonWeekly() {
        for kind in UsageWindowKind.allCases {
            XCTAssertEqual(kind.isWeekly, kind != .fiveHour, "\(kind)")
        }
    }

    // MARK: - Subtitles

    func testSubtitleIsASpelledOutCountdown() {
        let row = entry(.fiveHour, 15, resetsIn: 3 * 3600 + 19 * 60)
        XCTAssertEqual(row.subtitle(now: now), "Resets in 3 hr 19 min")
    }

    /// Claude says "You haven't used Fable yet" rather than showing a countdown
    /// on a window nothing has touched.
    func testUnusedScopedWindowSaysSoInsteadOfCountingDown() {
        let row = entry(.weeklyScoped, 0, scope: "Fable", resetsIn: 3600)
        XCTAssertEqual(row.subtitle(now: now), "You haven't used Fable yet")
    }

    func testUnusedUnscopedWindowFallsBackToGenericWording() {
        XCTAssertEqual(entry(.sevenDay, 0).subtitle(now: now), "You haven't used this yet")
    }

    func testElapsedWindowReadsAsReset() {
        XCTAssertEqual(entry(.fiveHour, 40, resetsIn: -60).subtitle(now: now), "Reset")
    }

    func testNoSubtitleWithoutAResetTime() {
        XCTAssertNil(entry(.fiveHour, 40).subtitle(now: now))
    }

    func testLongCountdownWording() {
        func text(_ seconds: TimeInterval) -> String {
            RelativeTime.longCountdown(to: now.addingTimeInterval(seconds), from: now)
        }
        XCTAssertEqual(text(30), "30 sec")
        XCTAssertEqual(text(45 * 60), "45 min")
        XCTAssertEqual(text(3 * 3600), "3 hr")
        XCTAssertEqual(text(3 * 3600 + 19 * 60), "3 hr 19 min")
        XCTAssertEqual(text(18 * 3600 + 59 * 60), "18 hr 59 min")
        XCTAssertEqual(text(24 * 3600), "1 day")
        XCTAssertEqual(text(2 * 86_400 + 4 * 3600), "2 days 4 hr")
        XCTAssertEqual(text(-5), "now")
    }

    // MARK: - Colour themes

    /// The point of the Claude theme: the fill never changes, so it can't be
    /// read as a warning.
    func testClaudeThemeIsFlatAtEveryLevel() {
        let levels = [0.0, 25, 49, 50, 79, 80, 94, 95, 100].map {
            UsageColor.nsColor($0, theme: .claude)
        }
        XCTAssertEqual(Set(levels).count, 1)
        XCTAssertEqual(levels.first, UsageColor.claudeBlue)
    }

    func testUsageThemeRampsAtTheDocumentedThresholds() {
        XCTAssertEqual(UsageColor.nsColor(49.9, theme: .usage), .systemGreen)
        XCTAssertEqual(UsageColor.nsColor(50, theme: .usage), .systemYellow)
        XCTAssertEqual(UsageColor.nsColor(79.9, theme: .usage), .systemYellow)
        XCTAssertEqual(UsageColor.nsColor(80, theme: .usage), .systemOrange)
        XCTAssertEqual(UsageColor.nsColor(94.9, theme: .usage), .systemOrange)
        XCTAssertEqual(UsageColor.nsColor(95, theme: .usage), .systemRed)
    }

    /// An untouched window gets a neutral track in both themes, which is what
    /// makes a 0% row read as empty rather than as a very small amount.
    func testZeroUsageGetsANeutralTrackInEitherTheme() {
        for theme in ColorTheme.allCases {
            XCTAssertEqual(UsageColor.trackColor(0, theme: theme), UsageColor.emptyTrack, "\(theme)")
        }
    }

    /// The regression a real dark-mode screenshot caught: the usage ramp's
    /// track used to be flat `primary.opacity(0.12)`, a hair from the empty
    /// grey, so a 2% row and a 0% row rendered identically.
    func testUsedTrackIsDistinctFromTheEmptyTrackInBothThemes() {
        for theme in ColorTheme.allCases {
            for percentage in [1.0, 2, 24, 93] {
                XCTAssertNotEqual(
                    UsageColor.trackColor(percentage, theme: theme),
                    UsageColor.emptyTrack,
                    "\(theme) at \(percentage)%"
                )
            }
        }
    }

    /// A tinted track has to follow its fill, or the ramp's colour stops
    /// meaning anything once a bar is mostly empty.
    func testTintedTrackFollowsTheFillColour() {
        XCTAssertNotEqual(
            UsageColor.trackColor(24, theme: .usage),
            UsageColor.trackColor(93, theme: .usage)
        )
        XCTAssertEqual(
            UsageColor.trackColor(24, theme: .claude),
            UsageColor.trackColor(93, theme: .claude),
            "the Claude theme is flat by design"
        )
    }

    // MARK: - Menu bar indicator

    func testBothIndicatorStylesRenderAtEveryState() {
        for style in MenuBarStyle.allCases {
            for theme in ColorTheme.allCases {
                for percentage in [nil, 0, 1, 50, 100] as [Double?] {
                    let image = UsageGaugeIcon.make(
                        percentage: percentage,
                        stale: false,
                        style: style,
                        theme: theme
                    )
                    XCTAssertGreaterThan(image.size.width, 0, "\(style) \(theme) \(String(describing: percentage))")
                    XCTAssertGreaterThan(image.size.height, 0)
                }
            }
        }
    }

    func testPercentageTextWidensTheStatusImage() {
        for style in MenuBarStyle.allCases {
            let bare = UsageGaugeIcon.makeStatusImage(
                percentage: 42, stale: false, showText: false, style: style, theme: .claude
            )
            let labelled = UsageGaugeIcon.makeStatusImage(
                percentage: 42, stale: false, showText: true, style: style, theme: .claude
            )
            XCTAssertGreaterThan(labelled.size.width, bare.size.width, "\(style)")
        }
    }

    func testRingAndBarAreDifferentShapes() {
        let ring = UsageGaugeIcon.make(percentage: 50, stale: false, style: .ring, theme: .claude)
        let bar = UsageGaugeIcon.make(percentage: 50, stale: false, style: .bar, theme: .claude)
        XCTAssertNotEqual(ring.size, bar.size)
    }

    // MARK: - Plan badge

    func testPlanLabelFromRateLimitTier() {
        XCTAssertEqual(
            ClaudeCredentials.planLabel(subscriptionType: "max", rateLimitTier: "default_claude_max_5x"),
            "Max (5x)"
        )
        XCTAssertEqual(
            ClaudeCredentials.planLabel(subscriptionType: "max", rateLimitTier: "default_claude_max_20x"),
            "Max (20x)"
        )
        XCTAssertEqual(
            ClaudeCredentials.planLabel(subscriptionType: "pro", rateLimitTier: "default_claude_pro"),
            "Pro"
        )
    }

    func testPlanLabelFallsBackToSubscriptionType() {
        XCTAssertEqual(ClaudeCredentials.planLabel(subscriptionType: "max", rateLimitTier: nil), "Max")
        XCTAssertEqual(ClaudeCredentials.planLabel(subscriptionType: "max", rateLimitTier: ""), "Max")
        XCTAssertNil(ClaudeCredentials.planLabel(subscriptionType: nil, rateLimitTier: nil))
    }

    /// The generic tier names the product, not the plan, so it has to defer to
    /// the subscription type.
    ///
    /// A plain Pro account reports `default_claude_ai`. Stripping the prefixes
    /// leaves the bare word "ai", and taking that at face value badged the
    /// account "Ai" in the popover header.
    func testGenericTierDefersToSubscriptionType() {
        XCTAssertEqual(
            ClaudeCredentials.planLabel(subscriptionType: "pro", rateLimitTier: "default_claude_ai"),
            "Pro"
        )
        XCTAssertEqual(
            ClaudeCredentials.planLabel(subscriptionType: "max", rateLimitTier: "claude_ai"),
            "Max"
        )
        // Nothing to fall back to is better said with no badge than with "Ai".
        XCTAssertNil(
            ClaudeCredentials.planLabel(subscriptionType: nil, rateLimitTier: "default_claude_ai")
        )
        // A tier that does name a plan still wins over the subscription type.
        XCTAssertEqual(
            ClaudeCredentials.planLabel(
                subscriptionType: "pro",
                rateLimitTier: "default_claude_ai_max_5x"
            ),
            "Max (5x)"
        )
    }

    /// An unfamiliar tier should still produce something readable rather than
    /// leaking a raw identifier into the header.
    func testUnknownTierIsStillHumanReadable() {
        XCTAssertEqual(
            ClaudeCredentials.planLabel(subscriptionType: nil, rateLimitTier: "default_claude_team_premium_3x"),
            "Team Premium (3x)"
        )
    }
}
