import AVFoundation
import UIKit

// Swift twin of android/.../session/RemoteView.kt.

/// One finger: the touch's identity and its position in view points.
struct TouchPoint {
    let id: AnyHashable
    let x: Float
    let y: Float

    init<H: Hashable>(id: H, x: Float, y: Float) {
        self.id = AnyHashable(id)
        self.x = x
        self.y = y
    }
}

/// The remote desktop surface: video, local cursor, touch and keyboard.
///
/// Plain UIKit on purpose: video frames and cursor motion must never trigger
/// a SwiftUI update. The video goes to an `AVSampleBufferDisplayLayer` whose
/// frame implements letterboxing, zoom and pan; the cursor is a light layer
/// that only moves itself.
///
/// Units: view points. They stand in for Android's dp, so `density` is 1
/// and the dp constants (slop, scroll step, typing spot) keep their values.
final class RemoteView: UIView, TouchActions, CursorSink {
    /// Fraction of the view (per axis) the pointer roams before the view follows.
    static let FOLLOW_ZONE: Float = 0.6
    /// Minimum zoom while typing, so the text is legible.
    static let TYPING_ZOOM: Float = 1.5

    private let input: RemoteInput
    private let videoView = VideoLayerView()
    private let stillView = UIImageView()
    private let cursorLayer = CursorLayer()
    private let captureNotice = UILabel()
    let keyboard: KeyInputView
    /// Cover and background rules (see PrivacyCover.swift).
    let privacy: PrivacyGuard
    private let density: Float = 1
    private let forwarder = TouchForwarder()
    private lazy var interpreter = TouchInterpreter(
        actions: forwarder, slop: 8, scrollStepPx: 28 * density, density: density)
    private let clickFeedback = UIImpactFeedbackGenerator(style: .rigid)

    /// Milliseconds on the touch clock (`UIEvent.timestamp`, seconds since boot).
    var now: () -> Int64 = { Int64(ProcessInfo.processInfo.systemUptime * 1000) } {
        didSet { cursorLayer.now = now }
    }

    private var videoW = 0
    private var videoH = 0
    private var sourceW = 0
    private var sourceH = 0
    private(set) var base = Box(left: 0, top: 0, width: 0, height: 0)
    private(set) var box = Box(left: 0, top: 0, width: 0, height: 0)
    var fill = false {
        didSet {
            box = Box(left: 0, top: 0, width: 0, height: 0)
            setNeedsLayout()
        }
    }

    /// Trackpad mode: the phone owns the pointer position (see `TrackpadCursor`).
    private let cursor = TrackpadCursor()
    private var movedThisTouch = false
    private var timer: DispatchSourceTimer?
    /// Fingers down, in landing order (Android's pointer indices).
    private var active: [TouchPoint] = []

    /// Zoom the view starts at; 1 = the whole desktop.
    var initialZoom: Float = 1
    /// Three-finger tap (the screen uses it to toggle the keyboard).
    var onThreeFingerTap: (() -> Void)?
    /// Called after every layout with the current zoom level and the 1:1 actual-pixels zoom.
    var onZoomChanged: ((Float, Float) -> Void)?

    /// Points at the bottom covered by a keyboard (0 = closed).
    private var keyboardInset: Float = 0
    /// Last caret reported by the PC while the keyboard is open (x, y, w, h).
    private var caret: [Int]?
    /// Height of the part of the view not covered by the keyboard.
    private var visibleH: Float {
        let h = Float(bounds.height)
        return keyboardInset <= 0 ? h : max(h - keyboardInset, h * 0.25)
    }

    var pointerMode: PointerMode {
        get { interpreter.mode }
        set {
            interpreter.reset()
            interpreter.mode = newValue
            cursorLayer.trackpad = newValue == .trackpad
        }
    }

    func setSensitivity(_ sens: Float, _ accel: Float) {
        interpreter.sensitivity = sens
        interpreter.acceleration = accel
    }

