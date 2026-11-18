import XCTest
@testable import LANLink

// Swift twins of android/.../core/CoreLogicTest.kt. Same inputs, expected
// values and tolerances; KeyMap uses iOS HID usages for the same physical
// keys and asserts the same Windows virtual-key values.

private func assertPair<T: Equatable>(
    _ actual: (T, T), _ expected: (T, T),
    file: StaticString = #filePath, line: UInt = #line
) {
    XCTAssertEqual(actual.0, expected.0, file: file, line: line)
    XCTAssertEqual(actual.1, expected.1, file: file, line: line)
}

final class ViewportTests: XCTestCase {
    func testLetterboxesWideVideoInPortraitView() {
        let b = Viewport.base(viewW: 1080, viewH: 2400, videoW: 2560, videoH: 1600, fill: false)
        XCTAssertEqual(b.width, 1080, accuracy: 0.01)
        XCTAssertEqual(b.height, 675, accuracy: 0.01)
        XCTAssertEqual(b.top, (2400 - 675) / 2, accuracy: 0.01)
        XCTAssertEqual(b.left, 0, accuracy: 0.01)
    }

    func testFillCropsInsteadOfLetterboxing() {
        let b = Viewport.base(viewW: 2400, viewH: 1080, videoW: 1920, videoH: 1200, fill: true)
        XCTAssertEqual(b.width, 2400, accuracy: 0.01)
        XCTAssertTrue(b.height > 1080)
        XCTAssertTrue(b.top < 0)
    }

    func testCornersMapToRemoteCornersAndBack() {
        let b = Viewport.base(viewW: 2400, viewH: 1080, videoW: 1920, videoH: 1200, fill: false)
        assertPair(Viewport.toRemote(x: b.left, y: b.top, box: b), (0, 0))
        assertPair(Viewport.toRemote(x: b.right - 1, y: b.bottom - 1, box: b), (65535, 65535))
        // Points on the letterbox bars clamp to the nearest edge.
        XCTAssertEqual(Viewport.toRemote(x: 0, y: 500, box: b).0, 0)
        let (x, y) = Viewport.toView(nx: 32768, ny: 32768, box: b)
        let back = Viewport.toRemote(x: x, y: y, box: b)
        XCTAssertEqual(32768.0, Double(back.0), accuracy: 2.0)
        XCTAssertEqual(32768.0, Double(back.1), accuracy: 2.0)
    }

    func testZoomKeepsFocusPointFixedAndClamps() {
        let base = Viewport.base(viewW: 1000, viewH: 1000, videoW: 1000, videoH: 1000, fill: false)
        let z = Viewport.zoomAround(box: base, base: base, fx: 250, fy: 250, factor: 2, viewW: 1000, viewH: 1000)
        XCTAssertEqual(Viewport.zoomOf(box: z, base: base), 2, accuracy: 0.001)
        // The content under (250,250) stays under the finger (to within one
        // phone pixel: normalization uses the pixel-centre convention).
        let before = Viewport.toRemote(x: 250, y: 250, box: base)
        let after = Viewport.toRemote(x: 250, y: 250, box: z)
        let onePixel = 65535.0 / 1000
        XCTAssertEqual(Double(before.0), Double(after.0), accuracy: onePixel)
        XCTAssertEqual(Double(before.1), Double(after.1), accuracy: onePixel)
        // Never zooms out past the base or beyond the maximum.
        let out = Viewport.zoomAround(box: z, base: base, fx: 500, fy: 500, factor: 0.1, viewW: 1000, viewH: 1000)
        XCTAssertEqual(Viewport.zoomOf(box: out, base: base), 1, accuracy: 0.001)
        let big = Viewport.zoomAround(box: base, base: base, fx: 500, fy: 500, factor: 100, viewW: 1000, viewH: 1000)
        XCTAssertEqual(Viewport.zoomOf(box: big, base: base), Viewport.MAX_ZOOM, accuracy: 0.001)
    }

