import AVFoundation
import CoreMedia
import XCTest
@testable import LANLink

// The decoder waits for a keyframe with parameter sets and asks for one
// (Review Focus 2), and turns a real H.264 keyframe into a format
// description it can display.

final class VideoDecoderTests: XCTestCase {
    func testDeltaFrameBeforeParameterSetsWaits() {
        var requests = 0
        let decoder = VideoDecoder(requestKeyframe: { requests += 1 })
        // A non-IDR slice (NAL type 1) with no SPS/PPS seen yet.
        let delta = Data([0, 0, 0, 1, 0x41, 0x9A, 0x02, 0x03])

        XCTAssertEqual(decoder.handle(frame: delta, keyframe: false, discontinuity: false), .waitingForKeyframe)
        XCTAssertEqual(requests, 1)
        XCTAssertNil(decoder.formatDescription)
    }

    func testKeyframeWithParameterSetsCreatesFormat() throws {
        // ios/LANLinkTests/Fixtures/key64.h264: one 64×64 IDR frame with its
        // SPS and PPS, from ffmpeg/libx264 (see the Task 10 report).
        let url = try XCTUnwrap(
            Bundle(for: VideoDecoderTests.self).url(forResource: "key64", withExtension: "h264"),
            "key64.h264 is not in the test bundle")
        let frame = try Data(contentsOf: url)
        var requests = 0
        let decoder = VideoDecoder(requestKeyframe: { requests += 1 })

        XCTAssertEqual(decoder.handle(frame: frame, keyframe: true, discontinuity: false), .displayed)
        let format = try XCTUnwrap(decoder.formatDescription)
        let size = CMVideoFormatDescriptionGetDimensions(format)
        XCTAssertEqual(size.width, 64)
        XCTAssertEqual(size.height, 64)
        XCTAssertEqual(requests, 0)
    }
}
