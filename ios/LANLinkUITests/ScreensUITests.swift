import XCTest

/// Walks the non-remote screens and keeps a screenshot of each, for the
/// side-by-side check against the Android screens in docs/media.
/// `-uiTestSeed` gives the app the fictional "Studio PC" in stores of its own.
final class ScreensUITests: XCTestCase {
    /// A fictional pairing link (made-up fingerprint and code).
    static let link =
        "lanlink://pair?v=1&a=192.168.1.20&p=47990"
        + "&f=9d41ae88f3bffe4028e25054dfc2f247ff6ebb405fd3ba8abadbebae0d9e405c"
        + "&c=48213907&n=Studio%20PC"

    override func setUp() {
        continueAfterFailure = false
    }

    @MainActor
    func testScreens() {
        let app = XCUIApplication()
        app.launchArguments = ["-uiTestSeed"]
        app.launch()

        // Home
        XCTAssertTrue(app.staticTexts["Studio PC"].firstMatch.waitForExistence(timeout: 30))
        XCTAssertTrue(app.buttons["Pair a PC"].firstMatch.exists)
        snap(app, "01-home")

        // A pairing link only offers. (The deployment target is iOS 16.0;
        // the CI simulator is newer.)
        // The link is opened through the system, as a tap in Safari would
        // (XCUIApplication.open relaunches the app instead).
        guard #available(iOS 16.4, *) else {
            return XCTFail("opening a URL in a UI test needs iOS 16.4 or newer")
        }
        XCUIDevice.shared.system.open(URL(string: Self.link)!)
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let confirm = springboard.buttons["Open"]
        if confirm.waitForExistence(timeout: 5) { confirm.tap() }
        XCTAssertTrue(app.staticTexts["Pair with Studio PC?"].waitForExistence(timeout: 15))
        snap(app, "02-offer")
        app.buttons["Cancel"].firstMatch.tap()
        XCTAssertTrue(app.buttons["Pair a PC"].firstMatch.waitForExistence(timeout: 10))

        // Pair, idle
        app.buttons["Pair a PC"].firstMatch.tap()
        XCTAssertTrue(app.buttons["Scan QR code"].firstMatch.waitForExistence(timeout: 10))
        snap(app, "03-pair")
        app.buttons["Back"].firstMatch.tap()

        // Settings, scrolled to Privacy
        XCTAssertTrue(app.buttons["Settings"].firstMatch.waitForExistence(timeout: 10))
        app.buttons["Settings"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["STREAM QUALITY"].waitForExistence(timeout: 10))
        scrollUntilHittable(app, app.staticTexts["PRIVACY"])
        snap(app, "04-settings")

        // Diagnostics
        let diagnostics = app.buttons["Performance diagnostics"].firstMatch
        scrollUntilHittable(app, diagnostics)
        diagnostics.tap()
        XCTAssertTrue(app.staticTexts["Connect to a PC to see live measurements."].waitForExistence(timeout: 10))
        snap(app, "05-diagnostics")
        app.buttons["Back"].firstMatch.tap()

        // Security
        let security = app.buttons["Security and paired PCs"].firstMatch
        scrollUntilHittable(app, security)
        security.tap()
        XCTAssertTrue(app.staticTexts["THIS PHONE'S IDENTITY"].waitForExistence(timeout: 10))
        snap(app, "06-security")
        app.terminate()

        // Onboarding: a launch without the seed uses the app's own (empty)
        // stores.
        let fresh = XCUIApplication()
        fresh.launch()
        XCTAssertTrue(fresh.staticTexts["Your PC, in your hand."].waitForExistence(timeout: 30))
        snap(fresh, "07-onboarding")
    }

    private func scrollUntilHittable(_ app: XCUIApplication, _ element: XCUIElement) {
        var swipes = 0
        while !(element.exists && element.isHittable) && swipes < 12 {
            app.swipeUp(velocity: .slow)
            swipes += 1
        }
        XCTAssertTrue(element.exists && element.isHittable, "could not scroll to \(element)")
    }

    private func snap(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