    func testFollowCameraStillInsideZoneAndPullsBackOutside() {
        // 1000x500 view, middle 60 %: x in 200..800, y in 100..400.
        assertPair(Viewport.followPan(px: 500, py: 250, viewW: 1000, viewH: 500, zone: 0.6), (0, 0))
        assertPair(Viewport.followPan(px: 200, py: 400, viewW: 1000, viewH: 500, zone: 0.6), (0, 0))
        let (dx, dy) = Viewport.followPan(px: 900, py: 50, viewW: 1000, viewH: 500, zone: 0.6)
        XCTAssertEqual(dx, -100, accuracy: 0.01) // pan left so the point lands at x = 800
        XCTAssertEqual(dy, 50, accuracy: 0.01)   // pan down so it lands at y = 100
    }

    func testRevealPanLeavesAComfortableCaretAlone() {
        // 2000 x 500 visible above the keyboard; caret in the middle.
        assertPair(Viewport.revealPan(x: 1000, top: 230, bottom: 260, viewW: 2000, visibleH: 500), (0, 0))
    }

    func testRevealPanLiftsACaretHiddenByTheKeyboard() {
        // Caret at y 900..930, keyboard covers everything below 500.
        let (dx, dy) = Viewport.revealPan(x: 1000, top: 900, bottom: 930, viewW: 2000, visibleH: 500)
        XCTAssertEqual(dx, 0, accuracy: 0.01)
        XCTAssertEqual(dy, 500 * Viewport.REVEAL_AT - 930, accuracy: 0.01) // bottom lands at 75 %
    }

    func testRevealPanBringsBackACaretNearTheSides() {
        let (dxRight, _) = Viewport.revealPan(x: 1950, top: 230, bottom: 260, viewW: 2000, visibleH: 500)
        XCTAssertEqual(dxRight, 1200 - 1950, accuracy: 0.01) // to 60 % of the width
        let (dxLeft, _) = Viewport.revealPan(x: 50, top: 230, bottom: 260, viewW: 2000, visibleH: 500)
        XCTAssertEqual(dxLeft, 800 - 50, accuracy: 0.01) // to 40 %
    }

    func testKeyboardRegionClampLetsTheBottomOfThePcRiseAboveTheKeyboard() {
        // Fit: the picture fills the 1000-high view; the keyboard leaves 400.
        let box = Box(left: 0, top: 0, width: 1600, height: 1000)
        let lifted = Viewport.pan(box: box, dx: 0, dy: -800, viewW: 1600, viewH: 400)
        XCTAssertEqual(lifted.top, -600, accuracy: 0.01) // bottom edge sits at y 400
    }

    func testZoomToAndActualPixels() {
        let base = Viewport.base(viewW: 2000, viewH: 1000, videoW: 2560, videoH: 1600, fill: false) // 1600x1000
        let z2 = Viewport.zoomTo(box: base, base: base, fx: 1000, fy: 500, target: 2, viewW: 2000, viewH: 1000)
        XCTAssertEqual(Viewport.zoomOf(box: z2, base: base), 2, accuracy: 1e-3)
        XCTAssertEqual(Viewport.actualPixelsZoom(base: base, sourceW: 2560), 1.6, accuracy: 1e-3)
        // A PC smaller than the phone view never zooms below fit.
        XCTAssertEqual(Viewport.actualPixelsZoom(base: base, sourceW: 1280), 1, accuracy: 1e-3)
        let back = Viewport.zoomTo(box: z2, base: base, fx: 1000, fy: 500, target: 1, viewW: 2000, viewH: 1000)
        XCTAssertEqual(back.width, base.width, accuracy: 0.01)
        XCTAssertEqual(back.left, base.left, accuracy: 0.01)
        XCTAssertEqual(back.top, base.top, accuracy: 0.01)
    }

    func testPanCannotExposeEmptySpace() {
        let base = Viewport.base(viewW: 1000, viewH: 1000, videoW: 1000, videoH: 1000, fill: false)
        let z = Viewport.zoomAround(box: base, base: base, fx: 500, fy: 500, factor: 2, viewW: 1000, viewH: 1000)
        let p = Viewport.pan(box: z, dx: 5000, dy: 5000, viewW: 1000, viewH: 1000)
        XCTAssertEqual(p.left, 0, accuracy: 0.01)
        XCTAssertEqual(p.top, 0, accuracy: 0.01)
        let q = Viewport.pan(box: z, dx: -5000, dy: 0, viewW: 1000, viewH: 1000)
        XCTAssertEqual(q.right, 1000, accuracy: 0.01)
    }
}

