import QuartzCore
import UIKit

/// Draws the PC's pointer on the phone (twin of `CursorView` in
/// RemoteView.kt). In direct-touch mode the pointer jumps to the finger at
/// once (local feedback) and the PC's own report takes over again shortly
/// after; in trackpad mode motion is predicted locally and reconciled with
/// the PC's position.
///
/// A layer that only moves a small sublayer, so pointer motion never
/// redraws the screen: the host cursor shape (from `Bridge.cursorPixels`) is
/// the sublayer's contents, and the default arrow is a shape layer.
final class CursorLayer: CALayer {
    /// The picture's box in view points.
    var box = Box(left: 0, top: 0, width: 0, height: 0)
    /// Points per PC pixel at the current zoom.
    var scale: Float = 1
    var trackpad = false
    /// Milliseconds on the touch clock (injected by the view).
    var now: () -> Int64 = { Int64(ProcessInfo.processInfo.systemUptime * 1000) }

    private var shape: CGImage?
    private var shownShape: CGImage?
    private var hotX = 0
    private var hotY = 0
    private var nx = 32768
    private var ny = 32768
    private var cursorVisible = false
    private var lastLocal = Int64.min / 2
    private let bitmapLayer = CALayer()
    private let arrowLayer = CAShapeLayer()

    override init() {
        super.init()
        setUp()
    }

    /// Core Animation copies layers for presentation; copy the state too.
    override init(layer: Any) {
        super.init(layer: layer)
        if let other = layer as? CursorLayer {
            box = other.box
            scale = other.scale
            trackpad = other.trackpad
            now = other.now
            shape = other.shape
            hotX = other.hotX
            hotY = other.hotY
            nx = other.nx
            ny = other.ny
            cursorVisible = other.cursorVisible
            lastLocal = other.lastLocal
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    private func setUp() {
        // The Android arrow, in points (Kotlin draws it in dp × density).
        let arrow = UIBezierPath()
        arrow.move(to: CGPoint(x: 0, y: 0))
        arrow.addLine(to: CGPoint(x: 0, y: 17))
        arrow.addLine(to: CGPoint(x: 4.5, y: 13))
        arrow.addLine(to: CGPoint(x: 8, y: 20))
        arrow.addLine(to: CGPoint(x: 10.5, y: 19))
        arrow.addLine(to: CGPoint(x: 7.5, y: 12))
        arrow.addLine(to: CGPoint(x: 13, y: 12))
        arrow.close()
        arrowLayer.path = arrow.cgPath
        arrowLayer.fillColor = UIColor.white.cgColor
        arrowLayer.strokeColor = UIColor.black.cgColor
        arrowLayer.lineWidth = 1.5
        arrowLayer.lineJoin = .round
        arrowLayer.bounds = CGRect(x: 0, y: 0, width: 14, height: 21)
        arrowLayer.anchorPoint = .zero
        bitmapLayer.anchorPoint = .zero
        for l in [bitmapLayer, arrowLayer] as [CALayer] {
            l.isHidden = true
            l.actions = ["position": NSNull(), "bounds": NSNull(), "contents": NSNull(), "hidden": NSNull()]
            addSublayer(l)
        }
        actions = ["position": NSNull(), "bounds": NSNull(), "hidden": NSNull()]
    }

    func setShape(_ image: CGImage, hotX hx: Int, hotY hy: Int) {
        shape = image
        hotX = hx
        hotY = hy
        redraw()
    }

    func hostAt(_ x: Int, _ y: Int, visible: Bool) {
        cursorVisible = visible
        // Do not fight the user's finger: local feedback wins briefly.
        if now() - lastLocal >= 250 {
            nx = x
            ny = y
        }
        redraw()
    }

    func localAt(_ x: Int, _ y: Int) {
        nx = x
        ny = y
        cursorVisible = true
        lastLocal = now()
        redraw()
    }

    func normalized() -> (Int, Int) { (nx, ny) }

    func viewPosition() -> (Float, Float)? {
        box.width > 0 ? Viewport.toView(nx: nx, ny: ny, box: box) : nil
    }

    /// Moves the pointer sublayer to the current position.
    func redraw() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        defer { CATransaction.commit() }
        guard cursorVisible, box.width > 0 else {
            bitmapLayer.isHidden = true
            arrowLayer.isHidden = true
            return
        }
        let (x, y) = Viewport.toView(nx: nx, ny: ny, box: box)
        if let b = shape {
            // The PC's size, but never smaller than comfortably visible on a
            // phone (Kotlin: 0.6 × density / 2 px per PC pixel = 0.3 pt).
            let s = CGFloat(max(scale, 0.3))
            if shownShape !== b {
                bitmapLayer.contents = b
                shownShape = b
            }
            bitmapLayer.frame = CGRect(
                x: CGFloat(x) - CGFloat(hotX) * s, y: CGFloat(y) - CGFloat(hotY) * s,
                width: CGFloat(b.width) * s, height: CGFloat(b.height) * s)
            bitmapLayer.isHidden = false
            arrowLayer.isHidden = true
        } else {
            arrowLayer.position = CGPoint(x: CGFloat(x), y: CGFloat(y))
            arrowLayer.isHidden = false
            bitmapLayer.isHidden = true
        }
    }
}
