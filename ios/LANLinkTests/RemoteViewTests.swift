import UIKit
import XCTest
@testable import LANLink

// Tests for the remote view (twin of android/.../session/RemoteView.kt),
// the privacy cover and the compact keyboard's key repeat. Touches are fed
// as plain points with millisecond times, as MotionEvents are on Android.

final class RemoteViewTests: XCTestCase {
    /// Trackpad drag: the pointer moves by exact PC pixels from the position
    /// the PC reported (Kotlin `movesByExactPcPixelsFromTheReportedPosition`).
    /// The source is 2561 × 1601 as in the Kotlin fixture, so one PC pixel is
    /// exactly 1/2560 of the width; the view is half that size, so one point
    /// of finger travel is two PC pixels. The view must not be leaked by its
    /// gesture interpreter (Task 6 finding M6).
    @MainActor
    func testTrackpadDragMovesCursorByExactPixels() {
        let input = RecordingInput()
        weak var weakView: RemoteView?
        autoreleasepool {
            let view = RemoteView(input: input)
            weakView = view
            view.frame = CGRect(x: 0, y: 0, width: 1280.5, height: 800.5)
            view.pointerMode = .trackpad
            view.setSensitivity(1.0, 0)
            view.onVideoGeometry(width: 2561, height: 1601, sourceWidth: 2561, sourceHeight: 1601)
            view.layoutIfNeeded()
            view.onCursorPosition(nx: 0, ny: 0, visible: true)

            // Fast strokes (≈ 21 pt/ms): no precision slow-down, and the
            // acceleration is off, so the gain is exactly 1.
            view.feedBegan([TouchPoint(id: 1, x: 100, y: 100)], t: 1000)
            view.feedMoved([TouchPoint(id: 1, x: 420, y: 200)], t: 1016)
            view.feedMoved([TouchPoint(id: 1, x: 740, y: 300)], t: 1032)
            view.feedEnded([TouchPoint(id: 1, x: 740, y: 300)], t: 1048)

            // 640 × 200 points = 1280 × 400 PC pixels: half way across and a
            // quarter down. The last position is sent again on lift-off.
            XCTAssertEqual(input.pointers, ["16384,8192", "32768,16384", "32768,16384"])
            XCTAssertFalse(input.calls.contains { $0.hasPrefix("button") }, "a drag must not click")
            view.shutdown()
        }
        XCTAssertNil(weakView, "RemoteView is retained (TouchInterpreter → actions → view)")
    }

    /// Keyboard open: the view slides so the bottom of the PC's caret sits at
    /// REVEAL_AT (75 %) of the part of the view above the keyboard.
    @MainActor
    func testKeyboardInsetKeepsCaretVisible() {
        let input = RecordingInput()
        let view = RemoteView(input: input)
        view.frame = CGRect(x: 0, y: 0, width: 1000, height: 800)
        view.onVideoGeometry(width: 2560, height: 1600, sourceWidth: 2560, sourceHeight: 1600)
        view.layoutIfNeeded()

        view.setKeyboardInset(400)
        XCTAssertEqual(input.calls.filter { $0 == "setTextInput(true)" }.count, 1)
        // The caret near the bottom of the PC screen.
        view.onCaret(visible: true, x: 32768, y: 50000, w: 200, h: 1000)

        let visibleH: Float = 800 - 400
        let (_, bottom) = Viewport.toView(nx: 32768, ny: 51000, box: view.box)
        XCTAssertEqual(bottom, visibleH * Viewport.REVEAL_AT, accuracy: 2)
        // Typing zoom keeps the text legible.
        XCTAssertGreaterThanOrEqual(view.zoomLevel, RemoteView.TYPING_ZOOM - 0.01)

        view.setKeyboardInset(0)
        XCTAssertEqual(input.calls.last, "setTextInput(false)")
        view.shutdown()
    }

