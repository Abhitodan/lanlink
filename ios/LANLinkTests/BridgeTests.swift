import XCTest
@testable import LANLink

final class BridgeTests: XCTestCase {
    func testIdentityRoundTrip() throws {
        let identity = try Bridge.generateIdentity(label: "LANLink test")
        XCTAssertFalse(identity.isEmpty)

        let fingerprint = try Bridge.identityFingerprint(identity)
        XCTAssertEqual(fingerprint.count, 64)
        XCTAssertTrue(fingerprint.allSatisfy { $0.isHexDigit }, fingerprint)
    }

    func testParsePairingUriRejectsPublicAddress() {
        let fp = "5a3c9e17b2d84f60a1c7e9b35d2f8046c1e7a93b5d28f4061ac3e7b95d1f2a84"
        let uri = "lanlink://pair?v=1&a=8.8.8.8&p=47990&f=\(fp)&c=12345678"
        XCTAssertThrowsError(try Bridge.parsePairingUri(uri)) { error in
            guard let e = error as? LANLinkError else {
                return XCTFail("expected LANLinkError, got \(error)")
            }
            XCTAssertTrue(e.message.contains("outside your local network"), e.message)
        }
    }

    func testStaleHandleThrows() {
        XCTAssertThrowsError(try Bridge.sendKey(handle: 999_999, vk: 0x41, down: true)) { error in
            XCTAssertTrue(error is LANLinkError, "expected LANLinkError, got \(error)")
        }
    }
}
