import Network
import XCTest
import dnssd
@testable import LANLink

// Tests for Discovery's pure helpers. Behaviour mirrors
// android/.../core/Discovery.kt (name cleaning, "PC" fallback).

final class DiscoveryTests: XCTestCase {
    func testTXTParsingCleansNames() {
        let r = Discovery.parseTXT(["n": "\u{202E}enohP", "id": "abcd", "v": "2"], serviceName: "svc")
        XCTAssertEqual(r.name, "enohP")
        XCTAssertEqual(r.shortId, "abcd")
        XCTAssertEqual(r.protocol, 2)
    }

    func testEmptyNameFallsBackToPC() {
        let r = Discovery.parseTXT(["n": "\u{200B}"], serviceName: "svc")
        XCTAssertEqual(r.name, "PC")
        XCTAssertEqual(r.shortId, "")
        XCTAssertEqual(r.protocol, 0)
    }

    func testDeniedErrorMapsToLocalNetworkDenied() {
        let err = NWError.dns(DNSServiceErrorType(kDNSServiceErr_PolicyDenied))
        XCTAssertEqual(Discovery.status(for: err), .localNetworkDenied)
    }
}
