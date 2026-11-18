import Foundation

/// What the gesture layer asks the session to do. Coordinates are view pixels.
protocol TouchActions {
    /// Absolute mode: move the remote pointer to this phone point.
    func pointerTo(x: Float, y: Float)
    /// Trackpad mode: move the remote pointer by this many phone pixels.
    func pointerBy(dx: Float, dy: Float)
    /// Button transition; [atX]/[atY] are the phone point, or NaN for "where the pointer is".
    func button(button: Int, down: Bool, atX: Float, atY: Float)
    /// Wheel delta in Windows units (120 = one notch); +dy scrolls up, +dx right.
    func scroll(dx: Int, dy: Int)
    func zoom(focusX: Float, focusY: Float, factor: Float)
    func pan(dx: Float, dy: Float)
    func feedback(kind: Feedback)
    /// Three fingers tapped together (used to toggle the keyboard).
    func threeFingerTap()
}

extension TouchActions {
    /// Kotlin's default arguments: the phone point is NaN, "where the pointer is".
    func button(button: Int, down: Bool) {
        self.button(button: button, down: down, atX: .nan, atY: .nan)
    }

    func threeFingerTap() {}
}

enum Feedback: String {
    case dragStart = "DRAG_START"
    case rightClick = "RIGHT_CLICK"
}

enum PointerMode: String {
    case direct = "DIRECT"
    case trackpad = "TRACKPAD"
}

let BUTTON_LEFT = 0
let BUTTON_RIGHT = 1

/// Turns raw touches into remote-desktop actions. Pure logic: the view feeds
/// it pointer positions with timestamps and calls [onTimer] when asked to.
///
/// Direct touch: tap = click there; long press = start a drag there; one
/// finger moving = move the pointer (hover); two-finger drag = scroll; pinch
/// = zoom the phone view; two-finger tap = right click.
///
/// Trackpad: one finger = relative pointer motion; tap = click; double tap
/// and hold = drag; two fingers as in direct mode (right click at the
/// pointer).
///
/// Accidental clicks are avoided by deciding late: nothing is sent for a
/// touch until it is clearly a tap, a hold, a move, or a two-finger gesture.
final class TouchInterpreter {
    private let actions: TouchActions
    /// Movement (px) below which a touch is still a tap.
    private let slop: Float
    /// Pixels of finger travel per wheel notch.
    private let scrollStepPx: Float
    /// Screen density (px per dp), so speed thresholds mean the same on every phone.
    private let density: Float

    init(actions: TouchActions, slop: Float, scrollStepPx: Float, density: Float = 1) {
        self.actions = actions
        self.slop = slop
        self.scrollStepPx = scrollStepPx
        self.density = density
    }

    var mode = PointerMode.direct
    var sensitivity: Float = 1.0
    var acceleration: Float = 0.5

    static let TAP_MS: Int64 = 250
    static let LONG_PRESS_MS: Int64 = 420
    static let DOUBLE_TAP_MS: Int64 = 280
    static let ZOOM_SPAN_RATIO: Float = 0.08
    /// A third finger this soon after the second still counts as one tap.
    static let THIRD_FINGER_MS: Int64 = 150
    /// Below this finger speed (dp/ms) the trackpad slows down for precision.
    static let PRECISION_SPEED: Float = 0.12
    /// Gain at a standstill, relative to [sensitivity].
    static let PRECISION_MIN_GAIN: Float = 0.45

    /// Trackpad gain for a finger speed in dp/ms: a precision zone for
    /// slow, deliberate motion, 1x in the middle, acceleration for flicks.
    static func trackpadGain(speedDpMs: Float, sensitivity: Float, acceleration: Float) -> Float {
        let precision: Float = speedDpMs < PRECISION_SPEED
            ? PRECISION_MIN_GAIN + (1 - PRECISION_MIN_GAIN) * (speedDpMs / PRECISION_SPEED)
            : 1
        let accel = 1 + acceleration * min(max(speedDpMs - 0.3, 0), 2)
        return sensitivity * precision * accel
    }

