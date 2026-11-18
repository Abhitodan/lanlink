package com.lanlink.app.core

import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.max

/** What the gesture layer asks the session to do. Coordinates are view pixels. */
interface TouchActions {
    /** Absolute mode: move the remote pointer to this phone point. */
    fun pointerTo(x: Float, y: Float)
    /** Trackpad mode: move the remote pointer by this many phone pixels. */
    fun pointerBy(dx: Float, dy: Float)
    /** Button transition; [atX]/[atY] are the phone point, or NaN for "where the pointer is". */
    fun button(button: Int, down: Boolean, atX: Float = Float.NaN, atY: Float = Float.NaN)
    /** Wheel delta in Windows units (120 = one notch); +dy scrolls up, +dx right. */
    fun scroll(dx: Int, dy: Int)
    fun zoom(focusX: Float, focusY: Float, factor: Float)
    fun pan(dx: Float, dy: Float)
    fun feedback(kind: Feedback)
    /** Three fingers tapped together (used to toggle the keyboard). */
    fun threeFingerTap() {}
}

enum class Feedback { DRAG_START, RIGHT_CLICK }

enum class PointerMode { DIRECT, TRACKPAD }

const val BUTTON_LEFT = 0
const val BUTTON_RIGHT = 1

/**
 * Turns raw touches into remote-desktop actions. Pure logic: the view feeds
 * it pointer positions with timestamps and calls [onTimer] when asked to.
 *
 * Direct touch: tap = click there; long press = start a drag there; one
 * finger moving = move the pointer (hover); two-finger drag = scroll; pinch
 * = zoom the phone view; two-finger tap = right click.
 *
 * Trackpad: one finger = relative pointer motion; tap = click; double tap
 * and hold = drag; two fingers as in direct mode (right click at the
 * pointer).
 *
 * Accidental clicks are avoided by deciding late: nothing is sent for a
 * touch until it is clearly a tap, a hold, a move, or a two-finger gesture.
 */
