import Foundation

/// The pointer position in trackpad mode, owned by the phone.
///
/// Earlier builds sent relative moves. Windows then applied its own pointer
/// speed and "Enhance pointer precision" curve on top of ours, and every lost
/// datagram was a lost piece of motion, so the PC's pointer drifted away from
/// the one drawn on the phone and clicks landed somewhere else. Now the phone
/// integrates finger motion into an absolute position and sends that:
/// idempotent (a lost packet is corrected by the next one), unaffected by the
/// PC's mouse settings, and the pointer drawn on the phone is exactly where the
/// PC will click.
///
/// Positions are fractions of the captured display (0..1); [normalized] gives
/// the protocol's 0..65535 form.
final class TrackpadCursor {
    /// After the finger last moved the pointer, ignore the PC's reports for
    /// this long: they may describe the position before our latest move
    /// arrived. After that the PC is the truth (its own mouse, or an app
    /// that moved the pointer).
    static let HOLD_MS: Int64 = 250

    private(set) var fx: Float = 0.5
    private(set) var fy: Float = 0.5
    private var lastLocalT: Int64 = Int64.min / 2

    /// Move by PC pixels; returns the new position in protocol units.
    @discardableResult
    func moveBy(dxPc: Float, dyPc: Float, srcW: Int, srcH: Int, t: Int64) -> (Int, Int) {
        if srcW > 1 { fx = min(max(fx + dxPc / Float(srcW - 1), 0), 1) }
        if srcH > 1 { fy = min(max(fy + dyPc / Float(srcH - 1), 0), 1) }
        lastLocalT = t
        return normalized()
    }

    /// The PC reported its pointer. Returns whether it was adopted.
    @discardableResult
    func hostReport(nx: Int, ny: Int, t: Int64) -> Bool {
        if t - lastLocalT < TrackpadCursor.HOLD_MS { return false }
        fx = Float(min(max(nx, 0), 65535)) / 65535
        fy = Float(min(max(ny, 0), 65535)) / 65535
        return true
    }

    func normalized() -> (Int, Int) {
        (Viewport.roundHalfUp(fx * 65535), Viewport.roundHalfUp(fy * 65535))
    }
}
