package com.lanlink.app.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ViewportTest {
    @Test fun letterboxesWideVideoInPortraitView() {
        val b = Viewport.base(1080f, 2400f, 2560, 1600, fill = false)
        assertEquals(1080f, b.width, 0.01f)
        assertEquals(675f, b.height, 0.01f)
        assertEquals((2400f - 675f) / 2f, b.top, 0.01f)
        assertEquals(0f, b.left, 0.01f)
    }

    @Test fun fillCropsInsteadOfLetterboxing() {
        val b = Viewport.base(2400f, 1080f, 1920, 1200, fill = true)
        assertEquals(2400f, b.width, 0.01f)
        assertTrue(b.height > 1080f)
        assertTrue(b.top < 0f)
    }

    @Test fun cornersMapToRemoteCornersAndBack() {
        val b = Viewport.base(2400f, 1080f, 1920, 1200, fill = false)
        assertEquals(0 to 0, Viewport.toRemote(b.left, b.top, b))
        assertEquals(65535 to 65535, Viewport.toRemote(b.right - 1f, b.bottom - 1f, b))
        // Points on the letterbox bars clamp to the nearest edge.
        assertEquals(0, Viewport.toRemote(0f, 500f, b).first)
        val (x, y) = Viewport.toView(32768, 32768, b)
        val back = Viewport.toRemote(x, y, b)
        assertEquals(32768.0, back.first.toDouble(), 2.0)
        assertEquals(32768.0, back.second.toDouble(), 2.0)
    }

    @Test fun zoomKeepsFocusPointFixedAndClamps() {
        val base = Viewport.base(1000f, 1000f, 1000, 1000, fill = false)
        val z = Viewport.zoomAround(base, base, 250f, 250f, 2f, 1000f, 1000f)
        assertEquals(2f, Viewport.zoomOf(z, base), 0.001f)
        // The content under (250,250) stays under the finger (to within one
        // phone pixel: normalization uses the pixel-centre convention).
        val before = Viewport.toRemote(250f, 250f, base)
        val after = Viewport.toRemote(250f, 250f, z)
        val onePixel = 65535.0 / 1000
        assertEquals(before.first.toDouble(), after.first.toDouble(), onePixel)
        assertEquals(before.second.toDouble(), after.second.toDouble(), onePixel)
        // Never zooms out past the base or beyond the maximum.
        val out = Viewport.zoomAround(z, base, 500f, 500f, 0.1f, 1000f, 1000f)
        assertEquals(1f, Viewport.zoomOf(out, base), 0.001f)
        val big = Viewport.zoomAround(base, base, 500f, 500f, 100f, 1000f, 1000f)
        assertEquals(Viewport.MAX_ZOOM, Viewport.zoomOf(big, base), 0.001f)
    }

    @Test fun followCameraStillInsideZoneAndPullsBackOutside() {
        // 1000x500 view, middle 60 %: x in 200..800, y in 100..400.
        assertEquals(0f to 0f, Viewport.followPan(500f, 250f, 1000f, 500f, 0.6f))
        assertEquals(0f to 0f, Viewport.followPan(200f, 400f, 1000f, 500f, 0.6f))
        val (dx, dy) = Viewport.followPan(900f, 50f, 1000f, 500f, 0.6f)
        assertEquals(-100f, dx, 0.01f) // pan left so the point lands at x = 800
        assertEquals(50f, dy, 0.01f)   // pan down so it lands at y = 100
    }

    @Test fun revealPanLeavesAComfortableCaretAlone() {
        // 2000 × 500 visible above the keyboard; caret in the middle.
        assertEquals(0f to 0f, Viewport.revealPan(1000f, 230f, 260f, 2000f, 500f))
    }

    @Test fun revealPanLiftsACaretHiddenByTheKeyboard() {
        // Caret at y 900..930, keyboard covers everything below 500.
        val (dx, dy) = Viewport.revealPan(1000f, 900f, 930f, 2000f, 500f)
        assertEquals(0f, dx, 0.01f)
        assertEquals(500f * Viewport.REVEAL_AT - 930f, dy, 0.01f) // bottom lands at 75 %
    }

    @Test fun revealPanBringsBackACaretNearTheSides() {
        val (dxRight, _) = Viewport.revealPan(1950f, 230f, 260f, 2000f, 500f)
        assertEquals(1200f - 1950f, dxRight, 0.01f) // to 60 % of the width
        val (dxLeft, _) = Viewport.revealPan(50f, 230f, 260f, 2000f, 500f)
        assertEquals(800f - 50f, dxLeft, 0.01f) // to 40 %
    }

    @Test fun keyboardRegionClampLetsTheBottomOfThePcRiseAboveTheKeyboard() {
        // Fit: the picture fills the 1000-high view; the keyboard leaves 400.
        val box = Box(0f, 0f, 1600f, 1000f)
        val lifted = Viewport.pan(box, 0f, -800f, 1600f, 400f)
        assertEquals(-600f, lifted.top, 0.01f) // bottom edge sits at y 400
    }

    @Test fun zoomToAndActualPixels() {
        val base = Viewport.base(2000f, 1000f, 2560, 1600, fill = false) // 1600x1000
        val z2 = Viewport.zoomTo(base, base, 1000f, 500f, 2f, 2000f, 1000f)
        assertEquals(2f, Viewport.zoomOf(z2, base), 1e-3f)
        assertEquals(1.6f, Viewport.actualPixelsZoom(base, 2560), 1e-3f)
        // A PC smaller than the phone view never zooms below fit.
        assertEquals(1f, Viewport.actualPixelsZoom(base, 1280), 1e-3f)
        val back = Viewport.zoomTo(z2, base, 1000f, 500f, 1f, 2000f, 1000f)
        assertEquals(base.width, back.width, 0.01f)
        assertEquals(base.left, back.left, 0.01f)
        assertEquals(base.top, back.top, 0.01f)
    }

    @Test fun panCannotExposeEmptySpace() {
        val base = Viewport.base(1000f, 1000f, 1000, 1000, fill = false)
        val z = Viewport.zoomAround(base, base, 500f, 500f, 2f, 1000f, 1000f)
        val p = Viewport.pan(z, 5000f, 5000f, 1000f, 1000f)
        assertEquals(0f, p.left, 0.01f)
        assertEquals(0f, p.top, 0.01f)
        val q = Viewport.pan(z, -5000f, 0f, 1000f, 1000f)
        assertEquals(1000f, q.right, 0.01f)
    }
}