private final class Rec: TouchActions {
    var log: [String] = []
    func pointerTo(x: Float, y: Float) { log.append("to(\(Int(x)),\(Int(y)))") }
    func pointerBy(dx: Float, dy: Float) { log.append("by") }
    func button(button: Int, down: Bool, atX: Float, atY: Float) {
        let at = atX.isNaN ? "" : "@\(Int(atX)),\(Int(atY))"
        let name = button == 0 ? "L" : "R"
        let edge = down ? "down" : "up"
        log.append(name + edge + at)
    }
    func scroll(dx: Int, dy: Int) { log.append("scroll(\(dx),\(dy))") }
    func zoom(focusX: Float, focusY: Float, factor: Float) { log.append("zoom") }
    func pan(dx: Float, dy: Float) {}
    func feedback(kind: Feedback) { log.append(kind.rawValue) }
    func threeFingerTap() { log.append("THREE") }
}

final class TouchInterpreterTests: XCTestCase {
    private func ti(_ rec: Rec, _ mode: PointerMode = .direct) -> TouchInterpreter {
        let t = TouchInterpreter(actions: rec, slop: 10, scrollStepPx: 40)
        t.mode = mode
        return t
    }

    func testTapClicksWhereTouched() {
        let r = Rec(); let t = ti(r)
        t.onDown(x: 100, y: 200, t: 0); t.onMove(x: 103, y: 201, t: 50); t.onUp(x: 103, y: 201, t: 90)
        XCTAssertEqual(r.log, ["Ldown@100,200", "Lup@100,200"])
    }

    func testMovingFingerHoversWithoutClicking() {
        let r = Rec(); let t = ti(r)
        t.onDown(x: 100, y: 100, t: 0); t.onMove(x: 150, y: 100, t: 30)
        t.onMove(x: 200, y: 100, t: 60); t.onUp(x: 200, y: 100, t: 90)
        XCTAssertEqual(r.log, ["to(150,100)", "to(200,100)"])
    }

    func testLongPressStartsDragAndLiftEndsIt() {
        let r = Rec(); let t = ti(r)
        t.onDown(x: 10, y: 10, t: 0)
        XCTAssertEqual(t.timerAt, TouchInterpreter.LONG_PRESS_MS)
        t.onTimer(t: TouchInterpreter.LONG_PRESS_MS)
        t.onMove(x: 80, y: 10, t: 600); t.onUp(x: 80, y: 10, t: 700)
        XCTAssertEqual(r.log, ["Ldown@10,10", "DRAG_START", "to(80,10)", "Lup@80,10"])
    }

    func testTwoFingerTapRightClicksAndRemainingFingerDoesNotClick() {
        let r = Rec(); let t = ti(r)
        t.onDown(x: 100, y: 100, t: 0)
        t.onSecondDown(x0: 100, y0: 100, x1: 200, y1: 100, t: 20)
        t.onSecondUp(t: 120)
        t.onUp(x: 100, y: 100, t: 140)
        XCTAssertEqual(r.log, ["Rdown@150,100", "Rup@150,100", "RIGHT_CLICK"])
    }

    func testTwoFingerDragScrollsNaturally() {
        let r = Rec(); let t = ti(r)
        t.onDown(x: 100, y: 100, t: 0)
        t.onSecondDown(x0: 100, y0: 100, x1: 200, y1: 100, t: 10)
        // Fingers move down 60 px: past slop, then 40 px = one notch up.
        t.onTwoMove(x0: 100, y0: 120, x1: 200, y1: 120, t: 30)
        t.onTwoMove(x0: 100, y0: 160, x1: 200, y1: 160, t: 60)
        t.onSecondUp(t: 400); t.onUp(x: 100, y: 160, t: 410)
        XCTAssertEqual(r.log, ["scroll(0,120)"])
    }

