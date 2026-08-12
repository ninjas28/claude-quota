import XCTest
@testable import ClaudeQuotaBar

/// Covers the parsing layer, which is where a silent wrong-number bug would
/// live: every field arrives as `Any` and a bad cast yields `nil` rather than
/// an error, so a rename or a scale change shows up as an empty or wrong menu
/// bar instead of a crash.
final class OAuthUsageClientTests: XCTestCase {

    /// Shaped like a real `GET /api/oauth/usage` response, trimmed to the parts
    /// we read plus a couple of the buckets we deliberately ignore.
    private let liveShapedResponse = """
    {
      "five_hour":  { "utilization": 12.0, "resets_at": "2026-08-09T00:19:59.589710+00:00" },
      "seven_day":  { "utilization": 17.0, "resets_at": "2026-08-09T15:59:59.589737+00:00" },
      "nimbus_quill": { "utilization": 0.0, "resets_at": null },
      "extra_usage": { "is_enabled": false },
      "limits": [
        { "kind": "session",    "group": "session", "percent": 12,
          "resets_at": "2026-08-09T00:19:59.589710+00:00", "scope": null },
        { "kind": "weekly_all", "group": "weekly",  "percent": 17,
          "resets_at": "2026-08-09T15:59:59.589737+00:00", "scope": null },
        { "kind": "weekly_scoped", "group": "weekly", "percent": 0, "resets_at": null,
          "scope": { "model": { "id": null, "display_name": "Fable" } } }
      ]
    }
    """

    private func parse(_ json: String) -> UsageSnapshot? {
        OAuthUsageClient.parse(Data(json.utf8))
    }

    // MARK: - Scale

    /// The single most dangerous confusion in this repo. The response headers
    /// the legacy CLI reads report 0-1 fractions; this endpoint reports 0-100.
    /// Both land in `UsageWindow.usedPercentage`, which the colour ramp and the
    /// ring both read as 0-100 — so getting this backwards renders 12% as 0%,
    /// green, with no error anywhere.
    func testUtilizationIsPercentageNotFraction() throws {
        let snapshot = try XCTUnwrap(parse(liveShapedResponse))
        XCTAssertEqual(try XCTUnwrap(snapshot[.fiveHour]).usedPercentage, 12.0)
        XCTAssertEqual(try XCTUnwrap(snapshot[.sevenDay]).usedPercentage, 17.0)
    }

    // MARK: - limits array

    func testPrefersLimitsArray() throws {
        let snapshot = try XCTUnwrap(parse(liveShapedResponse))
        XCTAssertEqual(snapshot.presentWindows.count, 3)
    }

    /// The whole reason to read `limits`: a model-scoped weekly ceiling exists
    /// only as a `weekly_scoped` entry.
    func testScopedWeeklyWindowIsLabelledWithItsModel() throws {
        let snapshot = try XCTUnwrap(parse(liveShapedResponse))
        let scoped = try XCTUnwrap(snapshot.presentWindows.first { $0.kind == .weeklyScoped })
        XCTAssertEqual(scoped.window.scopeLabel, "Fable")
        // Named after its model; the "Weekly limits" section header supplies
        // the rest of the context, as it does on Claude's own usage screen.
        XCTAssertEqual(scoped.label, "Fable")
        XCTAssertTrue(scoped.kind.isWeekly)
    }

    func testUnknownLimitKindsAreSkipped() throws {
        let json = """
        { "limits": [ { "kind": "session", "percent": 5, "resets_at": null },
                      { "kind": "iguana_necktie", "percent": 99, "resets_at": null } ] }
        """
        let snapshot = try XCTUnwrap(parse(json))
        XCTAssertEqual(snapshot.presentWindows.map(\.kind), [.fiveHour])
    }

    /// An account can be scoped on more than one model. These share a `kind`,
    /// which is exactly what the old dictionary-keyed snapshot could not hold.
    func testSeveralScopedWindowsAllSurvive() throws {
        let json = """
        { "limits": [
            { "kind": "weekly_scoped", "percent": 10, "resets_at": null,
              "scope": { "model": { "display_name": "Opus" } } },
            { "kind": "weekly_scoped", "percent": 40, "resets_at": null,
              "scope": { "model": { "display_name": "Sonnet" } } } ] }
        """
        let snapshot = try XCTUnwrap(parse(json))
        XCTAssertEqual(snapshot.presentWindows.map(\.window.scopeLabel), ["Opus", "Sonnet"])
        XCTAssertEqual(Set(snapshot.presentWindows.map(\.id)).count, 2, "ids must stay unique")
        XCTAssertEqual(snapshot.peak?.window.scopeLabel, "Sonnet")
    }