private class Rec : TouchActions {
    val log = mutableListOf<String>()
    override fun pointerTo(x: Float, y: Float) { log += "to(${x.toInt()},${y.toInt()})" }
    override fun pointerBy(dx: Float, dy: Float) { log += "by" }
    override fun button(button: Int, down: Boolean, atX: Float, atY: Float) {
        val at = if (atX.isNaN()) "" else "@${atX.toInt()},${atY.toInt()}"
        log += "${if (button == 0) "L" else "R"}${if (down) "down" else "up"}$at"
    }
    override fun scroll(dx: Int, dy: Int) { log += "scroll($dx,$dy)" }
    override fun zoom(focusX: Float, focusY: Float, factor: Float) { log += "zoom" }
    override fun pan(dx: Float, dy: Float) {}
    override fun feedback(kind: Feedback) { log += kind.name }
    override fun threeFingerTap() { log += "THREE" }
}

class TouchInterpreterTest {
    private fun ti(rec: Rec, mode: PointerMode = PointerMode.DIRECT) =
        TouchInterpreter(rec, slop = 10f, scrollStepPx = 40f).also { it.mode = mode }

    @Test fun tapClicksWhereTouched() {
        val r = Rec(); val t = ti(r)
        t.onDown(100f, 200f, 0); t.onMove(103f, 201f, 50); t.onUp(103f, 201f, 90)
        assertEquals(listOf("Ldown@100,200", "Lup@100,200"), r.log)
    }

    @Test fun movingFingerHoversWithoutClicking() {
        val r = Rec(); val t = ti(r)
        t.onDown(100f, 100f, 0); t.onMove(150f, 100f, 30); t.onMove(200f, 100f, 60); t.onUp(200f, 100f, 90)
        assertEquals(listOf("to(150,100)", "to(200,100)"), r.log)
    }

