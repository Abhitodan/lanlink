import XCTest
@testable import LANLink

// Annex B parsing for the video path. The bytes match the Rust
// `extracts_parameter_sets` test in crates/lanlink-mobile/src/video.rs.

final class AnnexBTests: XCTestCase {
    func testSplitsThreeAndFourByteStartCodes() {
        let au = Data([0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 9, 9])
        let nals = AnnexB.nalUnits(au)
        XCTAssertEqual(nals, [Data([0x67, 1, 2]), Data([0x68, 3]), Data([0x65, 9, 9])])

        let ps = AnnexB.parameterSets(nals)
        XCTAssertEqual(ps?.sps, Data([0x67, 1, 2]))
        XCTAssertEqual(ps?.pps, Data([0x68, 3]))
        XCTAssertNil(AnnexB.parameterSets([Data([0x65, 9, 9])]))
    }

    func testAvccLengthPrefixes() {
        XCTAssertEqual(AnnexB.avcc([Data([0x65, 9, 9])]), Data([0, 0, 0, 3, 0x65, 9, 9]))
    }

    func testTruncatedInputYieldsNoGarbage() {
        XCTAssertEqual(AnnexB.nalUnits(Data([0, 0, 1])), [])
        XCTAssertEqual(AnnexB.nalUnits(Data()), [])
    }
}
