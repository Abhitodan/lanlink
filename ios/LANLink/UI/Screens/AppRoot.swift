import SwiftUI

// Twin of android/.../ui/screens/AppRoot.kt: one screen at a time from
// `AppModel.screen`, cross-fading only when the kind of screen changes.
// Back is the page's back button (the model keeps the back stack).

struct AppRoot: View {
    @ObservedObject var model: AppModel

    var body: some View {
        NavigationStack {
            ZStack {
                screen
                    .id(kind(model.screen))
                    .transition(.opacity)
            }
            .animation(.easeInOut(duration: 0.2), value: kind(model.screen))
        }
        .tint(Theme.primary)
    }

    @ViewBuilder
    private var screen: some View {
        switch model.screen {
        case .onboarding:
            OnboardingScreen(model: model)
        case .home:
            HomeScreen(model: model)
        case let .pair(prefillAddress, prefillPort):
            PairScreen(model: model, prefillAddress: prefillAddress, prefillPort: prefillPort)
        case let .host(fingerprint):
            HostScreen(model: model, fingerprint: fingerprint)
        case .remote:
            // RemoteScreen arrives with Task 11.
            Page(title: "", onBack: { model.back() }) {}
        case .settings:
            SettingsScreen(model: model)
        case .diagnostics:
            LiveDiagnosticsScreen(model: model)
        case .security:
            SecurityScreen(model: model)
        }
    }

    /// The screen's kind, like Compose's `contentKey = { it::class }`.
    private func kind(_ s: Screen) -> Int {
        switch s {
        case .onboarding: return 0
        case .home: return 1
        case .pair: return 2
        case .host: return 3
        case .remote: return 4
        case .settings: return 5
        case .diagnostics: return 6
        case .security: return 7
        }
    }
}