    init(input: RemoteInput) {
        self.input = input
        keyboard = KeyInputView(input: input)
        privacy = PrivacyGuard(input: input)
        super.init(frame: .zero)
        forwarder.target = self
        backgroundColor = .black
        clipsToBounds = true
        isMultipleTouchEnabled = true

        videoView.displayLayer.videoGravity = .resize
        videoView.isUserInteractionEnabled = false
        stillView.frame = videoView.bounds
        stillView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        stillView.contentMode = .scaleToFill
        stillView.isHidden = true
        videoView.addSubview(stillView)
        addSubview(videoView)
        layer.addSublayer(cursorLayer)

        captureNotice.text = "Screen recording is on — the PC screen is hidden"
        captureNotice.textColor = Palette.mist
        captureNotice.font = .systemFont(ofSize: 16, weight: .medium)
        captureNotice.numberOfLines = 0
        captureNotice.textAlignment = .center
        captureNotice.isHidden = true
        addSubview(captureNotice)

        keyboard.onHardwareKey = { [weak self] press, down in
            self?.handleHardwareKey(press, down: down) ?? false
        }
        addSubview(keyboard)

        privacy.host = self
        privacy.onCaptureChanged = { [weak self] hidden in self?.setCaptureHidden(hidden) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    deinit {
        timer?.cancel()
    }

    /// The layer the session decodes into (`RemoteSession.attachLayer`).
    var displayLayer: AVSampleBufferDisplayLayer { videoView.displayLayer }

    /// The screen is going away: no timers, no held buttons, no cover.
    func shutdown() {
        timer?.cancel()
        timer = nil
        interpreter.reset()
        active.removeAll()
        privacy.stop()
        if keyboard.isFirstResponder { _ = keyboard.resignFirstResponder() }
    }

    /// A still picture in place of the video (UI tests: `-uiTestRemoteDemo`).
    func showStill(_ image: CGImage) {
        stillView.image = UIImage(cgImage: image)
        stillView.isHidden = false
    }

    private func setCaptureHidden(_ hidden: Bool) {
        videoView.isHidden = hidden
        cursorLayer.isHidden = hidden
        captureNotice.isHidden = !hidden
    }

    // MARK: - Layout

    override func layoutSubviews() {
        super.layoutSubviews()
        let w = Float(bounds.width)
        let h = Float(bounds.height)
        let newBase = videoW > 0
            ? Viewport.base(viewW: w, viewH: h, videoW: videoW, videoH: videoH, fill: fill)
            : Box(left: 0, top: 0, width: w, height: h)
        if newBase != base || box.width <= 0 {
            // Orientation/size change: keep the zoom level, re-clamp. A fresh
            // stream starts at the configured zoom.
            let z = box.width <= 0 || base.width <= 0 ? initialZoom : Viewport.zoomOf(box: box, base: base)
            base = newBase
            box = Viewport.clamp(
                box: Box(
                    left: base.left - (base.width * (z - 1)) / 2, top: base.top - (base.height * (z - 1)) / 2,
                    width: base.width * z, height: base.height * z),
                base: base, viewW: w, viewH: h)
        }
        applyBox()
        captureNotice.frame = bounds.insetBy(dx: 32, dy: 32)
        keyboard.frame = CGRect(x: 0, y: 0, width: 1, height: 1)
        onZoomChanged?(Viewport.zoomOf(box: box, base: base), actualPixelsZoom)
    }

    /// One PC pixel per phone pixel (Android works in pixels, iOS in points).
    private var actualPixelsZoom: Float {
        let scale = Float(window?.screen.scale ?? UIScreen.main.scale)
        let px = Box(left: 0, top: 0, width: base.width * scale, height: base.height * scale)
        return Viewport.actualPixelsZoom(base: px, sourceW: sourceW)
    }

    private func applyBox() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        videoView.frame = CGRect(
            x: CGFloat(box.left), y: CGFloat(box.top), width: CGFloat(box.width), height: CGFloat(box.height))
        cursorLayer.frame = bounds
        CATransaction.commit()
        cursorLayer.box = box
        cursorLayer.scale = sourceW > 0 ? box.width / Float(sourceW) : 1
        cursorLayer.redraw()
    }

    private func relayoutBox(_ newBox: Box) {
        box = newBox
        // The cursor must see the new box at once: the follow camera reads
        // it on the next move, which can arrive before the layout pass.
        cursorLayer.box = newBox
        cursorLayer.scale = sourceW > 0 ? newBox.width / Float(sourceW) : 1
        cursorLayer.redraw()
        setNeedsLayout()
    }

    /// Current zoom (1 = whole desktop visible).
    var zoomLevel: Float { Viewport.zoomOf(box: box, base: base) }

    var zoomed: Bool { zoomLevel > 1.01 }

    /// Where zoom presets centre: the pointer if it is on screen, else the view centre.
    private func zoomFocus() -> (Float, Float) {
        let w = Float(bounds.width)
        let h = Float(bounds.height)
        if let p = cursorLayer.viewPosition(), (0...w).contains(p.0), (0...h).contains(p.1) { return p }
        return (w / 2, h / 2)
    }

    func zoomFit() { relayoutBox(base) }

    func zoomTo(_ level: Float) {
        let (fx, fy) = zoomFocus()
        relayoutBox(Viewport.zoomTo(
            box: box, base: base, fx: fx, fy: fy, target: level,
            viewW: Float(bounds.width), viewH: Float(bounds.height)))
    }

    /// One PC pixel per phone pixel.
    func zoomActualPixels() { zoomTo(actualPixelsZoom) }

    func resetZoom() { zoomFit() }

    // MARK: - Touch

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        feedBegan(points(touches), t: ms(touches, event))
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        feedMoved(points(touches), t: ms(touches, event))
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        feedEnded(points(touches), t: ms(touches, event))
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        feedCancelled()
    }

