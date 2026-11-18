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

    func testNoAuthMapsToLocalNetworkDenied() {
        let err = NWError.dns(DNSServiceErrorType(kDNSServiceErr_NoAuth))
        XCTAssertEqual(Discovery.status(for: err), .localNetworkDenied)
    }

    func testOtherErrorIsFailedNotDenied() {
        let err = NWError.posix(.ECONNREFUSED)
        if case .failed = Discovery.status(for: err) {} else { XCTFail("expected .failed") }
    }

    func testAddressStringKeepsIPv4AndGlobalV6() {
        XCTAssertEqual(Discovery.addressString(.ipv4(IPv4Address("192.168.1.5")!)), "192.168.1.5")
        XCTAssertEqual(Discovery.addressString(.ipv4(IPv4Address("169.254.3.4")!)), "169.254.3.4")
        XCTAssertEqual(Discovery.addressString(.ipv6(IPv6Address("2001:db8::1")!)), "2001:db8::1")
    }

    func testAddressStringRejectsLinkLocalV6AndHostnames() {
        XCTAssertNil(Discovery.addressString(.ipv6(IPv6Address("fe80::1")!)))
        XCTAssertNil(Discovery.addressString(.name("pc.local", nil)))
    }

    func testLiveNamesFromSnapshotDedupesInterfacesAndIgnoresNonServices() {
        let a = NWEndpoint.service(name: "pc-a", type: "_lanlink._udp", domain: "local", interface: nil)
        let b = NWEndpoint.service(name: "pc-b", type: "_lanlink._udp", domain: "local", interface: nil)
        let other = NWEndpoint.hostPort(host: "192.168.1.5", port: 80)
        XCTAssertEqual(Discovery.liveNames(endpoints: [a, a, b, other]), ["pc-a", "pc-b"])
        XCTAssertEqual(Discovery.liveNames(endpoints: []), [])
    }
}