    /// Rotation keeps the zoom level and re-clamps: the zoomed picture still
    /// covers the view on the axis where it is larger and is centred on the
    /// other, so no empty edge appears.
    @MainActor
    func testRotationReclampsViewport() {
        let view = RemoteView(input: RecordingInput())
        view.frame = CGRect(x: 0, y: 0, width: 844, height: 390)
        view.onVideoGeometry(width: 2560, height: 1600, sourceWidth: 2560, sourceHeight: 1600)
        view.layoutIfNeeded()
        view.zoomTo(2)
        XCTAssertEqual(view.zoomLevel, 2, accuracy: 0.01)

        view.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
        view.setNeedsLayout()
        view.layoutIfNeeded()

        let b = view.box
        XCTAssertEqual(view.zoomLevel, 2, accuracy: 0.01)
        XCTAssertLessThanOrEqual(b.left, 0)
        XCTAssertGreaterThanOrEqual(b.right, 390)
        XCTAssertEqual(b.top, (844 - b.height) / 2, accuracy: 0.5)
        XCTAssertEqual(b.width, view.base.width * 2, accuracy: 0.5)
        view.shutdown()
    }

    /// Leaving the foreground: held keys and buttons are released on the PC
    /// and the PC screen is covered before iOS takes the app-switcher
    /// snapshot (the iPhone stand-in for FLAG_SECURE, fix M5).
    @MainActor
    func testBackgroundReleasesInputAndCovers() {
        let input = RecordingInput()
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        let view = RemoteView(input: input)
        view.frame = window.bounds
        view.privacy.allowScreenshots = false
        window.addSubview(view)

        NotificationCenter.default.post(name: UIScene.willDeactivateNotification, object: nil)
        XCTAssertTrue(input.calls.contains("releaseAll"))
        XCTAssertTrue(window.subviews.contains { $0 is PrivacyCoverView }, "no cover over the PC screen")

        NotificationCenter.default.post(name: UIScene.didActivateNotification, object: nil)
        XCTAssertFalse(window.subviews.contains { $0 is PrivacyCoverView })
        view.shutdown()
    }

    /// Holding ⌫ on the compact keyboard: one press at once, a repeat after
    /// 400 ms, then every 55 ms: 1 + ⌊(600 − 400) / 55⌋ + 1 = 5 at 600 ms.
    @MainActor
    func testCompactKeyboardRepeatsBackspace() {
        let clock = ManualClock()
        var keys: [Int] = []
        let keyboard = CompactKeyboardController(
            clock: clock, onText: { _ in }, onKey: { keys.append($0) }, onSticky: { _ in }, onHide: {})
        let backspace = CompactLayout.rows(symbols: false, shifted: false)[0].last!
        XCTAssertEqual(backspace.action, .key(KeyMap.VK_BACK))

        let hold = keyboard.press(backspace)
        clock.advance(by: 600)
        hold.release()
        clock.advance(by: 1000)

        XCTAssertEqual(keys, Array(repeating: KeyMap.VK_BACK, count: 5))
    }
}

/// Records what the view sends to the PC.
private final class RecordingInput: RemoteInput {
    var calls: [String] = []
    var pointers: [String] = []

    func pointer(x: Int, y: Int) {
        pointers.append("\(x),\(y)")
        calls.append("pointer")
    }

    func pointerBy(dx: Int, dy: Int) { calls.append("pointerBy") }
    func button(_ button: Int, down: Bool, x: Int, y: Int) { calls.append("button(\(button),\(down))") }
    func scroll(dx: Int, dy: Int) { calls.append("scroll") }
    func key(_ vk: Int, down: Bool) { calls.append("key(\(vk),\(down))") }
    func tapKey(_ vk: Int) { calls.append("tapKey(\(vk))") }
    func text(_ t: String) { calls.append("text(\(t))") }
    func setTextInput(_ active: Bool) { calls.append("setTextInput(\(active))") }
    func releaseAll() { calls.append("releaseAll") }
    func toggleSticky(_ vk: Int) { calls.append("toggleSticky(\(vk))") }
    func combo(vks: Int...) { calls.append("combo") }
}

/// A clock the test moves by hand; due actions run in time order.
private final class ManualClock: RepeatClock {
    private struct Pending {
        let id: Int
        let at: Int64
        let action: () -> Void
    }

    private var now: Int64 = 0
    private var nextId = 0
    private var pending: [Pending] = []

    func after(ms: Int64, _ action: @escaping () -> Void) -> () -> Void {
        nextId += 1
        let id = nextId
        pending.append(Pending(id: id, at: now + ms, action: action))
        return { [weak self] in self?.pending.removeAll { $0.id == id } }
    }

    func advance(by ms: Int64) {
        let end = now + ms
        while let next = pending.filter({ $0.at <= end }).min(by: { ($0.at, $0.id) < ($1.at, $1.id) }) {
            pending.removeAll { $0.id == next.id }
            now = next.at
            next.action()
        }
        now = end
    }
}