    private func points(_ touches: Set<UITouch>) -> [TouchPoint] {
        touches.sorted { $0.timestamp < $1.timestamp }.map { touch in
            let p = touch.location(in: self)
            return TouchPoint(id: ObjectIdentifier(touch), x: Float(p.x), y: Float(p.y))
        }
    }

    private func ms(_ touches: Set<UITouch>, _ event: UIEvent?) -> Int64 {
        Int64((event?.timestamp ?? touches.first?.timestamp ?? 0) * 1000)
    }

    /// Fingers landed (Android ACTION_DOWN / ACTION_POINTER_DOWN).
    func feedBegan(_ points: [TouchPoint], t: Int64) {
        for p in points {
            active.removeAll { $0.id == p.id }
            active.append(p)
            switch active.count {
            case 1:
                movedThisTouch = false
                interpreter.onDown(x: p.x, y: p.y, t: t)
            case 2:
                interpreter.onSecondDown(x0: active[0].x, y0: active[0].y, x1: active[1].x, y1: active[1].y, t: t)
            case 3:
                interpreter.onThirdDown(t: t)
            default:
                break
            }
        }
        rearmTimer()
    }

    /// Fingers moved (ACTION_MOVE): `points` holds the ones that moved.
    func feedMoved(_ points: [TouchPoint], t: Int64) {
        for p in points {
            if let i = active.firstIndex(where: { $0.id == p.id }) { active[i] = p }
        }
        if active.count >= 2 {
            interpreter.onTwoMove(x0: active[0].x, y0: active[0].y, x1: active[1].x, y1: active[1].y, t: t)
        } else if let p = active.first {
            interpreter.onMove(x: p.x, y: p.y, t: t)
        }
        rearmTimer()
    }