    func testPinchZoomsAndNeverClicks() {
        let r = Rec(); let t = ti(r)
        t.onDown(x: 100, y: 100, t: 0)
        t.onSecondDown(x0: 100, y0: 100, x1: 200, y1: 100, t: 10)
        t.onTwoMove(x0: 80, y0: 100, x1: 220, y1: 100, t: 40)
        t.onTwoMove(x0: 60, y0: 100, x1: 240, y1: 100, t: 70)
        t.onSecondUp(t: 100); t.onUp(x: 60, y: 100, t: 110)
        XCTAssertTrue(r.log.allSatisfy { $0 == "zoom" })
        XCTAssertFalse(r.log.isEmpty)
    }

    func testTrackpadTapClicksAtPointerAndDoubleTapHoldDrags() {
        let r = Rec(); let t = ti(r, .trackpad)
        t.onDown(x: 500, y: 500, t: 0); t.onUp(x: 500, y: 500, t: 80)
        XCTAssertEqual(r.log, ["Ldown", "Lup"])
        r.log.removeAll()
        t.onDown(x: 502, y: 501, t: 200) // second tap within the window: press
        t.onMove(x: 540, y: 501, t: 260); t.onUp(x: 540, y: 501, t: 400)
        XCTAssertEqual(r.log.first, "Ldown")
        XCTAssertTrue(r.log.contains("by"))
        XCTAssertEqual(r.log.last, "Lup")
    }

    func testThreeFingerTapIsReportedAndNothingClicks() {
        let r = Rec(); let t = ti(r)
        t.onDown(x: 100, y: 100, t: 0)
        t.onSecondDown(x0: 100, y0: 100, x1: 200, y1: 100, t: 10)
        t.onThirdDown(t: 20)
        t.onThirdUp(t: 150); t.onSecondUp(t: 160); t.onUp(x: 100, y: 100, t: 170)
        XCTAssertEqual(r.log, ["THREE"])
    }

    func testThreeFingerTapStillCountsWhenFingersLandUnevenly() {
        // Real fingers land tens of ms apart; the first two can already look
        // like a pinch before the third arrives.
        let r = Rec(); let t = ti(r, .trackpad)
        t.onDown(x: 100, y: 100, t: 0)
        t.onSecondDown(x0: 100, y0: 100, x1: 200, y1: 100, t: 10)
        t.onTwoMove(x0: 100, y0: 100, x1: 230, y1: 100, t: 25) // span +30 %: looks like a pinch
        t.onThirdDown(t: 45)
        t.onThirdUp(t: 200); t.onSecondUp(t: 210); t.onUp(x: 100, y: 100, t: 220)
        XCTAssertEqual(r.log, ["THREE"])
    }

    func testThirdFingerLongAfterAPinchIsNotATap() {
        let r = Rec(); let t = ti(r, .trackpad)
        t.onDown(x: 100, y: 100, t: 0)
        t.onSecondDown(x0: 100, y0: 100, x1: 200, y1: 100, t: 10)
        t.onTwoMove(x0: 100, y0: 100, x1: 230, y1: 100, t: 25)
        t.onTwoMove(x0: 100, y0: 100, x1: 260, y1: 100, t: 300)
        t.onThirdDown(t: 600)
        t.onThirdUp(t: 700); t.onSecondUp(t: 710); t.onUp(x: 100, y: 100, t: 720)
        XCTAssertFalse(r.log.contains("THREE"))
    }

    func testSlowThreeFingerHoldIsNotATap() {
        let r = Rec(); let t = ti(r)
        t.onDown(x: 100, y: 100, t: 0)
        t.onSecondDown(x0: 100, y0: 100, x1: 200, y1: 100, t: 10)
        t.onThirdDown(t: 20)
        t.onThirdUp(t: 900); t.onSecondUp(t: 910); t.onUp(x: 100, y: 100, t: 920)
        XCTAssertTrue(r.log.isEmpty)
    }

    func testTrackpadGainHasPrecisionZoneAndAcceleration() {
        let still = TouchInterpreter.trackpadGain(speedDpMs: 0, sensitivity: 1, acceleration: 0.5)
        let slow = TouchInterpreter.trackpadGain(speedDpMs: 0.06, sensitivity: 1, acceleration: 0.5)
        let normal = TouchInterpreter.trackpadGain(speedDpMs: 0.2, sensitivity: 1, acceleration: 0.5)
        let fast = TouchInterpreter.trackpadGain(speedDpMs: 1.5, sensitivity: 1, acceleration: 0.5)
        XCTAssertEqual(still, TouchInterpreter.PRECISION_MIN_GAIN, accuracy: 1e-4)
        XCTAssertTrue(still < slow && slow < normal)
        XCTAssertEqual(normal, 1, accuracy: 1e-4)
        XCTAssertTrue(fast > 1.5)
        // Sensitivity scales everything.
        let doubled = TouchInterpreter.trackpadGain(speedDpMs: 0.2, sensitivity: 2, acceleration: 0.5)
        XCTAssertEqual(2 * normal, doubled, accuracy: 1e-4)
    }

