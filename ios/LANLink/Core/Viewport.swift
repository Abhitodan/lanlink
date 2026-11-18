import Foundation

/// Axis-aligned rectangle in view pixels (floats; may extend past the view).
struct Box: Equatable {
    var left: Float
    var top: Float
    var width: Float
    var height: Float

    var right: Float { left + width }
    var bottom: Float { top + height }
}

/// Where the remote picture sits on the phone screen, and conversions between
/// phone coordinates and LANLink's normalized remote coordinates (0..65535
/// across the captured display).
///
/// The *base* box fits the picture into the view preserving aspect ratio
/// (letterboxing) or fills it (cropping). Zoom scales the box around a focal
/// point; pan moves it. Both are clamped so the picture never drifts away:
/// when larger than the view it always covers the view, when smaller it is
/// centered.
enum Viewport {
    static let MAX_ZOOM: Float = 6

    static func base(viewW: Float, viewH: Float, videoW: Int, videoH: Int, fill: Bool) -> Box {
        if viewW <= 0 || viewH <= 0 || videoW <= 0 || videoH <= 0 {
            return Box(left: 0, top: 0, width: viewW, height: viewH)
        }
        let sx = viewW / Float(videoW)
        let sy = viewH / Float(videoH)
        let s = fill ? max(sx, sy) : min(sx, sy)
        let w = Float(videoW) * s
        let h = Float(videoH) * s
        return Box(left: (viewW - w) / 2, top: (viewH - h) / 2, width: w, height: h)
    }

    /// Zoom factor of [box] relative to [base].
    static func zoomOf(box: Box, base: Box) -> Float {
        base.width > 0 ? box.width / base.width : 1
    }

    private static func coerce(_ v: Float, _ lo: Float, _ hi: Float) -> Float {
        min(max(v, lo), hi)
    }

    private static func axis(_ pos: Float, _ size: Float, _ view: Float) -> Float {
        size <= view ? (view - size) / 2 : coerce(pos, view - size, 0)
    }

    static func clamp(box: Box, base: Box, viewW: Float, viewH: Float) -> Box {
        let z = coerce(zoomOf(box: box, base: base), 1, MAX_ZOOM)
        let w = base.width * z
        let h = base.height * z
        // Keep the box centered on the same content point when zoom was clamped.
        let cx = box.left + box.width / 2
        let cy = box.top + box.height / 2
        return Box(left: axis(cx - w / 2, w, viewW), top: axis(cy - h / 2, h, viewH), width: w, height: h)
    }

    /// Scale [box] by [factor] around ([fx],[fy]), then clamp.
    static func zoomAround(box: Box, base: Box, fx: Float, fy: Float, factor: Float, viewW: Float, viewH: Float) -> Box {
        let target = coerce(zoomOf(box: box, base: base) * factor, 1, MAX_ZOOM)
        let f = target / zoomOf(box: box, base: base)
        let scaled = Box(
            left: fx - (fx - box.left) * f,
            top: fy - (fy - box.top) * f,
            width: box.width * f,
            height: box.height * f
        )
        return clampPosition(box: scaled, viewW: viewW, viewH: viewH)
    }

    static func pan(box: Box, dx: Float, dy: Float, viewW: Float, viewH: Float) -> Box {
        var moved = box
        moved.left = box.left + dx
        moved.top = box.top + dy
        return clampPosition(box: moved, viewW: viewW, viewH: viewH)
    }

    private static func clampPosition(box: Box, viewW: Float, viewH: Float) -> Box {
        var out = box
        out.left = axis(box.left, box.width, viewW)
        out.top = axis(box.top, box.height, viewH)
        return out
    }

    /// Scale [box] to an absolute zoom of [target] around ([fx],[fy]).
    static func zoomTo(box: Box, base: Box, fx: Float, fy: Float, target: Float, viewW: Float, viewH: Float) -> Box {
        let current = zoomOf(box: box, base: base)
        if current <= 0 { return box }
        return zoomAround(box: box, base: base, fx: fx, fy: fy, factor: target / current, viewW: viewW, viewH: viewH)
    }

    /// Zoom at which one PC pixel covers one phone pixel ("actual pixels"),
    /// never below 1 (fit).
    static func actualPixelsZoom(base: Box, sourceW: Int) -> Float {
        base.width > 0 && sourceW > 0 ? max(1, Float(sourceW) / base.width) : 1
    }

    /// Follow camera: how far to pan so the point ([px],[py]) sits inside the
    /// central dead zone of the view. The zone spans [zone] of each axis
    /// (0.6 = the middle 60 %); inside it the camera stays still, outside it
    /// the view moves just enough to bring the point back to the zone edge.
    static func followPan(px: Float, py: Float, viewW: Float, viewH: Float, zone: Float) -> (Float, Float) {
        let mx = viewW * (1 - zone) / 2
        let my = viewH * (1 - zone) / 2
        let dx: Float
        if px < mx { dx = mx - px }
        else if px > viewW - mx { dx = (viewW - mx) - px }
        else { dx = 0 }
        let dy: Float
        if py < my { dy = my - py }
        else if py > viewH - my { dy = (viewH - my) - py }
        else { dy = 0 }
        return (dx, dy)
    }

    /// Where the bottom of the typing spot is placed in the visible region.
    static let REVEAL_AT: Float = 0.75

    /// Keyboard open: how far to pan so the typing spot (x, and the span
    /// [top]..[bottom]) is comfortably visible in the region above the
    /// keyboard ([viewW] x [visibleH]). Inside the comfortable band (15-85 %
    /// vertically, 10-90 % horizontally) nothing moves, so typing along a
    /// line does not shake the view. Outside it, the spot's bottom goes to
    /// [REVEAL_AT] of the visible height (close to the keyboard, leaving the
    /// text above it in view), and x goes to 40 % / 60 % of the width.
    static func revealPan(x: Float, top: Float, bottom: Float, viewW: Float, visibleH: Float) -> (Float, Float) {
        let dy: Float = (top < visibleH * 0.15 || bottom > visibleH * 0.85) ? visibleH * REVEAL_AT - bottom : 0
        let dx: Float
        if x < viewW * 0.1 { dx = viewW * 0.4 - x }
        else if x > viewW * 0.9 { dx = viewW * 0.6 - x }
        else { dx = 0 }
        return (dx, dy)
    }

    /// Phone point -> normalized remote point, clamped to the picture edges.
    static func toRemote(x: Float, y: Float, box: Box) -> (Int, Int) {
        if box.width <= 1 || box.height <= 1 { return (0, 0) }
        let nx = coerce((x - box.left) / (box.width - 1), 0, 1)
        let ny = coerce((y - box.top) / (box.height - 1), 0, 1)
        return (roundHalfUp(nx * 65535), roundHalfUp(ny * 65535))
    }

    /// Normalized remote point -> phone point.
    static func toView(nx: Int, ny: Int, box: Box) -> (Float, Float) {
        (box.left + Float(nx) / 65535 * (box.width - 1), box.top + Float(ny) / 65535 * (box.height - 1))
    }

    /// Whether the phone point lies on the picture (not on letterbox bars).
    static func contains(x: Float, y: Float, box: Box) -> Bool {
        x >= box.left && x < box.right && y >= box.top && y < box.bottom
    }

    /// Kotlin `roundToInt` semantics (half rounds up), shared with TrackpadCursor.
    static func roundHalfUp(_ v: Float) -> Int {
        Int((v + 0.5).rounded(.down))
    }
}