    /// Fingers lifted (ACTION_POINTER_UP / ACTION_UP).
    func feedEnded(_ points: [TouchPoint], t: Int64) {
        for p in points {
            guard let i = active.firstIndex(where: { $0.id == p.id }) else { continue }
            switch active.count {
            case 3:
                interpreter.onThirdUp(t: t)
            case 2:
                interpreter.onSecondUp(t: t)
            case 1:
                interpreter.onUp(x: p.x, y: p.y, t: t)
                // Pointer moves are datagrams; repeat the final position so
                // a lost last packet cannot leave the PC's pointer short.
                if movedThisTouch {
                    let (nx, ny) = cursor.normalized()
                    input.pointer(x: nx, y: ny)
                }
            default:
                break
            }
            active.remove(at: i)
        }
        rearmTimer()
    }

    /// The system took the touches (ACTION_CANCEL).
    func feedCancelled() {
        interpreter.onCancel()
        active.removeAll()
        rearmTimer()
    }

    /// The interpreter's long-press timer on a dispatch timer (no display link).
    private func rearmTimer() {
        let at = interpreter.timerAt
        if at == 0 {
            timer?.schedule(deadline: .distantFuture)
            return
        }
        let t: DispatchSourceTimer
        if let existing = timer {
            t = existing
        } else {
            t = DispatchSource.makeTimerSource(queue: .main)
            t.setEventHandler { [weak self] in self?.timerFired() }
            t.schedule(deadline: .distantFuture)
            t.resume()
            timer = t
        }
        let delay = max(0, at - now())
        t.schedule(deadline: .now() + .milliseconds(Int(delay)))
    }

    private func timerFired() {
        interpreter.onTimer(t: now())
        rearmTimer()
    }

    // MARK: - TouchActions

    func pointerTo(x: Float, y: Float) {
        let (nx, ny) = Viewport.toRemote(x: x, y: y, box: box)
        input.pointer(x: nx, y: ny)
        cursorLayer.localAt(nx, ny)
    }

    func pointerBy(dx: Float, dy: Float) {
        // Phone points → PC pixels at the un-zoomed scale, so the pointer
        // speed does not depend on the zoom level. The phone integrates the
        // motion and sends the absolute result (see TrackpadCursor).
        let k: Float = base.width > 0 && sourceW > 0 ? Float(sourceW) / base.width : 1
        let before = cursor.normalized()
        let p = cursor.moveBy(dxPc: dx * k, dyPc: dy * k, srcW: sourceW, srcH: sourceH, t: now())
        if p == before { return }
        movedThisTouch = true
        input.pointer(x: p.0, y: p.1)
        cursorLayer.localAt(p.0, p.1)
        followCursor()
    }

    /// Follow camera: when zoomed, the pointer moves freely inside the middle
    /// of the view; once it leaves that zone the view moves with it, so the
    /// pointer never runs off screen and the user never pans by hand.
    private func followCursor() {
        let vh = visibleH
        // With the keyboard open the picture may extend below the visible
        // part even at Fit, so follow there too.
        if zoomLevel <= 1.01 && box.height <= vh + 1 { return }
        guard let p = cursorLayer.viewPosition() else { return }
        let (cx, cy) = p
        let w = Float(bounds.width)
        let (dx, dy) = Viewport.followPan(px: cx, py: cy, viewW: w, viewH: vh, zone: Self.FOLLOW_ZONE)
        if dx != 0 || dy != 0 { relayoutBox(Viewport.pan(box: box, dx: dx, dy: dy, viewW: w, viewH: vh)) }
    }

    func button(button: Int, down: Bool, atX: Float, atY: Float) {
        if down { haptic() }
        if atX.isNaN {
            // Trackpad: click exactly where the phone shows the pointer. The
            // position travels with the click on the reliable stream.
            if pointerMode == .trackpad && sourceW > 0 {
                let (nx, ny) = cursor.normalized()
                input.button(button, down: down, x: nx, y: ny)
            } else {
                input.button(button, down: down)
            }
        } else {
            let (nx, ny) = Viewport.toRemote(x: atX, y: atY, box: box)
            input.button(button, down: down, x: nx, y: ny)
            cursorLayer.localAt(nx, ny)
        }
    }