    func testTrackpadMoveIsRelative() {
        let r = Rec(); let t = ti(r, .trackpad)
        t.onDown(x: 0, y: 0, t: 0); t.onMove(x: 30, y: 0, t: 16)
        t.onMove(x: 60, y: 0, t: 32); t.onUp(x: 60, y: 0, t: 48)
        XCTAssertEqual(r.log, ["by", "by"])
    }
}

final class TrackpadCursorTests: XCTestCase {
    // 2561 x 1601 source: one pixel is exactly 1/2560 of the width.
    private let w = 2561
    private let h = 1601

    func testMovesByExactPcPixelsFromTheReportedPosition() {
        let c = TrackpadCursor()
        XCTAssertTrue(c.hostReport(nx: 0, ny: 0, t: 0))
        let (nx, ny) = c.moveBy(dxPc: 1280, dyPc: 400, srcW: w, srcH: h, t: 1000)
        XCTAssertEqual(nx, 32768) // half way across (0.5 x 65535, rounded)
        XCTAssertEqual(ny, 16384) // a quarter down
    }

    func testSubPixelMotionAccumulatesInsteadOfBeingLost() {
        let c = TrackpadCursor()
        _ = c.hostReport(nx: 0, ny: 0, t: 0)
        for i in 0..<10 { _ = c.moveBy(dxPc: 0.25, dyPc: 0, srcW: w, srcH: h, t: 1000 + Int64(i)) }
        XCTAssertEqual(c.fx, 2.5 / 2560, accuracy: 1e-6)
    }

    func testStaysOnTheScreen() {
        let c = TrackpadCursor()
        _ = c.hostReport(nx: 60000, ny: 60000, t: 0)
        assertPair(c.moveBy(dxPc: 9000, dyPc: 9000, srcW: w, srcH: h, t: 1000), (65535, 65535))
        assertPair(c.moveBy(dxPc: -99999, dyPc: -99999, srcW: w, srcH: h, t: 1001), (0, 0))
    }

    func testPcReportIsIgnoredWhileTheFingerMovesAndAdoptedAfter() {
        let c = TrackpadCursor()
        _ = c.hostReport(nx: 0, ny: 0, t: 0)
        _ = c.moveBy(dxPc: 1280, dyPc: 0, srcW: w, srcH: h, t: 1000)
        // A report sent before our move reached the PC must not yank it back.
        XCTAssertFalse(c.hostReport(nx: 100, ny: 100, t: 1000 + TrackpadCursor.HOLD_MS - 1))
        XCTAssertEqual(c.normalized().0, 32768)
        // Later the PC is the truth (e.g. someone moved the real mouse).
        XCTAssertTrue(c.hostReport(nx: 100, ny: 200, t: 1000 + TrackpadCursor.HOLD_MS))
        assertPair(c.normalized(), (100, 200))
    }
}

final class CompactKeyboardTests: XCTestCase {
    func testShiftIsOneShotAndDoubleTapLocks() {
        let s = ShiftState()
        s.tap(t: 0); XCTAssertTrue(s.shifted); s.typed(); XCTAssertFalse(s.shifted)
        s.tap(t: 1000); s.tap(t: 1000 + ShiftState.DOUBLE_TAP_MS)
        XCTAssertTrue(s.locked)
        s.typed(); s.typed(); XCTAssertTrue(s.shifted)
        s.tap(t: 5000); XCTAssertFalse(s.shifted); XCTAssertFalse(s.locked)
    }

    func testSlowSecondShiftTapTurnsShiftOff() {
        let s = ShiftState()
        s.tap(t: 0); s.tap(t: ShiftState.DOUBLE_TAP_MS + 1)
        XCTAssertFalse(s.shifted); XCTAssertFalse(s.locked)
    }

