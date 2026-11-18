import SwiftUI

@main
struct LANLinkApp: App {
    @StateObject private var model: AppModel

    init() {
        let m = ProcessInfo.processInfo.arguments.contains(UITestSeed.argument)
            ? UITestSeed.makeModel()
            : AppModel()
        _model = StateObject(wrappedValue: m)
    }

    var body: some Scene {
        WindowGroup {
            AppRoot(model: model)
                .onOpenURL { url in
                    // A link can come from any web page or app, so it only
                    // proposes pairing; nothing connects until the user
                    // confirms on screen. Onboarding is completed only if
                    // the user accepts (acceptOffer). iOS delivers each
                    // open once.
                    guard url.scheme?.lowercased() == "lanlink", url.host?.lowercased() == "pair" else { return }
                    model.go(.pair())
                    model.offerPairFromLink(url.absoluteString)
                }
        }
    }
}

/// UI tests only (`-uiTestSeed`): the app runs on stores of its own,
/// emptied at each launch and holding just the fictional "Studio PC", with
/// onboarding done. The app's real stores are not touched.
@MainActor
enum UITestSeed {
    static let argument = "-uiTestSeed"

    static func makeModel() -> AppModel {
        let settings = Settings(defaults: fresh("uitest.settings"))
        let repository = HostRepository(defaults: fresh("uitest.hosts"))
        settings.onboarded = true
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let day: Int64 = 24 * 3600 * 1000
        repository.put(PairedHost(
            fingerprint: "5a3c9e17b2d84f60a1c7e9b35d2f8046c1e7a93b5d28f4061ac3e7b95d1f2a84",
            name: "Studio PC",
            addresses: ["192.168.1.20"],
            port: 47990,
            pairedAt: now - 3 * day,
            lastConnected: now - day))
        return AppModel(
            settings: settings,
            repository: repository,
            identity: IdentityStore(service: IdentityStore.defaultService + ".uitest"))
    }

    private static func fresh(_ suite: String) -> UserDefaults {
        let d = UserDefaults(suiteName: suite) ?? .standard
        d.removePersistentDomain(forName: suite)
        return d
    }
}