    private enum State: String {
        case idle = "IDLE"
        case pending = "PENDING"
        case hover = "HOVER"
        case drag = "DRAG"
        case trackMove = "TRACK_MOVE"
        case trackDrag = "TRACK_DRAG"
        case twoPending = "TWO_PENDING"
        case scroll = "SCROLL"
        case zoom = "ZOOM"
        case threePending = "THREE_PENDING"
        case done = "DONE"
    }

    private var state = State.idle
    private var downX: Float = 0
    private var downY: Float = 0
    private var downT: Int64 = 0
    private var lastX: Float = 0
    private var lastY: Float = 0
    private var lastT: Int64 = 0
    private var lastTapEndT: Int64 = Int64.min / 2
    private var lastTapX: Float = 0
    private var lastTapY: Float = 0
    private var trackDragMoved = false

    // Two-finger state.
    private var twoDownT: Int64 = 0
    private var startSpan: Float = 0
    private var lastSpan: Float = 0
    private var startCx: Float = 0
    private var startCy: Float = 0
    private var lastCx: Float = 0
    private var lastCy: Float = 0
    private var scrollAccX: Float = 0
    private var scrollAccY: Float = 0
    private var threeDownT: Int64 = 0

    /// When non-zero, the view should call [onTimer] at this uptime (ms).
    private(set) var timerAt: Int64 = 0

    func reset() {
        if state == .drag || state == .trackDrag { actions.button(button: BUTTON_LEFT, down: false) }
        state = .idle
        timerAt = 0
    }

    // ------------------------------------------------------------ single

    func onDown(x: Float, y: Float, t: Int64) {
        downX = x; downY = y; downT = t
        lastX = x; lastY = y; lastT = t
        let near = hypotf(x - lastTapX, y - lastTapY) < slop * 3
        if mode == .trackpad && t - lastTapEndT < TouchInterpreter.DOUBLE_TAP_MS && near {
            // Second touch of a double tap: press now; drag on move.
            actions.button(button: BUTTON_LEFT, down: true)
            state = .trackDrag
            trackDragMoved = false
            timerAt = 0
            return
        }
        state = .pending
        timerAt = mode == .direct ? t + TouchInterpreter.LONG_PRESS_MS : 0
    }

    func onTimer(t: Int64) {
        if timerAt == 0 || t < timerAt { return }
        timerAt = 0
        if state == .pending && mode == .direct {
            actions.button(button: BUTTON_LEFT, down: true, atX: downX, atY: downY)
            actions.feedback(kind: .dragStart)
            state = .drag
        }
    }

    func onMove(x: Float, y: Float, t: Int64) {
        switch state {
        case .pending:
            if hypotf(x - downX, y - downY) > slop {
                timerAt = 0
                if mode == .direct {
                    state = .hover
                    actions.pointerTo(x: x, y: y)
                } else {
                    state = .trackMove
                    relative(x: x, y: y, t: t)
                }
            }
        case .hover, .drag:
            actions.pointerTo(x: x, y: y)
        case .trackMove:
            relative(x: x, y: y, t: t)
        case .trackDrag:
            if trackDragMoved || hypotf(x - downX, y - downY) > slop {
                trackDragMoved = true
                relative(x: x, y: y, t: t)
            }
        default:
            break
        }
        lastX = x; lastY = y; lastT = t
    }

    private func relative(x: Float, y: Float, t: Int64) {
        let dx = x - lastX
        let dy = y - lastY
        let dt = max(1, t - lastT)
        let speed = hypotf(dx, dy) / Float(dt) / density // dp per ms
        let gain = TouchInterpreter.trackpadGain(speedDpMs: speed, sensitivity: sensitivity, acceleration: acceleration)
        actions.pointerBy(dx: dx * gain, dy: dy * gain)
        lastX = x; lastY = y; lastT = t
    }

    func onUp(x: Float, y: Float, t: Int64) {
        timerAt = 0
        switch state {
        case .pending:
            // Within slop: a tap (a slow tap still counts; only a hold
            // past LONG_PRESS_MS turns into a drag).
            if mode == .direct {
                actions.button(button: BUTTON_LEFT, down: true, atX: downX, atY: downY)
                actions.button(button: BUTTON_LEFT, down: false, atX: downX, atY: downY)
            } else if t - downT <= TouchInterpreter.TAP_MS * 2 {
                actions.button(button: BUTTON_LEFT, down: true)
                actions.button(button: BUTTON_LEFT, down: false)
                lastTapEndT = t
                lastTapX = x; lastTapY = y
            }
        case .drag:
            actions.button(button: BUTTON_LEFT, down: false, atX: x, atY: y)
        case .trackDrag:
            actions.button(button: BUTTON_LEFT, down: false)
            lastTapEndT = Int64.min / 2 // a triple tap starts fresh
        default:
            break
        }
        state = .idle
    }