    func testLettersFollowShiftAndEveryLayerHasTheDesktopKeys() {
        let lower = CompactLayout.rows(symbols: false, shifted: false)
        let upper = CompactLayout.rows(symbols: false, shifted: true)
        XCTAssertEqual(KeyAction.text("q"), lower[0][0].action)
        XCTAssertEqual(KeyAction.text("Q"), upper[0][0].action)
        for symbols in [false, true] {
            let rows = CompactLayout.rows(symbols: symbols, shifted: false)
            let actions = rows.flatMap { $0 }.map { $0.action }
            XCTAssertEqual(4, rows.count, "4 compact rows")
            for vk in [KeyMap.VK_BACK, KeyMap.VK_RETURN, KeyMap.VK_LEFT, KeyMap.VK_RIGHT] {
                XCTAssertTrue(actions.contains(KeyAction.key(vk)), "vk \(vk) on layer symbols=\(symbols)")
            }
            XCTAssertTrue(actions.contains(KeyAction.sticky(KeyMap.VK_LCONTROL)))
            XCTAssertTrue(actions.contains(KeyAction.hide))
            XCTAssertTrue(actions.contains(KeyAction.layer))
        }
        let symbolActions = CompactLayout.rows(symbols: true, shifted: false).flatMap { $0 }.map { $0.action }
        for d in "0123456789" {
            XCTAssertTrue(symbolActions.contains(KeyAction.text(String(d))))
        }
        XCTAssertTrue(symbolActions.contains(KeyAction.key(KeyMap.VK_ESCAPE)))
    }
}

final class KeyMapTests: XCTestCase {
    // UIKeyboardHIDUsage values (USB HID usage page 0x07) for the same
    // physical keys the Kotlin test names by Android code.
    func testLettersDigitsAndNavigation() {
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x04), 0x41) // A
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x1D), 0x5A) // Z
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x27), 0x30) // 0
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x1E), 0x31) // 1
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x45), KeyMap.VK_F1 + 11) // F12
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x3A), KeyMap.VK_F1) // F1
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x28), KeyMap.VK_RETURN)
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x2B), KeyMap.VK_TAB)
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x29), KeyMap.VK_ESCAPE)
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x2A), KeyMap.VK_BACK) // Backspace
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x4C), KeyMap.VK_DELETE) // Forward delete
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x50), KeyMap.VK_LEFT)
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x4F), KeyMap.VK_RIGHT)
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x52), KeyMap.VK_UP)
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0x51), KeyMap.VK_DOWN)
        XCTAssertEqual(KeyMap.vkFor(keyCode: 0xE0), KeyMap.VK_LCONTROL) // Left Control
        XCTAssertNil(KeyMap.vkFor(keyCode: 0x80)) // volume up is not forwarded
    }

    func testModifiersAndShortcutChars() {
        XCTAssertTrue(KeyMap.isModifier(vk: KeyMap.VK_LCONTROL))
        XCTAssertTrue(KeyMap.isModifier(vk: KeyMap.VK_LWIN))
        XCTAssertFalse(KeyMap.isModifier(vk: 0x41))
        XCTAssertEqual(KeyMap.vkForShortcutChar(c: "c"), 0x43)
        XCTAssertEqual(KeyMap.vkForShortcutChar(c: "C"), 0x43)
        XCTAssertNil(KeyMap.vkForShortcutChar(c: "\u{E9}"))
    }
}

final class DisplayTextTests: XCTestCase {
    func testStripsInvisibleAndDirectionCharacters() {
        XCTAssertEqual("Office PC", DisplayText.name(s: "Office PC"))
        XCTAssertEqual("enohP", DisplayText.name(s: "\u{202E}enohP\u{202C}"))
        XCTAssertEqual("ABC", DisplayText.name(s: "A\u{2066}B\u{2069}\u{200B}C"))
        XCTAssertEqual("Line break", DisplayText.name(s: "  Line\n\tbreak "))
        XCTAssertEqual("Pixel", DisplayText.name(s: "Pixel 10", maxChars: 5))
        XCTAssertEqual("PC \u{1F4BB}", DisplayText.name(s: "PC \u{1F4BB}"))
    }
}
