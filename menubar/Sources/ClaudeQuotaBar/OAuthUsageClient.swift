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
            guard let snapshot = Self.parse(data) else {
                throw Failure.transport(.noData)
            }
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

        let keys: [(UsageWindowKind, String)] = [
            (.fiveHour, "five_hour"),
            (.sevenDay, "seven_day"),
            (.sevenDayOpus, "seven_day_opus"),
            (.sevenDaySonnet, "seven_day_sonnet")
        ]

        var windows: [UsageWindowKind: UsageWindow] = [:]
        for (kind, key) in keys {
            guard let object = root[key] as? [String: Any],
                  let utilization = object["utilization"] as? Double else { continue }
            windows[kind] = UsageWindow(
                usedPercentage: utilization,
                resetsAt: decodeDate(object["resets_at"])
            )
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

    /// `resets_at` comes back as an ISO-8601 string here and as epoch seconds in
    /// the status line payload, so accept either.
    static func decodeDate(_ value: Any?) -> Date? {
        if let seconds = value as? Double, seconds > 0 {
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
