import XCTest
@testable import LANLink

// Tests for Settings, HostRepository and IdentityStore. Defaults and clamps
// match android/.../core/Stores.kt.

final class StoresTests: XCTestCase {
    private func freshDefaults(_ name: String) -> UserDefaults {
        let d = UserDefaults(suiteName: name)!
        d.removePersistentDomain(forName: name)
        return d
    }

    func testSettingsDefaultsMatchAndroid() {
        let s = Settings(defaults: freshDefaults("test.settings.defaults"))
        XCTAssertTrue(s.compactKeyboard)
        XCTAssertEqual(s.keyHeight, 36)
        XCTAssertEqual(s.pointerMode, .trackpad)
        XCTAssertEqual(s.startZoom, 2)
        XCTAssertFalse(s.allowScreenshots)
        XCTAssertTrue(s.landscape)
        XCTAssertTrue(s.keepScreenOn)
    }

    func testKeyHeightIsClamped() {
        let s = Settings(defaults: freshDefaults("test.settings.clamp"))
        s.keyHeight = 100
        XCTAssertEqual(s.keyHeight, 48)
        s.keyHeight = 1
        XCTAssertEqual(s.keyHeight, 28)
    }

    func testHostRepositoryRoundTripAndTouchOrder() {
        let repo = HostRepository(defaults: freshDefaults("test.hosts"))
        let a = PairedHost(
            fingerprint: "aaaaaaaaaaaaaaaabbbbbbbb", name: "Sam's PC",
            addresses: ["192.168.1.50"], port: 47990, pairedAt: 1000, lastConnected: 1000)
        let b = PairedHost(
            fingerprint: "ccccccccccccccccdddddddd", name: "Sam's Laptop",
            addresses: ["192.168.1.60"], port: 47990, pairedAt: 2000, lastConnected: 2000)
        repo.put(a)
        repo.put(b)
        XCTAssertEqual(repo.get(a.fingerprint), a)
        XCTAssertEqual(repo.all().map { $0.fingerprint }, [b.fingerprint, a.fingerprint])
        XCTAssertEqual(a.shortId, "aaaaaaaaaaaaaaaa")

        repo.touch(fp: a.fingerprint, address: "192.168.1.51")
        let touched = repo.get(a.fingerprint)!
        XCTAssertEqual(touched.addresses, ["192.168.1.51", "192.168.1.50"])
        XCTAssertGreaterThan(touched.lastConnected, 2000)
        XCTAssertEqual(repo.all().map { $0.fingerprint }, [a.fingerprint, b.fingerprint])

        repo.remove(b.fingerprint)
        XCTAssertEqual(repo.all().count, 1)
        repo.clear()
        XCTAssertTrue(repo.all().isEmpty)
    }

    func testIdentityIsStableAndClearable() throws {
        let store = IdentityStore(service: "io.lanlink.identity.tests")
        store.clear()
        defer { store.clear() }
        let first = try store.get(deviceLabel: "Sam's iPhone")
        let second = try store.get(deviceLabel: "Sam's iPhone")
        XCTAssertFalse(first.isEmpty)
        XCTAssertEqual(first, second)
        XCTAssertFalse(try store.fingerprint(deviceLabel: "Sam's iPhone").isEmpty)
        store.clear()
        let third = try store.get(deviceLabel: "Sam's iPhone")
        XCTAssertNotEqual(first, third)
    }
}
