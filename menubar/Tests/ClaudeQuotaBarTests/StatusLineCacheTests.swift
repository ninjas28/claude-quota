import XCTest
@testable import ClaudeQuotaBar

/// The Swift half of the cross-language cache contract.
///
/// `tests/test_statusline.py` pins the shape the Python bridge *writes*, and
/// asserts that the key names it uses still appear in `StatusLineCache.swift`.
/// This file pins what Swift *reads* out of that same shape. Together they
/// cover a rename on either side; neither does on its own.
final class StatusLineCacheTests: XCTestCase {
    private var directory: URL!

    override func setUpWithError() throws {
        directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("quota-bar-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        setenv("CLAUDE_QUOTA_CACHE", directory.appendingPathComponent("cache.json").path, 1)
    }

    override func tearDownWithError() throws {
        unsetenv("CLAUDE_QUOTA_CACHE")
        try? FileManager.default.removeItem(at: directory)
    }

    private func writeCache(_ json: String) throws {
        try Data(json.utf8).write(to: StatusLineCache.fileURL)
    }

    /// Byte-for-byte what the bridge writes for a Pro/Max session.
    private let bridgeOutput = """
    {
      "five_hour": { "used_percentage": 23.5, "resets_at": 1738425600 },
      "seven_day": { "used_percentage": 41.2, "resets_at": 1738857600 },
      "captured_at": 1738420000.0,
      "model": "Opus 5",
      "session_id": "sess-abc123"
    }
    """

    func testReadsWhatTheBridgeWrites() throws {
        try writeCache(bridgeOutput)
        let (snapshot, context) = try XCTUnwrap(StatusLineCache.read())

        XCTAssertEqual(snapshot[.fiveHour]?.usedPercentage, 23.5)
        XCTAssertEqual(snapshot[.sevenDay]?.usedPercentage, 41.2)
        XCTAssertEqual(snapshot[.fiveHour]?.resetsAt, Date(timeIntervalSince1970: 1738425600))
        XCTAssertEqual(snapshot.capturedAt, Date(timeIntervalSince1970: 1738420000))
        XCTAssertEqual(snapshot.source, .statusLine)
        XCTAssertEqual(context.model, "Opus 5")
        XCTAssertEqual(context.sessionID, "sess-abc123")
    }

    /// The bridge writes `used_percentage` as 0-100 and `resets_at` as an
    /// integer epoch. Integers must survive the `Double` cast.
    func testIntegerPercentagesAndEpochsSurvive() throws {
        try writeCache("""
        { "five_hour": { "used_percentage": 50, "resets_at": 1738425600 } }
        """)
        let (snapshot, _) = try XCTUnwrap(StatusLineCache.read())
        XCTAssertEqual(snapshot[.fiveHour]?.usedPercentage, 50.0)
        XCTAssertNotNil(snapshot[.fiveHour]?.resetsAt)
    }

    func testPartialCacheStillReads() throws {
        try writeCache(#"{ "five_hour": { "used_percentage": 9.0, "resets_at": null } }"#)
        let (snapshot, _) = try XCTUnwrap(StatusLineCache.read())
        XCTAssertEqual(snapshot.presentWindows.map(\.kind), [.fiveHour])
        XCTAssertNil(snapshot[.fiveHour]?.resetsAt)
    }

    /// The bridge stamps `captured_at`, but if it ever stopped, treating the
    /// file as brand new would make stale data look permanently fresh.
    func testFallsBackToFileModificationTime() throws {
        try writeCache(#"{ "five_hour": { "used_percentage": 9.0, "resets_at": null } }"#)
        let (snapshot, _) = try XCTUnwrap(StatusLineCache.read())
        XCTAssertEqual(snapshot.capturedAt.timeIntervalSinceNow, 0, accuracy: 5)
    }

    func testMissingOrJunkFileYieldsNil() throws {
        XCTAssertNil(StatusLineCache.read(), "no file at all")
        try writeCache("not json")
        XCTAssertNil(StatusLineCache.read())
        try writeCache("{}")
        XCTAssertNil(StatusLineCache.read(), "no windows means no snapshot")
    }
}

final class BackoffTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_700_000_000)

    private func seconds(consecutive: Int, retryAt: Date? = nil) -> TimeInterval {
        UsageModel.backoffDeadline(
            consecutiveRateLimits: consecutive,
            retryAt: retryAt,
            now: now
        ).timeIntervalSince(now)
    }

    /// The bug this replaced: the interval was `pollInterval * 2^n`, so at the
    /// 30-minute poll setting the very first 429 already sat on the ceiling.
    func testFirstRateLimitCostsOneMinuteRegardlessOfPollInterval() {
        XCTAssertEqual(seconds(consecutive: 1), 60)
    }

    func testDoublesPerConsecutiveRateLimit() {
        XCTAssertEqual(seconds(consecutive: 2), 120)
        XCTAssertEqual(seconds(consecutive: 3), 240)
        XCTAssertEqual(seconds(consecutive: 4), 480)
    }

    func testClampsToCeiling() {
        XCTAssertEqual(seconds(consecutive: 12), Defaults.maxBackoff)
    }

    func testLongerRetryAfterWins() {
        let retryAt = now.addingTimeInterval(600)
        XCTAssertEqual(seconds(consecutive: 1, retryAt: retryAt), 600)
    }

    /// A server that says "5 seconds" and then 429s again would otherwise get
    /// hammered at 5-second intervals forever.
    func testShorterRetryAfterCannotUndercutTheExponential() {
        let retryAt = now.addingTimeInterval(5)
        XCTAssertEqual(seconds(consecutive: 3, retryAt: retryAt), 240)
    }
}