    // MARK: - Legacy shape

    func testFallsBackToTopLevelKeysWithoutLimits() throws {
        let json = """
        { "five_hour": { "utilization": 30.0, "resets_at": null },
          "seven_day": { "utilization": 55.0, "resets_at": null } }
        """
        let snapshot = try XCTUnwrap(parse(json))
        XCTAssertEqual(snapshot.presentWindows.map(\.kind), [.fiveHour, .sevenDay])
        XCTAssertEqual(snapshot[.sevenDay]?.usedPercentage, 55.0)
    }

    func testNullWindowsAreSkippedNotZeroed() throws {
        let json = """
        { "five_hour": { "utilization": 30.0, "resets_at": null }, "seven_day": null }
        """
        let snapshot = try XCTUnwrap(parse(json))
        XCTAssertNil(snapshot[.sevenDay], "a null window must be absent, not 0%")
    }

    // MARK: - Degenerate input

    func testGarbageReturnsNil() {
        XCTAssertNil(parse("not json"))
        XCTAssertNil(parse("[1,2,3]"))
        XCTAssertNil(parse("{}"), "no windows at all is no snapshot")
        XCTAssertNil(parse(#"{"limits": []}"#))
    }

    func testExtraUsageFlag() throws {
        XCTAssertEqual(try XCTUnwrap(parse(liveShapedResponse)).extraUsageEnabled, false)
        let on = #"{"five_hour":{"utilization":1},"extra_usage":{"is_enabled":true}}"#
        XCTAssertEqual(try XCTUnwrap(parse(on)).extraUsageEnabled, true)
        let absent = #"{"five_hour":{"utilization":1}}"#
        XCTAssertNil(try XCTUnwrap(parse(absent)).extraUsageEnabled)
    }

    // MARK: - decodeNumber / decodeDate

    func testDecodeNumberAcceptsIntAndDouble() {
        // `percent` arrives as an integer, `utilization` as a real.
        XCTAssertEqual(OAuthUsageClient.decodeNumber(12 as Any), 12.0)
        XCTAssertEqual(OAuthUsageClient.decodeNumber(12.5 as Any), 12.5)
        XCTAssertNil(OAuthUsageClient.decodeNumber("12" as Any))
        XCTAssertNil(OAuthUsageClient.decodeNumber(nil))
    }

    /// `true` bridges to NSNumber, so a naive numeric cast turns a boolean flag
    /// into 100% used.
    func testDecodeNumberRejectsBooleans() {
        XCTAssertNil(OAuthUsageClient.decodeNumber(true as Any))
        XCTAssertNil(OAuthUsageClient.decodeNumber(false as Any))
    }

    func testDecodeDateAcceptsBothSourceFormats() throws {
        // ISO-8601 with fractional seconds — the usage endpoint.
        let fractional = try XCTUnwrap(
            OAuthUsageClient.decodeDate("2026-08-09T00:19:59.589710+00:00")
        )
        XCTAssertEqual(fractional.timeIntervalSince1970, 1786234799.589, accuracy: 1)

        // ISO-8601 without fractional seconds.
        XCTAssertNotNil(OAuthUsageClient.decodeDate("2026-08-09T00:19:59Z"))

        // Epoch seconds as an integer — the status line cache.
        XCTAssertEqual(
            OAuthUsageClient.decodeDate(1738425600 as Any),
            Date(timeIntervalSince1970: 1738425600)
        )
    }

    func testDecodeDateRejectsJunk() {
        XCTAssertNil(OAuthUsageClient.decodeDate(nil))
        XCTAssertNil(OAuthUsageClient.decodeDate(""))
        XCTAssertNil(OAuthUsageClient.decodeDate("yesterday"))
        XCTAssertNil(OAuthUsageClient.decodeDate(0 as Any), "epoch 0 is not a real reset time")
    }
}
