import Foundation
import Security

/// The OAuth credential Claude Code stores after you sign in.
struct ClaudeCredentials {
    var accessToken: String
    var expiresAt: Date?
    var subscriptionType: String?

    /// Treat a token as unusable slightly before its stated expiry so we don't
    /// fire a request that is guaranteed to 401 mid-flight.
    var isExpired: Bool {
        guard let expiresAt else { return false }
        return expiresAt.timeIntervalSinceNow < 30
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

    /// Linux/Keychain-less fallback, and the path Claude Code uses when the
    /// Keychain is unavailable.
    private static var credentialsFileURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".claude/.credentials.json")
    }

    static func load() -> ClaudeCredentials? {
        if let data = readKeychain(), let credentials = parse(data) {
            return credentials
        }
        if let data = try? Data(contentsOf: credentialsFileURL), let credentials = parse(data) {
            return credentials
        }
        return nil
    }

    private static func readKeychain() -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: keychainService,
            kSecReturnData as String: true,
            // Claude Code keys the item by macOS username; match on service
            // alone and take the first hit so we don't depend on that detail.
            kSecMatchLimit as String: kSecMatchLimitOne
        ]

        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        guard status == errSecSuccess else { return nil }
        return result as? Data
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
            subscriptionType: payload["subscriptionType"] as? String
        )
    }
}
