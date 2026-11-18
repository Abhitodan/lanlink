import XCTest
@testable import LANLink

// Tests for AppModel, the Swift twin of android/.../ui/MainViewModel.kt:
// pairing links only offer (fix M2), friendly pairing errors, and leaving
// the remote screen ends the session.

final class AppModelTests: XCTestCase {
    /// Fictional PC; the fingerprint is made up for these tests.
    static let validLink =
        "lanlink://pair?v=1&a=192.168.1.20&p=47990"
        + "&f=9d41ae88f3bffe4028e25054dfc2f247ff6ebb405fd3ba8abadbebae0d9e405c"
        + "&c=48213907&n=Studio%20PC"

    private func freshDefaults(_ name: String) -> UserDefaults {
        let d = UserDefaults(suiteName: name)!
        d.removePersistentDomain(forName: name)
        return d
    }

    @MainActor
    private func makeModel(
        _ name: String,
        makeSession: @escaping (PairedHost, String?) -> SessionControl = { _, _ in PlaceholderSession() }
    ) -> (AppModel, Settings) {
        let settings = Settings(defaults: freshDefaults("test.appmodel.settings.\(name)"))
        let model = AppModel(
            settings: settings,
            repository: HostRepository(defaults: freshDefaults("test.appmodel.hosts.\(name)")),
            identity: IdentityStore(service: "test.appmodel.identity"),
            makeSession: makeSession)
        return (model, settings)
    }

    @MainActor
    func testLinkOnlyOffersAndDoesNotPersistOnboarding() {
        let (model, settings) = makeModel("link")
        XCTAssertEqual(model.screen, .onboarding)

        model.offerPairFromLink(Self.validLink)
        guard case let .offer(name, addresses, port, fingerprint, code) = model.pair else {
            return XCTFail("expected an offer, got \(model.pair)")
        }
        XCTAssertEqual(name, "Studio PC")
        XCTAssertEqual(addresses, ["192.168.1.20"])
        XCTAssertEqual(port, 47990)
        XCTAssertEqual(fingerprint, "9d41ae88f3bffe4028e25054dfc2f247ff6ebb405fd3ba8abadbebae0d9e405c")
        XCTAssertEqual(code, "48213907")
        // A link alone neither completes onboarding nor connects.
        XCTAssertFalse(settings.onboarded)

        model.acceptOffer()
        XCTAssertTrue(settings.onboarded)
        // Stop the pairing attempt before it reaches the network.
        model.cancelPairing()
        XCTAssertEqual(model.pair, .idle)
    }

    @MainActor
    func testPublicAddressLinkFails() {
        let (model, settings) = makeModel("public")
        let link = Self.validLink.replacingOccurrences(of: "a=192.168.1.20", with: "a=8.8.8.8")
        model.offerPairFromLink(link)
        guard case let .failed(message) = model.pair else {
            return XCTFail("expected a failure, got \(model.pair)")
        }
        XCTAssertTrue(message.contains("outside your local network"), message)
        XCTAssertFalse(settings.onboarded)
    }

    func testFriendlyErrors() {
        XCTAssertEqual(
            AppModel.friendlyPairError("the pairing code is wrong"),
            "The code was not accepted. Check the 8-digit code on the PC and try again.")
        XCTAssertEqual(
            AppModel.friendlyPairError("the PC already knows this phone (unverified)"),
            "That PC says it already knows this phone, but without its QR code LANLink cannot check "
                + "that it is your PC. Scan the QR code on the PC instead.")
        XCTAssertEqual(
            AppModel.friendlyPairError("the PC did not answer"),
            "Cannot reach the PC. Make sure both are on the same Wi-Fi and Windows Firewall allows LANLink.")
        XCTAssertEqual(AppModel.friendlyPairError("something else"), "something else")
    }

    @MainActor
    func testBackFromRemoteEndsSession() {
        let recorder = RecordingSession()
        var made: [(PairedHost, String?)] = []
        let (model, settings) = makeModel("remote") { host, address in
            made.append((host, address))
            return recorder
        }
        settings.onboarded = true
        let host = PairedHost(
            fingerprint: "5a3c9e17b2d84f60a1c7e9b35d2f8046c1e7a93b5d28f4061ac3e7b95d1f2a84",
            name: "Studio PC", addresses: ["192.168.1.20"], port: 47990, pairedAt: 1, lastConnected: 0)

        model.connect(host, address: "192.168.1.21")
        XCTAssertEqual(model.screen, .remote)
        XCTAssertEqual(made.count, 1)
        XCTAssertEqual(made.first?.0, host)
        XCTAssertEqual(made.first?.1, "192.168.1.21")
        XCTAssertEqual(recorder.calls, ["start"])
        XCTAssertNotNil(model.session)

        XCTAssertTrue(model.back())
        XCTAssertEqual(recorder.calls, ["start", "stop"])
        XCTAssertNil(model.session)
        XCTAssertEqual(model.screen, .home)
    }
}

/// Records what the model asks of its session.
private final class RecordingSession: SessionControl {
    var calls: [String] = []
    func start() { calls.append("start") }
    func stop() { calls.append("stop") }
}
