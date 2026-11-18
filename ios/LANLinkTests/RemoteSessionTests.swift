import XCTest
@testable import LANLink

// The stream is requested once per connection (review finding I1): after a
// reconnect the kept display list must not stop the new SessionStart.

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
}