class TouchInterpreter(
    private val actions: TouchActions,
    /** Movement (px) below which a touch is still a tap. */
    private val slop: Float,
    /** Pixels of finger travel per wheel notch. */
    private val scrollStepPx: Float,
    /** Screen density (px per dp), so speed thresholds mean the same on every phone. */
    private val density: Float = 1f,
) {
    var mode = PointerMode.DIRECT
    var sensitivity = 1.0f
    var acceleration = 0.5f

    companion object {
        const val TAP_MS = 250L
        const val LONG_PRESS_MS = 420L
        const val DOUBLE_TAP_MS = 280L
        const val ZOOM_SPAN_RATIO = 0.08f
        /** A third finger this soon after the second still counts as one tap. */
        const val THIRD_FINGER_MS = 150L
        /** Below this finger speed (dp/ms) the trackpad slows down for precision. */
        const val PRECISION_SPEED = 0.12f
        /** Gain at a standstill, relative to [sensitivity]. */
        const val PRECISION_MIN_GAIN = 0.45f

        /**
         * Trackpad gain for a finger speed in dp/ms: a precision zone for
         * slow, deliberate motion, 1× in the middle, acceleration for flicks.
         */
        fun trackpadGain(speedDpMs: Float, sensitivity: Float, acceleration: Float): Float {
            val precision = if (speedDpMs < PRECISION_SPEED)
                PRECISION_MIN_GAIN + (1f - PRECISION_MIN_GAIN) * (speedDpMs / PRECISION_SPEED)
            else 1f
            val accel = 1f + acceleration * (speedDpMs - 0.3f).coerceIn(0f, 2f)
            return sensitivity * precision * accel
        }
    }

    private enum class State { IDLE, PENDING, HOVER, DRAG, TRACK_MOVE, TRACK_DRAG, TWO_PENDING, SCROLL, ZOOM, THREE_PENDING, DONE }

    private var state = State.IDLE
    private var downX = 0f
    private var downY = 0f
    private var downT = 0L
    private var lastX = 0f
    private var lastY = 0f
    private var lastT = 0L
    private var lastTapEndT = Long.MIN_VALUE / 2
    private var lastTapX = 0f
    private var lastTapY = 0f
    private var trackDragMoved = false

    // Two-finger state.
    private var twoDownT = 0L
    private var startSpan = 0f
    private var lastSpan = 0f
    private var startCx = 0f
    private var startCy = 0f
    private var lastCx = 0f
    private var lastCy = 0f
    private var scrollAccX = 0f
    private var scrollAccY = 0f
    private var threeDownT = 0L

    /** When non-zero, the view should call [onTimer] at this uptime (ms). */
    var timerAt = 0L
        private set

    fun reset() {
        if (state == State.DRAG || state == State.TRACK_DRAG) actions.button(BUTTON_LEFT, false)
        state = State.IDLE
        timerAt = 0L
    }

    // ------------------------------------------------------------ single

    fun onDown(x: Float, y: Float, t: Long) {
        downX = x; downY = y; downT = t
        lastX = x; lastY = y; lastT = t
        val near = hypot(x - lastTapX, y - lastTapY) < slop * 3
        if (mode == PointerMode.TRACKPAD && t - lastTapEndT < DOUBLE_TAP_MS && near) {
            // Second touch of a double tap: press now; drag on move.
            actions.button(BUTTON_LEFT, true)
            state = State.TRACK_DRAG
            trackDragMoved = false
            timerAt = 0L
            return
        }
        state = State.PENDING
        timerAt = if (mode == PointerMode.DIRECT) t + LONG_PRESS_MS else 0L
    }

    fun onTimer(t: Long) {
        if (timerAt == 0L || t < timerAt) return
        timerAt = 0L
        if (state == State.PENDING && mode == PointerMode.DIRECT) {
            actions.button(BUTTON_LEFT, true, downX, downY)
            actions.feedback(Feedback.DRAG_START)
            state = State.DRAG
        }
    }

    fun onMove(x: Float, y: Float, t: Long) {
        when (state) {
            State.PENDING -> {
                if (hypot(x - downX, y - downY) > slop) {
                    timerAt = 0L
                    if (mode == PointerMode.DIRECT) {
                        state = State.HOVER
                        actions.pointerTo(x, y)
                    } else {
                        state = State.TRACK_MOVE
                        relative(x, y, t)
                    }
                }
            }
            State.HOVER, State.DRAG -> actions.pointerTo(x, y)
            State.TRACK_MOVE -> relative(x, y, t)
            State.TRACK_DRAG -> {
                if (trackDragMoved || hypot(x - downX, y - downY) > slop) {
                    trackDragMoved = true
                    relative(x, y, t)
                }
            }
            else -> {}
        }
        lastX = x; lastY = y; lastT = t
    }

    private fun relative(x: Float, y: Float, t: Long) {
        val dx = x - lastX
        val dy = y - lastY
        val dt = max(1L, t - lastT)
        val speed = hypot(dx, dy) / dt / density // dp per ms
        val gain = trackpadGain(speed, sensitivity, acceleration)
        actions.pointerBy(dx * gain, dy * gain)
        lastX = x; lastY = y; lastT = t
    }

    fun onUp(x: Float, y: Float, t: Long) {
        timerAt = 0L
        when (state) {
            State.PENDING -> {
                // Within slop: a tap (a slow tap still counts; only a hold
                // past LONG_PRESS_MS turns into a drag).
                if (mode == PointerMode.DIRECT) {
                    actions.button(BUTTON_LEFT, true, downX, downY)
                    actions.button(BUTTON_LEFT, false, downX, downY)
                } else if (t - downT <= TAP_MS * 2) {
                    actions.button(BUTTON_LEFT, true)
                    actions.button(BUTTON_LEFT, false)
                    lastTapEndT = t
                    lastTapX = x; lastTapY = y
                }
            }
            State.DRAG -> actions.button(BUTTON_LEFT, false, x, y)
            State.TRACK_DRAG -> {
                actions.button(BUTTON_LEFT, false)
                lastTapEndT = Long.MIN_VALUE / 2 // a triple tap starts fresh
            }
            else -> {}
        }
        state = State.IDLE
    }

    // ------------------------------------------------------------ two fingers

    fun onSecondDown(x0: Float, y0: Float, x1: Float, y1: Float, t: Long) {
        timerAt = 0L
        when (state) {
            State.PENDING, State.HOVER, State.TRACK_MOVE, State.IDLE -> {
                state = State.TWO_PENDING
                twoDownT = t
                startSpan = hypot(x1 - x0, y1 - y0)
                lastSpan = startSpan
                startCx = (x0 + x1) / 2f; startCy = (y0 + y1) / 2f
                lastCx = startCx; lastCy = startCy
                scrollAccX = 0f; scrollAccY = 0f
            }
            // A second finger during a drag is ignored; the drag continues.
            else -> {}
        }
    }

    fun onTwoMove(x0: Float, y0: Float, x1: Float, y1: Float, t: Long) {
        val span = hypot(x1 - x0, y1 - y0)
        val cx = (x0 + x1) / 2f
        val cy = (y0 + y1) / 2f
        when (state) {
            State.TWO_PENDING -> {
                if (startSpan > 0f && abs(span - startSpan) / startSpan > ZOOM_SPAN_RATIO) {
                    state = State.ZOOM
                } else if (hypot(cx - startCx, cy - startCy) > slop) {
                    state = State.SCROLL
                }
                lastSpan = span; lastCx = cx; lastCy = cy
            }
            State.ZOOM -> {
                if (lastSpan > 0f) actions.zoom(cx, cy, span / lastSpan)
                actions.pan(cx - lastCx, cy - lastCy)
                lastSpan = span; lastCx = cx; lastCy = cy
            }
            State.SCROLL -> {
                // Natural scrolling: content follows the fingers.
                scrollAccY += (cy - lastCy) * 120f / scrollStepPx
                scrollAccX += -(cx - lastCx) * 120f / scrollStepPx
                val sy = scrollAccY.toInt()
                val sx = scrollAccX.toInt()
                if (sx != 0 || sy != 0) {
                    actions.scroll(sx, sy)
                    scrollAccX -= sx; scrollAccY -= sy
                }
                lastCx = cx; lastCy = cy
            }
            else -> {}
        }
    }

    /**
     * A third finger touched while two were down. Fingers of one tap land
     * tens of milliseconds apart, and in that time the first two may already
     * have been read as a pinch or a scroll; a third finger soon after the
     * second still makes it a three-finger tap.
     */
    fun onThirdDown(t: Long) {
        timerAt = 0L
        val landing = t - twoDownT <= THIRD_FINGER_MS
        if (state == State.TWO_PENDING || (landing && (state == State.ZOOM || state == State.SCROLL))) {
            state = State.THREE_PENDING
            threeDownT = t
        }
    }

    /** One of three fingers lifted. */
    fun onThirdUp(t: Long) {
        if (state == State.THREE_PENDING && t - threeDownT <= TAP_MS + 150) actions.threeFingerTap()
        if (state == State.THREE_PENDING) state = State.DONE
    }

    /** One of two fingers lifted. */
    fun onSecondUp(t: Long) {
        if (state == State.TWO_PENDING && t - twoDownT <= TAP_MS + 100) {
            if (mode == PointerMode.DIRECT) {
                actions.button(BUTTON_RIGHT, true, lastCx, lastCy)
                actions.button(BUTTON_RIGHT, false, lastCx, lastCy)
            } else {
                actions.button(BUTTON_RIGHT, true)
                actions.button(BUTTON_RIGHT, false)
            }
            actions.feedback(Feedback.RIGHT_CLICK)
        }
        if (state != State.DRAG && state != State.TRACK_DRAG) {
            // Wait for the remaining finger to lift; it must not click.
            state = State.DONE
        }
    }

    fun onCancel() = reset()

    /** For tests and the diagnostics overlay. */
    val stateName: String get() = state.name
}