    // ------------------------------------------------------------ two fingers

    func onSecondDown(x0: Float, y0: Float, x1: Float, y1: Float, t: Int64) {
        timerAt = 0
        switch state {
        case .pending, .hover, .trackMove, .idle:
            state = .twoPending
            twoDownT = t
            startSpan = hypotf(x1 - x0, y1 - y0)
            lastSpan = startSpan
            startCx = (x0 + x1) / 2; startCy = (y0 + y1) / 2
            lastCx = startCx; lastCy = startCy
            scrollAccX = 0; scrollAccY = 0
        default:
            // A second finger during a drag is ignored; the drag continues.
            break
        }
    }

    func onTwoMove(x0: Float, y0: Float, x1: Float, y1: Float, t: Int64) {
        let span = hypotf(x1 - x0, y1 - y0)
        let cx = (x0 + x1) / 2
        let cy = (y0 + y1) / 2
        switch state {
        case .twoPending:
            if startSpan > 0 && abs(span - startSpan) / startSpan > TouchInterpreter.ZOOM_SPAN_RATIO {
                state = .zoom
            } else if hypotf(cx - startCx, cy - startCy) > slop {
                state = .scroll
            }
            lastSpan = span; lastCx = cx; lastCy = cy
        case .zoom:
            if lastSpan > 0 { actions.zoom(focusX: cx, focusY: cy, factor: span / lastSpan) }
            actions.pan(dx: cx - lastCx, dy: cy - lastCy)
            lastSpan = span; lastCx = cx; lastCy = cy
        case .scroll:
            // Natural scrolling: content follows the fingers.
            scrollAccY += (cy - lastCy) * 120 / scrollStepPx
            scrollAccX += -(cx - lastCx) * 120 / scrollStepPx
            let sy = Int(scrollAccY)
            let sx = Int(scrollAccX)
            if sx != 0 || sy != 0 {
                actions.scroll(dx: sx, dy: sy)
                scrollAccX -= Float(sx); scrollAccY -= Float(sy)
            }
            lastCx = cx; lastCy = cy
        default:
            break
        }
    }

    /// A third finger touched while two were down. Fingers of one tap land
    /// tens of milliseconds apart, and in that time the first two may already
    /// have been read as a pinch or a scroll; a third finger soon after the
    /// second still makes it a three-finger tap.
    func onThirdDown(t: Int64) {
        timerAt = 0
        let landing = t - twoDownT <= TouchInterpreter.THIRD_FINGER_MS
        if state == .twoPending || (landing && (state == .zoom || state == .scroll)) {
            state = .threePending
            threeDownT = t
        }
    }

    /// One of three fingers lifted.
    func onThirdUp(t: Int64) {
        if state == .threePending && t - threeDownT <= TouchInterpreter.TAP_MS + 150 { actions.threeFingerTap() }
        if state == .threePending { state = .done }
    }

    /// One of two fingers lifted.
    func onSecondUp(t: Int64) {
        if state == .twoPending && t - twoDownT <= TouchInterpreter.TAP_MS + 100 {
            if mode == .direct {
                actions.button(button: BUTTON_RIGHT, down: true, atX: lastCx, atY: lastCy)
                actions.button(button: BUTTON_RIGHT, down: false, atX: lastCx, atY: lastCy)
            } else {
                actions.button(button: BUTTON_RIGHT, down: true)
                actions.button(button: BUTTON_RIGHT, down: false)
            }
            actions.feedback(kind: .rightClick)
        }
        if state != .drag && state != .trackDrag {
            // Wait for the remaining finger to lift; it must not click.
            state = .done
        }
    }

    func onCancel() { reset() }

    /// For tests and the diagnostics overlay.
    var stateName: String { state.rawValue }
}