    func scroll(dx: Int, dy: Int) { input.scroll(dx: dx, dy: dy) }

    func zoom(focusX: Float, focusY: Float, factor: Float) {
        relayoutBox(Viewport.zoomAround(
            box: box, base: base, fx: focusX, fy: focusY, factor: factor,
            viewW: Float(bounds.width), viewH: Float(bounds.height)))
    }

    func pan(dx: Float, dy: Float) {
        relayoutBox(Viewport.pan(box: box, dx: dx, dy: dy, viewW: Float(bounds.width), viewH: Float(bounds.height)))
    }

    func threeFingerTap() {
        haptic()
        onThreeFingerTap?()
    }

    func feedback(kind: Feedback) { haptic() }

    /// Click, right click and drag start (Android CLOCK_TICK / LONG_PRESS /
    /// CONTEXT_CLICK).
    private func haptic() {
        clickFeedback.impactOccurred()
        clickFeedback.prepare()
    }

    // MARK: - Keyboard overlay

    /// A keyboard (LANLink's compact one, or the phone's own plus the
    /// shortcut row) now covers `px` points at the bottom (0 = closed). The
    /// picture keeps its size; the view slides so the place being typed into
    /// stays visible above the keyboard. The PC is asked to report its text
    /// caret while the keyboard is open.
    func setKeyboardInset(_ px: CGFloat) {
        let wasOpen = keyboardInset > 0
        keyboardInset = max(0, Float(px))
        let open = keyboardInset > 0
        if open != wasOpen {
            input.setTextInput(open)
            if !open {
                caret = nil
                // Back to the full view: re-centre/re-clamp.
                relayoutBox(Viewport.clamp(box: box, base: base, viewW: Float(bounds.width), viewH: Float(bounds.height)))
                return
            }
        }
        if open { revealTypingSpot() }
    }

    func onCaret(visible: Bool, x: Int, y: Int, w: Int, h: Int) {
        if keyboardInset <= 0 { return }
        let next: [Int]? = visible ? [x, y, w, h] : nil
        if next == caret { return }
        caret = next
        revealTypingSpot()
    }

    /// View coordinates (x, top, bottom) of the caret, or of the pointer.
    private func typingSpot(_ b: Box) -> (x: Float, top: Float, bottom: Float) {
        if let c = caret {
            let (x, top) = Viewport.toView(nx: c[0], ny: c[1], box: b)
            let (_, bottom) = Viewport.toView(nx: c[0], ny: min(c[1] + c[3], 65535), box: b)
            return (x, top, max(bottom, top + 8 * density))
        }
        let (nx, ny) = cursorLayer.normalized()
        let (x, y) = Viewport.toView(nx: nx, ny: ny, box: b)
        return (x, y - 16 * density, y + 16 * density)
    }

    private func revealTypingSpot() {
        if keyboardInset <= 0 || box.width <= 0 || base.width <= 0 { return }
        let w = Float(bounds.width)
        let vh = visibleH
        var b = box
        var spot = typingSpot(b)
        // Text should be legible while typing.
        if Viewport.zoomOf(box: b, base: base) < Self.TYPING_ZOOM {
            b = Viewport.zoomTo(
                box: b, base: base, fx: spot.x, fy: (spot.top + spot.bottom) / 2, target: Self.TYPING_ZOOM,
                viewW: w, viewH: vh)
            spot = typingSpot(b)
        }
        let (dx, dy) = Viewport.revealPan(x: spot.x, top: spot.top, bottom: spot.bottom, viewW: w, visibleH: vh)
        b = Viewport.pan(box: b, dx: dx, dy: dy, viewW: w, viewH: vh)
        if b != box { relayoutBox(b) }
    }

    // MARK: - CursorSink

    func onCursorShape(bitmap: CGImage, hotX: Int, hotY: Int) {
        cursorLayer.setShape(bitmap, hotX: hotX, hotY: hotY)
    }

