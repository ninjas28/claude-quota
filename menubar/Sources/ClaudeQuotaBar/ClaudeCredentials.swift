import Foundation
import Security

/// The OAuth credential Claude Code stores after you sign in.
struct ClaudeCredentials {
    var accessToken: String
    var expiresAt: Date?
    var subscriptionType: String?
    var rateLimitTier: String?

    /// Treat a token as unusable slightly before its stated expiry so we don't
    /// fire a request that is guaranteed to 401 mid-flight.
    var isExpired: Bool {
        guard let expiresAt else { return false }
        return expiresAt.timeIntervalSinceNow < 30
    }

    var planLabel: String? {
        Self.planLabel(subscriptionType: subscriptionType, rateLimitTier: rateLimitTier)
    }

    /// The plan badge Claude's own usage screen shows, rebuilt from what the
    /// credential happens to carry: `default_claude_max_5x` -> "Max (5x)".
    /// Falls back to `subscriptionType` when the tier is missing or unfamiliar.
    static func planLabel(subscriptionType: String?, rateLimitTier: String?) -> String? {
        if let tier = rateLimitTier?.lowercased(), !tier.isEmpty {
            var parts = tier
                .replacingOccurrences(of: "default_", with: "")
                .replacingOccurrences(of: "claude_", with: "")
                .split(separator: "_")
                .map(String.init)
                // "ai" is what "claude_ai" leaves behind: it names the product,
                // not the plan. On a plain Pro account the tier is exactly
                // `default_claude_ai`, so it is all that survives the stripping
                // above — and the header ends up badging the account "Ai".
                .filter { $0 != "ai" }

            // A trailing "5x" is the plan multiplier, not part of its name.
            var multiplier: String?
            if let last = parts.last, last.hasSuffix("x"), Int(last.dropLast()) != nil {
                multiplier = last
                parts.removeLast()
            }

            let name = parts.map(\.capitalized).joined(separator: " ")
            if !name.isEmpty {
                return multiplier.map { "\(name) (\($0))" } ?? name
            }
        }

        guard let subscriptionType, !subscriptionType.isEmpty else { return nil }
        return subscriptionType.capitalized
    }
}

/// Reads Claude Code's stored login.
///
/// This is strictly read-only. We never write, refresh, or rotate the
/// credential: refreshing would rotate the refresh token out from under Claude
/// Code and could sign you out of it. When the token is expired we surface that
/// and wait for Claude Code to renew it on its own.
///
/// We also deliberately never set `CLAUDE_CODE_OAUTH_TOKEN` in any child
/// process — Claude Code deletes its Keychain entry on exit when that variable
/// is present.
enum ClaudeCredentialStore {
    /// The Keychain service name Claude Code writes under on macOS.
    private static let keychainService = "Claude Code-credentials"

    static func load() -> ClaudeCredentials? {
        for data in keychainCandidates() {
            if let credentials = parse(data) { return credentials }
        }
        return nil
    }

    /// Every Keychain item under our service, current user's first.
    ///
    /// Claude Code keys the item by macOS username. Asking for a single match
    /// and taking whatever came back first meant that a second item — another
    /// account, or a leftover from a previous login — could silently hand us
    /// someone else's quota. Order by account instead, and let the caller keep
    /// walking if the first blob doesn't parse.
    private static func keychainCandidates() -> [Data] {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecReturnData as String: true,
            kSecReturnAttributes as String: true,
            kSecMatchLimit as String: kSecMatchLimitAll
        ]

        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
              let items = result as? [[String: Any]] else {
            return []
        }

        // A partition rather than `sorted`: "is mine" is not a strict weak
        // ordering, and Swift's sort is undefined for predicates that aren't.
        let currentUser = NSUserName()
        let isCurrentUser = { (item: [String: Any]) in
            item[kSecAttrAccount as String] as? String == currentUser
        }
        let ordered = items.filter(isCurrentUser) + items.filter { !isCurrentUser($0) }
        return ordered.compactMap { $0[kSecValueData as String] as? Data }
    }

    /// The stored blob is `{"claudeAiOauth": {"accessToken": ..., "expiresAt": <ms>}}`.
    /// Older builds stored the inner object directly, so accept both shapes.
    private static func parse(_ data: Data) -> ClaudeCredentials? {
        guard let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return nil
        }
        let payload = (root["claudeAiOauth"] as? [String: Any]) ?? root
        guard let token = payload["accessToken"] as? String, !token.isEmpty else { return nil }

        var expiresAt: Date?
        if let milliseconds = payload["expiresAt"] as? Double {
            expiresAt = Date(timeIntervalSince1970: milliseconds / 1000)
        }

        return ClaudeCredentials(
            accessToken: token,
            expiresAt: expiresAt,
            subscriptionType: payload["subscriptionType"] as? String,
            rateLimitTier: payload["rateLimitTier"] as? String
        )
    }
}