    @Test fun longPressStartsDragAndLiftEndsIt() {
        val r = Rec(); val t = ti(r)
        t.onDown(10f, 10f, 0)
        assertEquals(TouchInterpreter.LONG_PRESS_MS, t.timerAt)
        t.onTimer(TouchInterpreter.LONG_PRESS_MS)
        t.onMove(80f, 10f, 600); t.onUp(80f, 10f, 700)
        assertEquals(listOf("Ldown@10,10", "DRAG_START", "to(80,10)", "Lup@80,10"), r.log)
    }

    @Test fun twoFingerTapRightClicksAndRemainingFingerDoesNotClick() {
        val r = Rec(); val t = ti(r)
        t.onDown(100f, 100f, 0)
        t.onSecondDown(100f, 100f, 200f, 100f, 20)
        t.onSecondUp(120)
        t.onUp(100f, 100f, 140)
        assertEquals(listOf("Rdown@150,100", "Rup@150,100", "RIGHT_CLICK"), r.log)
    }

    @Test fun twoFingerDragScrollsNaturally() {
        val r = Rec(); val t = ti(r)
        t.onDown(100f, 100f, 0)
        t.onSecondDown(100f, 100f, 200f, 100f, 10)
        // Fingers move down 60 px: past slop, then 40 px = one notch up.
        t.onTwoMove(100f, 120f, 200f, 120f, 30)
        t.onTwoMove(100f, 160f, 200f, 160f, 60)
        t.onSecondUp(400); t.onUp(100f, 160f, 410)
        assertEquals(listOf("scroll(0,120)"), r.log)
    }

    @Test fun pinchZoomsAndNeverClicks() {
        val r = Rec(); val t = ti(r)
        t.onDown(100f, 100f, 0)
        t.onSecondDown(100f, 100f, 200f, 100f, 10)
        t.onTwoMove(80f, 100f, 220f, 100f, 40)
        t.onTwoMove(60f, 100f, 240f, 100f, 70)
        t.onSecondUp(100); t.onUp(60f, 100f, 110)
        assertTrue(r.log.all { it == "zoom" })
        assertTrue(r.log.isNotEmpty())
    }

    @Test fun trackpadTapClicksAtPointerAndDoubleTapHoldDrags() {
        val r = Rec(); val t = ti(r, PointerMode.TRACKPAD)
        t.onDown(500f, 500f, 0); t.onUp(500f, 500f, 80)
        assertEquals(listOf("Ldown", "Lup"), r.log)
        r.log.clear()
        t.onDown(502f, 501f, 200) // second tap within the window: press
        t.onMove(540f, 501f, 260); t.onUp(540f, 501f, 400)
        assertEquals("Ldown", r.log.first())
        assertTrue(r.log.contains("by"))
        assertEquals("Lup", r.log.last())
    }

    @Test fun threeFingerTapIsReportedAndNothingClicks() {
        val r = Rec(); val t = ti(r)
        t.onDown(100f, 100f, 0)
        t.onSecondDown(100f, 100f, 200f, 100f, 10)
        t.onThirdDown(20)
        t.onThirdUp(150); t.onSecondUp(160); t.onUp(100f, 100f, 170)
        assertEquals(listOf("THREE"), r.log)
    }

    @Test fun threeFingerTapStillCountsWhenFingersLandUnevenly() {
        // Real fingers land tens of ms apart; the first two can already look
        // like a pinch before the third arrives.
        val r = Rec(); val t = ti(r, PointerMode.TRACKPAD)
        t.onDown(100f, 100f, 0)
        t.onSecondDown(100f, 100f, 200f, 100f, 10)
        t.onTwoMove(100f, 100f, 230f, 100f, 25) // span +30 %: looks like a pinch
        t.onThirdDown(45)
        t.onThirdUp(200); t.onSecondUp(210); t.onUp(100f, 100f, 220)
        assertEquals(listOf("THREE"), r.log)
    }