    func onCursorPosition(nx: Int, ny: Int, visible: Bool) {
        cursor.hostReport(nx: nx, ny: ny, t: now())
        cursorLayer.hostAt(nx, ny, visible: visible)
    }

    func onVideoGeometry(width: Int, height: Int, sourceWidth: Int, sourceHeight: Int) {
        videoW = width
        videoH = height
        sourceW = sourceWidth
        sourceH = sourceHeight
        box = Box(left: 0, top: 0, width: 0, height: 0)
        setNeedsLayout()
        // A new or restarted stream: the PC's session starts without caret
        // reporting, so ask again if the keyboard is open.
        if keyboardInset > 0 { input.setTextInput(true) }
    }

    // MARK: - Keyboard

    func showKeyboard() { _ = keyboard.becomeFirstResponder() }

    func hideKeyboard() {
        _ = keyboard.resignFirstResponder()
        // Back to this view, so physical keys still reach the PC.
        if window != nil { _ = becomeFirstResponder() }
    }

    /// Whether the phone's on-screen keyboard is attached right now.
    var keyboardVisible: Bool { keyboard.isFirstResponder }

    func toggleKeyboard() {
        if keyboardVisible { hideKeyboard() } else { showKeyboard() }
    }

    override var canBecomeFirstResponder: Bool { true }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window != nil && !keyboard.isFirstResponder { _ = becomeFirstResponder() }
    }

    // Physical keyboards (Bluetooth/USB) reach the first responder; route
    // them to the PC. Keyboard text arrives through `KeyInputView` instead.
    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let rest = presses.filter { !handleHardwareKey($0, down: true) }
        if !rest.isEmpty { super.pressesBegan(rest, with: event) }
    }

    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let rest = presses.filter { !handleHardwareKey($0, down: false) }
        if !rest.isEmpty { super.pressesEnded(rest, with: event) }
    }

    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let rest = presses.filter { !handleHardwareKey($0, down: false) }
        if !rest.isEmpty { super.pressesCancelled(rest, with: event) }
    }

    /// Physical keyboard keys (Bluetooth/USB), forwarded as key presses.
    func handleHardwareKey(_ press: UIPress, down: Bool) -> Bool {
        guard let key = press.key, let vk = KeyMap.vkFor(keyCode: key.keyCode.rawValue) else { return false }
        input.key(vk, down: down)
        return true
    }
}

/// The view behind the video: its layer is the display layer.
private final class VideoLayerView: UIView {
    override class var layerClass: AnyClass { AVSampleBufferDisplayLayer.self }

    // swiftlint:disable:next force_cast
    var displayLayer: AVSampleBufferDisplayLayer { layer as! AVSampleBufferDisplayLayer }
}

/// Hands the interpreter's actions to the view without owning it.
///
/// `TouchInterpreter` keeps its `actions` strongly and `TouchActions` is not
/// class-bound (Task 6 finding M6). If the view passed itself, view →
/// interpreter → view would be a retain cycle and every remote screen
/// would leak its view. This forwarder holds the view weakly.
private final class TouchForwarder: TouchActions {
    weak var target: RemoteView?

    func pointerTo(x: Float, y: Float) { target?.pointerTo(x: x, y: y) }
    func pointerBy(dx: Float, dy: Float) { target?.pointerBy(dx: dx, dy: dy) }
    func button(button: Int, down: Bool, atX: Float, atY: Float) {
        target?.button(button: button, down: down, atX: atX, atY: atY)
    }
    func scroll(dx: Int, dy: Int) { target?.scroll(dx: dx, dy: dy) }
    func zoom(focusX: Float, focusY: Float, factor: Float) { target?.zoom(focusX: focusX, focusY: focusY, factor: factor) }
    func pan(dx: Float, dy: Float) { target?.pan(dx: dx, dy: dy) }
    func feedback(kind: Feedback) { target?.feedback(kind: kind) }
    func threeFingerTap() { target?.threeFingerTap() }
}
