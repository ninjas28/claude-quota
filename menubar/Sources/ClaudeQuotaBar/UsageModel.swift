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
    @Published var menuBarStyle: MenuBarStyle {
        didSet { defaults.set(menuBarStyle.rawValue, forKey: SettingsKey.menuBarStyle) }
    }
    @Published var colorTheme: ColorTheme {
        didSet { defaults.set(colorTheme.rawValue, forKey: SettingsKey.colorTheme) }
    }

    private let defaults: UserDefaults

    // Main-actor isolated, like the rest of the class. A `nonisolated` init
    // can't work here: every setting below has a `didSet`, which makes the
    // compiler treat the assignment as a mutation of isolated state rather
    // than an initialization. `shared` is a static on a @MainActor type and
    // every caller is already on the main actor, so there is no hop to avoid.
    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        defaults.register(defaults: [
            SettingsKey.pollInterval: PollInterval.default,
            SettingsKey.barDisplayMode: BarDisplayMode.session.rawValue,
            SettingsKey.oauthFallbackEnabled: true,
            SettingsKey.showPercentageText: true,
            SettingsKey.menuBarStyle: MenuBarStyle.ring.rawValue,
            SettingsKey.colorTheme: ColorTheme.claude.rawValue
        ])
        pollInterval = defaults.integer(forKey: SettingsKey.pollInterval)
        barDisplayMode = BarDisplayMode(rawValue: defaults.string(forKey: SettingsKey.barDisplayMode) ?? "") ?? .session
        oauthFallbackEnabled = defaults.bool(forKey: SettingsKey.oauthFallbackEnabled)
        showPercentageText = defaults.bool(forKey: SettingsKey.showPercentageText)
        menuBarStyle = MenuBarStyle(rawValue: defaults.string(forKey: SettingsKey.menuBarStyle) ?? "") ?? .ring
        colorTheme = ColorTheme(rawValue: defaults.string(forKey: SettingsKey.colorTheme) ?? "") ?? .claude
    }

    static let shared = UsageModel()

    /// A model holding fixed data, for SwiftUI previews and for rendering the
    /// README images offscreen. Never used by the running app.
    ///
    /// Settings go to a throwaway `UserDefaults` suite so rendering a preview
    /// can't overwrite what the user actually chose, and the OAuth fallback is
    /// forced off so a render can't turn into a network call.
    static func preview(
        snapshot: UsageSnapshot,
        style: MenuBarStyle = .ring,
        theme: ColorTheme = .claude
    ) -> UsageModel {
        let suite = UserDefaults(suiteName: "ClaudeQuotaBar.preview") ?? .standard
        suite.removePersistentDomain(forName: "ClaudeQuotaBar.preview")
        let model = UsageModel(defaults: suite)
        model.oauthFallbackEnabled = false
        model.menuBarStyle = style
        model.colorTheme = theme
        model.snapshot = snapshot
        return model
    }

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
    ///
    /// The exponential starts at `backoffBase` and doubles per consecutive 429,
    /// so the first strike costs a minute rather than the whole ceiling. A
    /// `Retry-After` further out always wins — the server knows when the window
    /// actually reopens — but one closer in can't undercut the exponential,
    /// which is the only thing damping a server that keeps saying "try again
    /// in 5 seconds" and then 429s again.
    nonisolated static func backoffDeadline(
        consecutiveRateLimits: Int,
        retryAt: Date?,
        now: Date = Date()
    ) -> Date {
        let exponent = Double(max(0, consecutiveRateLimits - 1))
        let interval = min(Defaults.maxBackoff, Defaults.backoffBase * pow(2, exponent))
        let floorDate = now.addingTimeInterval(interval)
        return max(retryAt ?? floorDate, floorDate)
    }

    private func applyBackoff(retryAt: Date?) {
        consecutiveRateLimits += 1
        backoffUntil = Self.backoffDeadline(
            consecutiveRateLimits: consecutiveRateLimits,
            retryAt: retryAt
        )
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
        var adopted = new
        // The plan badge only ever arrives from the OAuth path. Once we know
        // it, keep it — otherwise it would blink out of the header on the next
        // status line write and back in on the next poll.
        if adopted.plan == nil { adopted.plan = snapshot?.plan }
        snapshot = adopted
        if let newContext { context = newContext }
    }
}