    @Test fun thirdFingerLongAfterAPinchIsNotATap() {
        val r = Rec(); val t = ti(r, PointerMode.TRACKPAD)
        t.onDown(100f, 100f, 0)
        t.onSecondDown(100f, 100f, 200f, 100f, 10)
        t.onTwoMove(100f, 100f, 230f, 100f, 25)
        t.onTwoMove(100f, 100f, 260f, 100f, 300)
        t.onThirdDown(600)
        t.onThirdUp(700); t.onSecondUp(710); t.onUp(100f, 100f, 720)
        assertFalse(r.log.contains("THREE"))
    }

    @Test fun slowThreeFingerHoldIsNotATap() {
        val r = Rec(); val t = ti(r)
        t.onDown(100f, 100f, 0)
        t.onSecondDown(100f, 100f, 200f, 100f, 10)
        t.onThirdDown(20)
        t.onThirdUp(900); t.onSecondUp(910); t.onUp(100f, 100f, 920)
        assertTrue(r.log.isEmpty())
    }

    @Test fun trackpadGainHasPrecisionZoneAndAcceleration() {
        val still = TouchInterpreter.trackpadGain(0f, 1f, 0.5f)
        val slow = TouchInterpreter.trackpadGain(0.06f, 1f, 0.5f)
        val normal = TouchInterpreter.trackpadGain(0.2f, 1f, 0.5f)
        val fast = TouchInterpreter.trackpadGain(1.5f, 1f, 0.5f)
        assertEquals(TouchInterpreter.PRECISION_MIN_GAIN, still, 1e-4f)
        assertTrue(still < slow && slow < normal)
        assertEquals(1f, normal, 1e-4f)
        assertTrue(fast > 1.5f)
        // Sensitivity scales everything.
        assertEquals(2f * normal, TouchInterpreter.trackpadGain(0.2f, 2f, 0.5f), 1e-4f)
    }

    @Test fun trackpadMoveIsRelative() {
        val r = Rec(); val t = ti(r, PointerMode.TRACKPAD)
        t.onDown(0f, 0f, 0); t.onMove(30f, 0f, 16); t.onMove(60f, 0f, 32); t.onUp(60f, 0f, 48)
        assertEquals(listOf("by", "by"), r.log)
    }
}

class TrackpadCursorTest {
    // 2561 × 1601 source: one pixel is exactly 1/2560 of the width.
    private val w = 2561
    private val h = 1601

    @Test fun movesByExactPcPixelsFromTheReportedPosition() {
        val c = TrackpadCursor()
        assertTrue(c.hostReport(0, 0, t = 0))
        val (nx, ny) = c.moveBy(1280f, 400f, w, h, t = 1000)
        assertEquals(32768, nx) // half way across (0.5 × 65535, rounded)
        assertEquals(16384, ny) // a quarter down
    }

    @Test fun subPixelMotionAccumulatesInsteadOfBeingLost() {
        val c = TrackpadCursor()
        c.hostReport(0, 0, t = 0)
        repeat(10) { c.moveBy(0.25f, 0f, w, h, t = 1000L + it) }
        assertEquals(2.5f / 2560f, c.fx, 1e-6f)
    }

    @Test fun staysOnTheScreen() {
        val c = TrackpadCursor()
        c.hostReport(60000, 60000, t = 0)
        assertEquals(65535 to 65535, c.moveBy(9000f, 9000f, w, h, t = 1000))
        assertEquals(0 to 0, c.moveBy(-99999f, -99999f, w, h, t = 1001))
    }

    @Test fun pcReportIsIgnoredWhileTheFingerMovesAndAdoptedAfter() {
        val c = TrackpadCursor()
        c.hostReport(0, 0, t = 0)
        c.moveBy(1280f, 0f, w, h, t = 1000)
        // A report sent before our move reached the PC must not yank it back.
        assertFalse(c.hostReport(100, 100, t = 1000 + TrackpadCursor.HOLD_MS - 1))
        assertEquals(32768, c.normalized().first)
        // Later the PC is the truth (e.g. someone moved the real mouse).
        assertTrue(c.hostReport(100, 200, t = 1000 + TrackpadCursor.HOLD_MS))
        assertEquals(100 to 200, c.normalized())
    }
}

