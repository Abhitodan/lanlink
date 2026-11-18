import Security
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
        // A second instance has no cache: this reads through the Keychain.
        let other = IdentityStore(service: "io.lanlink.identity.tests")
        XCTAssertEqual(try other.get(deviceLabel: "Sam's iPhone"), first)
        XCTAssertFalse(try store.fingerprint(deviceLabel: "Sam's iPhone").isEmpty)
        store.clear()
        let third = try store.get(deviceLabel: "Sam's iPhone")
        XCTAssertNotEqual(first, third)
    }

    func testPairedHostJsonKeys() throws {
        let h = PairedHost(
            fingerprint: "aa", name: "Sam's PC", addresses: ["192.168.1.50"],
            port: 47990, pairedAt: 1, lastConnected: 2)
        let data = try JSONEncoder().encode(h)
        let obj = try JSONSerialization.jsonObject(with: data) as! [String: Any]
        XCTAssertEqual(Set(obj.keys), ["fp", "name", "addrs", "port", "pairedAt", "last"])
        XCTAssertEqual(PairedHost.listFromJson(PairedHost.listToJson([h])), [h])
    }

    func testTouchKeepsSixAddressesNewestFirst() {
        let repo = HostRepository(defaults: freshDefaults("test.hosts.cap"))
        repo.put(PairedHost(
            fingerprint: "aa", name: "Sam's PC", addresses: [], port: 47990,
            pairedAt: 1, lastConnected: 1))
        for n in 1...8 { repo.touch(fp: "aa", address: "192.168.1.\(n)") }
        XCTAssertEqual(
            repo.get("aa")!.addresses,
            ["192.168.1.8", "192.168.1.7", "192.168.1.6", "192.168.1.5", "192.168.1.4", "192.168.1.3"])
    }

    func testSettingsStorageKeysAndClamps() {
        let name = "test.settings.keys"
        let d = freshDefaults(name)
        let s = Settings(defaults: d)
        s.allowScreenshots = true
        XCTAssertTrue(d.bool(forKey: "allowShots"))
        s.startZoom = 10
        XCTAssertEqual(s.startZoom, 4)
        s.startZoom = 0
        XCTAssertEqual(s.startZoom, 1)
        s.deviceName = String(repeating: "x", count: 60)
        XCTAssertEqual(s.deviceName.count, 48)
    }

    func testKeychainAttributes() {
        let store = IdentityStore(service: "io.lanlink.identity.tests")
        let add = store.addAttributes(for: Data([1, 2, 3]))
        XCTAssertEqual(add[kSecAttrService as String] as? String, "io.lanlink.identity.tests")
        XCTAssertEqual(add[kSecAttrAccount as String] as? String, "identity")
        XCTAssertEqual(add[kSecAttrSynchronizable as String] as? Bool, false)
        XCTAssertEqual(
            add[kSecAttrAccessible as String] as? String,
            kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String)
        XCTAssertEqual(IdentityStore.defaultService, "io.lanlink.identity")
    }
}
