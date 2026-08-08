import Foundation
import SwiftUI

/// Owns refresh policy and publishes the state the UI renders.
///
/// Policy, in order:
///  1. Read the status line cache. If it's fresh, use it and make no network call.
///  2. Otherwise, if the OAuth fallback is on, poll the usage endpoint.
///  3. If that fails, keep showing the last good snapshot and label it stale.
@MainActor
final class UsageModel: ObservableObject {
    @Published private(set) var snapshot: UsageSnapshot?
    @Published private(set) var context: StatusLineCache.Context?
    @Published private(set) var lastError: UsageError?
    @Published private(set) var isRefreshing = false

    // Settings are persisted by hand rather than with @AppStorage: that wrapper
    // is built for View bodies and does not drive objectWillChange from here.
    @Published var pollInterval: Int {
        didSet {
            defaults.set(pollInterval, forKey: SettingsKey.pollInterval)
            restartTimer()
        }
    }
    @Published var barDisplayMode: BarDisplayMode {
        didSet { defaults.set(barDisplayMode.rawValue, forKey: SettingsKey.barDisplayMode) }
    }
    @Published var oauthFallbackEnabled: Bool {
        didSet {
            defaults.set(oauthFallbackEnabled, forKey: SettingsKey.oauthFallbackEnabled)
            if oauthFallbackEnabled { refreshNow() }
        }
    }
    @Published var showPercentageText: Bool {
        didSet { defaults.set(showPercentageText, forKey: SettingsKey.showPercentageText) }
    }

    private let defaults: UserDefaults

    // `nonisolated` so `shared` can be built from a static initializer without
    // hopping to the main actor. Safe: it only assigns stored properties, and
    // `didSet` observers don't fire during init.
    nonisolated init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        defaults.register(defaults: [
            SettingsKey.pollInterval: PollInterval.default,
            SettingsKey.barDisplayMode: BarDisplayMode.session.rawValue,
            SettingsKey.oauthFallbackEnabled: true,
            SettingsKey.showPercentageText: true
        ])
        pollInterval = defaults.integer(forKey: SettingsKey.pollInterval)
        barDisplayMode = BarDisplayMode(rawValue: defaults.string(forKey: SettingsKey.barDisplayMode) ?? "") ?? .session
        oauthFallbackEnabled = defaults.bool(forKey: SettingsKey.oauthFallbackEnabled)
        showPercentageText = defaults.bool(forKey: SettingsKey.showPercentageText)
    }

    static let shared = UsageModel()

    private let client = OAuthUsageClient()
    private var timer: Timer?
    private var watcher: CacheFileWatcher?
    private var refreshTask: Task<Void, Never>?

    /// Set after a 429; suppresses polling until it passes.
    private var backoffUntil: Date?
    private var consecutiveRateLimits = 0

    func start() {
        // The watcher fires on a plain dispatch queue, so hop to the main actor
        // before touching published state.
        watcher = CacheFileWatcher { [weak self] in
            Task { @MainActor in self?.loadCacheIfNewer() }
        }
        watcher?.start()

        restartTimer()
        refresh()
    }

    func stop() {
        timer?.invalidate()
        timer = nil
        watcher?.stop()
        watcher = nil
        refreshTask?.cancel()
    }

    private func restartTimer() {
        timer?.invalidate()
        let interval = TimeInterval(max(60, pollInterval))
        timer = Timer.scheduledTimer(withTimeInterval: interval, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    /// A manual refresh from the popover clears any backoff — the user asked.
    func refreshNow() {
        backoffUntil = nil
        consecutiveRateLimits = 0
        refresh(force: true)
    }

    func refresh(force: Bool = false) {
        refreshTask?.cancel()
        refreshTask = Task { await performRefresh(force: force) }
    }

    private func performRefresh(force: Bool) async {
        // 1. Local cache first — free, and current whenever a session is open.
        let cached = StatusLineCache.read()
        if let cached {
            adopt(cached.snapshot, context: cached.context)
            if cached.snapshot.age < Defaults.cacheStaleAfter && !force {
                lastError = nil
                return
            }
        }

        guard oauthFallbackEnabled else {
            lastError = cached == nil ? .noData : nil
            return
        }

        // Opening the popover calls refresh() from onAppear, so without this
        // a handful of clicks would each become a request to an endpoint that
        // rate limits hard. Anything we already hold this recent is good enough.
        if !force, let snapshot, snapshot.age < Defaults.cacheStaleAfter {
            lastError = nil
            return
        }

        if let backoffUntil, Date() < backoffUntil, !force {
            lastError = .rateLimited(retryAt: backoffUntil)
            return
        }

        isRefreshing = true
        defer { isRefreshing = false }

        do {
            let fresh = try await client.fetch()
            guard !Task.isCancelled else { return }
            adopt(fresh, context: context)
            lastError = nil
            backoffUntil = nil
            consecutiveRateLimits = 0
        } catch let failure as OAuthUsageClient.Failure {
            guard !Task.isCancelled else { return }
            switch failure {
            case .credentials(let error):
                lastError = error
            case .transport(let error):
                if case .rateLimited(let retryAt) = error {
                    applyBackoff(retryAt: retryAt)
                    lastError = .rateLimited(retryAt: backoffUntil)
                } else {
                    lastError = error
                }
            }
        } catch {
            guard !Task.isCancelled else { return }
            lastError = .network(error.localizedDescription)
        }
    }

    /// Exponential backoff, honouring `Retry-After` when the server sent one.
    private func applyBackoff(retryAt: Date?) {
        consecutiveRateLimits += 1
        let exponential = min(
            Defaults.maxBackoff,
            TimeInterval(pollInterval) * pow(2, Double(consecutiveRateLimits))
        )
        let candidate = Date().addingTimeInterval(exponential)
        backoffUntil = max(retryAt ?? candidate, candidate)
    }

    /// File-watch callback. Only adopt a cache write that is actually newer than
    /// what we're showing, so a stale file can't clobber a fresh API result.
    private func loadCacheIfNewer() {
        guard let cached = StatusLineCache.read() else { return }
        if let current = snapshot, cached.snapshot.capturedAt <= current.capturedAt { return }
        adopt(cached.snapshot, context: cached.context)
        lastError = nil
    }

    private func adopt(_ new: UsageSnapshot, context newContext: StatusLineCache.Context?) {
        if let current = snapshot, new.capturedAt < current.capturedAt { return }
        snapshot = new
        if let newContext { context = newContext }
    }
}