class CompactKeyboardTest {
    @Test fun shiftIsOneShotAndDoubleTapLocks() {
        val s = ShiftState()
        s.tap(0); assertTrue(s.shifted); s.typed(); assertFalse(s.shifted)
        s.tap(1000); s.tap(1000 + ShiftState.DOUBLE_TAP_MS)
        assertTrue(s.locked)
        s.typed(); s.typed(); assertTrue(s.shifted)
        s.tap(5000); assertFalse(s.shifted); assertFalse(s.locked)
    }

    @Test fun slowSecondShiftTapTurnsShiftOff() {
        val s = ShiftState()
        s.tap(0); s.tap(ShiftState.DOUBLE_TAP_MS + 1)
        assertFalse(s.shifted); assertFalse(s.locked)
    }

    @Test fun lettersFollowShiftAndEveryLayerHasTheDesktopKeys() {
        val lower = CompactLayout.rows(symbols = false, shifted = false)
        val upper = CompactLayout.rows(symbols = false, shifted = true)
        assertEquals(KeyAction.Text("q"), lower[0][0].action)
        assertEquals(KeyAction.Text("Q"), upper[0][0].action)
        for (symbols in listOf(false, true)) {
            val actions = CompactLayout.rows(symbols, false).flatten().map { it.action }
            assertEquals("4 compact rows", 4, CompactLayout.rows(symbols, false).size)
            listOf(KeyMap.VK_BACK, KeyMap.VK_RETURN, KeyMap.VK_LEFT, KeyMap.VK_RIGHT).forEach {
                assertTrue("vk $it on layer symbols=$symbols", KeyAction.Key(it) in actions)
            }
            assertTrue(KeyAction.Sticky(KeyMap.VK_LCONTROL) in actions)
            assertTrue(KeyAction.Hide in actions)
            assertTrue(KeyAction.Layer in actions)
        }
        val symbols = CompactLayout.rows(symbols = true, shifted = false).flatten().map { it.action }
        ('0'..'9').forEach { assertTrue(KeyAction.Text(it.toString()) in symbols) }
        assertTrue(KeyAction.Key(KeyMap.VK_ESCAPE) in symbols)
    }
}

class KeyMapTest {
    @Test fun lettersDigitsAndNavigation() {
        assertEquals(0x41, KeyMap.vkFor(29)) // A
        assertEquals(0x5A, KeyMap.vkFor(54)) // Z
        assertEquals(0x30, KeyMap.vkFor(7))
        assertEquals(KeyMap.VK_F1 + 11, KeyMap.vkFor(142)) // F12
        assertEquals(KeyMap.VK_BACK, KeyMap.vkFor(67))
        assertEquals(KeyMap.VK_DELETE, KeyMap.vkFor(112))
        assertEquals(KeyMap.VK_LEFT, KeyMap.vkFor(21))
        assertEquals(KeyMap.VK_LCONTROL, KeyMap.vkFor(113))
        assertNull(KeyMap.vkFor(24)) // volume up is not forwarded
    }

    @Test fun modifiersAndShortcutChars() {
        assertTrue(KeyMap.isModifier(KeyMap.VK_LCONTROL))
        assertTrue(KeyMap.isModifier(KeyMap.VK_LWIN))
        assertFalse(KeyMap.isModifier(0x41))
        assertEquals(0x43, KeyMap.vkForShortcutChar('c'))
        assertEquals(0x43, KeyMap.vkForShortcutChar('C'))
        assertNull(KeyMap.vkForShortcutChar('é'))
    }
}

class DisplayTextTest {
    @org.junit.Test
    fun stripsInvisibleAndDirectionCharacters() {
        org.junit.Assert.assertEquals("Office PC", DisplayText.name("Office PC"))
        org.junit.Assert.assertEquals("enohP", DisplayText.name("‮enohP‬"))
        org.junit.Assert.assertEquals("ABC", DisplayText.name("A⁦B⁩​C"))
        org.junit.Assert.assertEquals("Line break", DisplayText.name("  Line\n\tbreak "))
        org.junit.Assert.assertEquals("Pixel", DisplayText.name("Pixel 10", 5))
        org.junit.Assert.assertEquals("PC 💻", DisplayText.name("PC 💻"))
    }
}
