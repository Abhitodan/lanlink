import XCTest

/// Walks the screens and keeps a screenshot of each, for the
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

        // The pointer-mode chips and the sliders under them. An iPhone 16
        // screen cannot hold both these and the key-size slider, so the
        // controls take two screenshots.
        dragToTop(app, app.staticTexts["TOUCH AND POINTER"])
        XCTAssertTrue(app.buttons["Trackpad"].firstMatch.isHittable)
        XCTAssertTrue(app.sliders.firstMatch.isHittable)
        snap(app, "09b-settings-pointer")
        // Framed like docs/media/phone-settings.png: Display, Keyboard and
        // the key-size slider.
        dragToTop(app, app.staticTexts["DISPLAY"])
        XCTAssertTrue(app.staticTexts["Key size: 36 dp"].isHittable)
        snap(app, "09-settings-controls")

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

    /// The remote screen with LANLink's compact keyboard open, on a still
    /// test frame (`-uiTestRemoteDemo`: no network). The demo stays upright:
    /// with the interface turned sideways by the app on a simulator held
    /// upright, XCUITest returns a rotated, cropped screenshot.
    @MainActor
    func testRemoteKeyboard() {
        let app = XCUIApplication()
        app.launchArguments = ["-uiTestSeed", "-uiTestRemoteDemo"]
        app.launch()

        // The rail slides away to its tab 4 s after the screen appears. Wait
        // for that, then bring it back, so the tap never races the slide.
        let show = app.buttons["Show keyboard"]
        let tab = app.buttons["Show controls"]
        XCTAssertTrue(tab.waitForExistence(timeout: 30), "the rail did not hide to its tab")
        tab.tap()
        XCTAssertTrue(show.waitForExistence(timeout: 5))
        show.tap()

        let space = app.descendants(matching: .any)["Space"]
        XCTAssertTrue(space.waitForExistence(timeout: 10))
        XCTAssertTrue(app.descendants(matching: .any)["Ctrl"].exists)
        XCTAssertTrue(app.buttons["Hide keyboard"].waitForExistence(timeout: 5))
        snap(app, "08-remote-keyboard")
    }

    /// Drags `element` up to just under the page header, without a fling.
    private func dragToTop(_ app: XCUIApplication, _ element: XCUIElement) {
        if !element.exists { scrollUntilHittable(app, element) }
        let from = element.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0)).withOffset(CGVector(dx: 0, dy: 150))
        from.press(forDuration: 0.1, thenDragTo: to, withVelocity: .slow, thenHoldForDuration: 0.3)
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
