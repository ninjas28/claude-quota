import Foundation

/// Fetches usage from the same endpoint Claude Code's `/usage` command calls.
///
/// This costs no quota — it reports your windows, it doesn't consume them — but
/// it is an undocumented endpoint and it does rate limit, so `UsageModel` only
/// reaches for it when the local status line cache has gone stale.
struct OAuthUsageClient {
    private static let endpoint = URL(string: "https://api.anthropic.com/api/oauth/usage")!
    private static let betaHeader = "oauth-2025-04-20"

    enum Failure: Error {
        case credentials(UsageError)
        case transport(UsageError)
    }

    var session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 15
        configuration.timeoutIntervalForResource = 20
        // The response is a live counter; a cached copy is worse than no copy.
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        return URLSession(configuration: configuration)
    }()

    func fetch() async throws -> UsageSnapshot {
        guard let credentials = ClaudeCredentialStore.load() else {
            throw Failure.credentials(.noCredentials)
        }
        guard !credentials.isExpired else {
            throw Failure.credentials(.credentialsExpired)
        }

        var request = URLRequest(url: Self.endpoint)
        request.httpMethod = "GET"
        request.setValue("Bearer \(credentials.accessToken)", forHTTPHeaderField: "Authorization")
        request.setValue(Self.betaHeader, forHTTPHeaderField: "anthropic-beta")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("ClaudeQuotaBar/1.0", forHTTPHeaderField: "User-Agent")

        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: request)
        } catch {
            throw Failure.transport(.network(error.localizedDescription))
        }

        guard let http = response as? HTTPURLResponse else {
            throw Failure.transport(.network("Unexpected response"))
        }

        switch http.statusCode {
        case 200:
            guard var snapshot = Self.parse(data) else {
                throw Failure.transport(.noData)
            }
            // The plan badge isn't in the payload — it rides on the credential
            // we just used, so stamp it here rather than reading the Keychain
            // a second time from the view.
            snapshot.plan = credentials.planLabel
            return snapshot
        case 401, 403:
            throw Failure.credentials(.credentialsExpired)
        case 404:
            // Returned for accounts without subscription-backed windows.
            throw Failure.credentials(.notSubscribed)
        case 429:
            throw Failure.transport(.rateLimited(retryAt: Self.retryDate(from: http)))
        default:
            throw Failure.transport(.network("HTTP \(http.statusCode)"))
        }
    }

    /// Honour `Retry-After`, which may be either seconds or an HTTP date.
    private static func retryDate(from response: HTTPURLResponse) -> Date? {
        guard let value = response.value(forHTTPHeaderField: "Retry-After") else { return nil }
        if let seconds = TimeInterval(value.trimmingCharacters(in: .whitespaces)) {
            return Date().addingTimeInterval(seconds)
        }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(identifier: "GMT")
        formatter.dateFormat = "EEE, dd MMM yyyy HH:mm:ss zzz"
        return formatter.date(from: value)
    }

    static func parse(_ data: Data) -> UsageSnapshot? {
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return nil
        }

        // Prefer the `limits` array. It is self-describing (kind, percent,
        // scope) and it is the only place a model-scoped weekly window shows
        // up. The top-level keys are the fallback for accounts that don't
        // report one.
        var windows = parseLimits(root["limits"])
        if windows.isEmpty {
            windows = parseTopLevelWindows(root)
        }
        guard !windows.isEmpty else { return nil }

        let extraUsage = root["extra_usage"] as? [String: Any]
        return UsageSnapshot(
            windows: windows,
            extraUsageEnabled: extraUsage?["is_enabled"] as? Bool,
            capturedAt: Date(),
            source: .oauth
        )
    }

    /// `kind` values we know how to display. Anything else (the endpoint also
    /// reports several internal buckets) is skipped rather than guessed at.
    private static let limitKinds: [String: UsageWindowKind] = [
        "session": .fiveHour,
        "weekly_all": .sevenDay,
        "weekly_scoped": .weeklyScoped
    ]

    static func parseLimits(_ value: Any?) -> [UsageEntry] {
        guard let entries = value as? [[String: Any]] else { return [] }
        return entries.compactMap { entry in
            guard let kindName = entry["kind"] as? String,
                  let kind = limitKinds[kindName],
                  let percent = decodeNumber(entry["percent"]) else { return nil }
            let model = (entry["scope"] as? [String: Any])?["model"] as? [String: Any]
            return UsageEntry(
                kind: kind,
                window: UsageWindow(
                    usedPercentage: percent,
                    resetsAt: decodeDate(entry["resets_at"]),
                    scopeLabel: model?["display_name"] as? String
                )
            )
        }
    }

    /// The per-key shape, for accounts that report no `limits` array. Still
    /// live data, unlike the `seven_day_opus` / `seven_day_sonnet` keys that
    /// used to be read here: those come back null now that scoped windows
    /// arrive through `limits`.
    static func parseTopLevelWindows(_ root: [String: Any]) -> [UsageEntry] {
        let keys: [(UsageWindowKind, String)] = [
            (.fiveHour, "five_hour"),
            (.sevenDay, "seven_day")
        ]
        return keys.compactMap { kind, key in
            guard let object = root[key] as? [String: Any],
                  let utilization = decodeNumber(object["utilization"]) else { return nil }
            return UsageEntry(
                kind: kind,
                window: UsageWindow(
                    usedPercentage: utilization,
                    resetsAt: decodeDate(object["resets_at"])
                )
            )
        }
    }

    /// JSON numbers arrive as `NSNumber`, and `percent` is an integer while
    /// `utilization` is a real. Accept either rather than depending on how
    /// Foundation happens to bridge a given literal.
    static func decodeNumber(_ value: Any?) -> Double? {
        guard let value else { return nil }
        if let number = value as? NSNumber {
            // Booleans bridge to NSNumber — but so do the literals 0 and 1, and
            // `number is Bool` is true for all four. Testing the CoreFoundation
            // type is the only way to tell a real boolean from the number 1.
            // Getting this wrong silently drops any window sitting at 0% or 1%.
            guard CFGetTypeID(number as CFTypeRef) != CFBooleanGetTypeID() else { return nil }
            return number.doubleValue
        }
        if let double = value as? Double { return double }
        if let integer = value as? Int { return Double(integer) }
        return nil
    }

    /// `resets_at` comes back as an ISO-8601 string here and as epoch seconds in
    /// the status line payload, so accept either.
    static func decodeDate(_ value: Any?) -> Date? {
        if let seconds = decodeNumber(value), seconds > 0 {
            return Date(timeIntervalSince1970: seconds)
        }
        guard let string = value as? String, !string.isEmpty else { return nil }

        let withFractional = ISO8601DateFormatter()
        withFractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = withFractional.date(from: string) { return date }

        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return plain.date(from: string)
    }
}
