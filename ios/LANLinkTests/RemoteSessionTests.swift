import XCTest
@testable import LANLink

// The stream is requested once per connection (review finding I1): after a
// reconnect the kept display list must not stop the new SessionStart.
// Connection epochs keep a stale connect loop from publishing a second
// handle (final review I-1). Connect errors point to the Local Network
// permission when it is denied (I-2). Sticky modifiers do not survive
// releaseAll() or stop() (I-3).

final class RemoteSessionTests: XCTestCase {
    func testSecondConnectionRequestsAgain() {
        var gate = SessionRequestGate()

        gate.connectionStarted()
        XCTAssertTrue(gate.onDisplays(phase: .starting))
        // A later list on the same connection (display change) does not
        // start a second session.
        XCTAssertFalse(gate.onDisplays(phase: .starting))
        XCTAssertFalse(gate.onDisplays(phase: .streaming))

        // Reconnect: the display list is still known, the PC needs a new
        // SessionStart.
        gate.connectionStarted()
        XCTAssertFalse(gate.onDisplays(phase: .reconnecting))
        XCTAssertTrue(gate.onDisplays(phase: .starting))
    }

    // MARK: Connection epochs (I-1)

    /// Background while reconnecting, then foreground: stop() and retry()
    /// each begin an epoch. The old loop's connect then returns a handle,
    /// which must be closed, not published.
    func testStaleEpochHandleIsClosed() {
        var gate = ConnectGate()
        let first = gate.advance() // start()
        _ = gate.advance() // stop() on background
        let second = gate.advance() // retry() on foreground

        XCTAssertFalse(gate.isCurrent(first))
        XCTAssertEqual(gate.decide(epoch: first, stopped: false, handle: 0), .close)
        XCTAssertEqual(gate.decide(epoch: second, stopped: false, handle: 0), .accept)
    }

    /// A handle is already published: a second one is never accepted, even
    /// from the current epoch.
    func testSecondHandleIsClosed() {
        var gate = ConnectGate()
        let e = gate.advance()
        XCTAssertEqual(gate.decide(epoch: e, stopped: false, handle: 0), .accept)
        XCTAssertEqual(gate.decide(epoch: e, stopped: false, handle: 7), .close)
    }

    func testCurrentEpochIsAcceptedUnlessStopped() {
        var gate = ConnectGate()
        let e = gate.advance()
        XCTAssertTrue(gate.isCurrent(e))
        XCTAssertEqual(gate.decide(epoch: e, stopped: false, handle: 0), .accept)
        XCTAssertEqual(gate.decide(epoch: e, stopped: true, handle: 0), .close)
    }

    // MARK: Local Network denied (I-2)

    func testConnectErrorPointsToLocalNetworkWhenDenied() {
        let denied = RemoteSession.friendlyConnectError(
            "The PC did not answer.", hostName: "Studio PC", localNetworkDenied: true)
        XCTAssertTrue(denied.contains("Local Network"), denied)
        XCTAssertTrue(denied.contains("Settings › Privacy & Security › Local Network › LANLink"), denied)
        XCTAssertFalse(denied.contains("Firewall"), denied)

        // Allowed: the Android wording is kept.
        let allowed = RemoteSession.friendlyConnectError(
            "The PC did not answer.", hostName: "Studio PC", localNetworkDenied: false)
        XCTAssertEqual(
            allowed,
            "Cannot reach Studio PC. Check that both devices are on the same Wi-Fi, that LANLink is running "
                + "on the PC, and that Windows Firewall allows it.")

        // An answer from the PC is shown as such, denied or not.
        XCTAssertEqual(
            RemoteSession.friendlyConnectError("not paired", hostName: "Studio PC", localNetworkDenied: true),
            "The PC no longer trusts this phone. Pair again from the PC.")
    }

    // MARK: Sticky modifiers (I-3)

    func testReleaseAllAndStopClearStickyModifiers() {
        let session = Self.makeSession("sticky")

        session.toggleSticky(KeyMap.VK_LCONTROL)
        session.toggleSticky(KeyMap.VK_LWIN)
        XCTAssertEqual(session.stickyModifiers, [KeyMap.VK_LCONTROL, KeyMap.VK_LWIN])
        session.releaseAll()
        XCTAssertEqual(session.stickyModifiers, [])

        session.toggleSticky(KeyMap.VK_LMENU)
        XCTAssertEqual(session.stickyModifiers, [KeyMap.VK_LMENU])
        session.stop()
        XCTAssertEqual(session.stickyModifiers, [])

        // The published state (the lit keys) is cleared too.
        let published = expectation(description: "state published on main")
        DispatchQueue.main.async { published.fulfill() }
        wait(for: [published], timeout: 2)
        XCTAssertEqual(session.state.modifiers, [])
        XCTAssertEqual(session.state.phase, .ended)
    }

    /// A session that is never started: nothing reaches the network.
    private static func makeSession(_ name: String) -> RemoteSession {
        func defaults(_ n: String) -> UserDefaults {
            let d = UserDefaults(suiteName: n)!
            d.removePersistentDomain(forName: n)
            return d
        }
        let host = PairedHost(
            fingerprint: "5a3c9e17b2d84f60a1c7e9b35d2f8046c1e7a93b5d28f4061ac3e7b95d1f2a84",
            name: "Studio PC", addresses: ["192.168.1.20"], port: 47990, pairedAt: 1, lastConnected: 0)
        return RemoteSession(
            host: host, preferredAddress: nil,
            settings: Settings(defaults: defaults("test.session.settings.\(name)")),
            hosts: HostRepository(defaults: defaults("test.session.hosts.\(name)")),
            identity: IdentityStore(service: "test.session.identity"))
    }
}
