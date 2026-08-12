import Foundation

/// Reads the cache file written by `statusline/claude-quota-statusline.py`.
///
/// Claude Code hands `rate_limits` to the status line command on stdin with no
/// network call of its own, so this path is free, instant, and refreshes on
/// every render while you're working. It is the app's primary source; the OAuth
/// poll only covers the gap when no session is running.
enum StatusLineCache {
    static var fileURL: URL {
        if let override = ProcessInfo.processInfo.environment["CLAUDE_QUOTA_CACHE"], !override.isEmpty {
            return URL(fileURLWithPath: (override as NSString).expandingTildeInPath)
        }
        return FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".claude/quota-bar-cache.json")
    }

    /// Extra detail the status line can tell us that the API can't.
    struct Context: Equatable {
        var model: String?
        var sessionID: String?
    }

    static func read() -> (snapshot: UsageSnapshot, context: Context)? {
        guard let data = try? Data(contentsOf: fileURL),
              let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return nil
        }

        let keys: [(UsageWindowKind, String)] = [
            (.fiveHour, "five_hour"),
            (.sevenDay, "seven_day")
        ]

        let windows: [UsageEntry] = keys.compactMap { kind, key in
            guard let object = root[key] as? [String: Any],
                  let used = OAuthUsageClient.decodeNumber(object["used_percentage"]) else {
                return nil
            }
            return UsageEntry(
                kind: kind,
                window: UsageWindow(
                    usedPercentage: used,
                    resetsAt: OAuthUsageClient.decodeDate(object["resets_at"])
                )
            )
        }

        guard !windows.isEmpty else { return nil }

        // Fall back to the file's own mtime if the writer didn't stamp it.
        let capturedAt = OAuthUsageClient.decodeDate(root["captured_at"])
            ?? (try? fileURL.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate)
            ?? Date()

        let snapshot = UsageSnapshot(
            windows: windows,
            extraUsageEnabled: nil,
            capturedAt: capturedAt,
            source: .statusLine
        )
        let context = Context(
            model: root["model"] as? String,
            sessionID: root["session_id"] as? String
        )
        return (snapshot, context)
    }
}

/// Watches the cache file so an active Claude Code session updates the menu bar
/// immediately, instead of waiting for the next poll tick.
final class CacheFileWatcher {
    private var source: DispatchSourceFileSystemObject?
    private var descriptor: CInt = -1
    private var directorySource: DispatchSourceFileSystemObject?
    private var directoryDescriptor: CInt = -1
    private let onChange: () -> Void

    init(onChange: @escaping () -> Void) {
        self.onChange = onChange
    }

    deinit { stop() }

    func start() {
        stop()
        watchDirectory()
        watchFile()
    }

    /// The status line writes atomically (write temp, rename), which replaces the
    /// inode — so we watch the containing directory too and re-arm the file
    /// watch whenever a new file lands.
    private func watchDirectory() {
        let directory = StatusLineCache.fileURL.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        directoryDescriptor = open(directory.path, O_EVTONLY)
        guard directoryDescriptor >= 0 else { return }

        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: directoryDescriptor,
            eventMask: [.write],
            queue: .main
        )
        source.setEventHandler { [weak self] in
            self?.watchFile()
            self?.onChange()
        }
        source.setCancelHandler { [weak self] in
            guard let self, self.directoryDescriptor >= 0 else { return }
            close(self.directoryDescriptor)
            self.directoryDescriptor = -1
        }
        source.resume()
        directorySource = source
    }

    private func watchFile() {
        source?.cancel()
        source = nil

        descriptor = open(StatusLineCache.fileURL.path, O_EVTONLY)
        guard descriptor >= 0 else { return }

        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .extend, .rename, .delete],
            queue: .main
        )
        source.setEventHandler { [weak self] in self?.onChange() }
        source.setCancelHandler { [weak self] in
            guard let self, self.descriptor >= 0 else { return }
            close(self.descriptor)
            self.descriptor = -1
        }
        source.resume()
        self.source = source
    }

    func stop() {
        source?.cancel()
        source = nil
        directorySource?.cancel()
        directorySource = nil
    }
}
