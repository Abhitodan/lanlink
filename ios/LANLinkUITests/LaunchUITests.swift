import XCTest

/// Placeholder until the screen walk-through lands: the app launches and
/// shows its first page.
final class LaunchUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    @MainActor
    func testLaunchShowsFirstPage() {
        let app = XCUIApplication()
        app.launch()
        XCTAssertTrue(app.staticTexts["LANLink"].firstMatch.waitForExistence(timeout: 30))

        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = "00-launch"
        shot.lifetime = .keepAlways
        add(shot)
    }
}
