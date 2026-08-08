import AppKit
import SwiftUI

struct SettingsView: View {
    @ObservedObject var model: UsageModel
    var onClose: () -> Void
    @State private var launchAtLogin = LaunchAtLogin.isEnabled

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Settings")
                .font(.system(size: 14, weight: .semibold))

            Picker("Menu bar shows", selection: $model.barDisplayMode) {
                ForEach(BarDisplayMode.allCases) { mode in
                    Text(mode.label).tag(mode)
                }
            }

            Picker("Check every", selection: $model.pollInterval) {
                ForEach(PollInterval.choices, id: \.self) { seconds in
                    Text(PollInterval.label(seconds)).tag(seconds)
                }
            }

            Toggle("Show percentage next to the ring", isOn: $model.showPercentageText)

            VStack(alignment: .leading, spacing: 4) {
                Toggle("Use the usage API when no session is running", isOn: $model.oauthFallbackEnabled)
                Text("Off: usage only updates while Claude Code is open, using the data it already hands the status line. On: also polls api.anthropic.com/api/oauth/usage with your Claude Code login. Neither consumes quota.")
                    .font(.system(size: 10))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if LaunchAtLogin.isAvailable {
                // A custom Binding rather than .onChange: registration can fail
                // (unsigned bundle, user restrictions), and this snaps the
                // toggle back to the truth instead of lying about the state.
                Toggle("Launch at login", isOn: Binding(
                    get: { launchAtLogin },
                    set: { newValue in
                        launchAtLogin = LaunchAtLogin.set(newValue)
                            ? newValue
                            : LaunchAtLogin.isEnabled
                    }
                ))
            }

            Divider()

            VStack(alignment: .leading, spacing: 3) {
                Text("Status line cache")
                    .font(.system(size: 11, weight: .medium))
                Text(StatusLineCache.fileURL.path)
                    .font(.system(size: 10).monospaced())
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .lineLimit(2)
                    .truncationMode(.middle)
            }

            HStack {
                Spacer()
                Button("Done", action: onClose)
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 380)
    }
}
