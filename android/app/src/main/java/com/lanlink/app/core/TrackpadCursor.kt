package com.lanlink.app.core

import kotlin.math.roundToInt

/**
 * The pointer position in trackpad mode, owned by the phone.
 *
 * Earlier builds sent relative moves. Windows then applied its own pointer
 * speed and "Enhance pointer precision" curve on top of ours, and every lost
 * datagram was a lost piece of motion, so the PC's pointer drifted away from
 * the one drawn on the phone and clicks landed somewhere else. Now the phone
 * integrates finger motion into an absolute position and sends that:
 * idempotent (a lost packet is corrected by the next one), unaffected by the
 * PC's mouse settings, and the pointer drawn on the phone is exactly where the
 * PC will click.
 *
 * Positions are fractions of the captured display (0..1); [normalized] gives
 * the protocol's 0..65535 form.
 */
class TrackpadCursor {
    companion object {
        /**
         * After the finger last moved the pointer, ignore the PC's reports for
         * this long: they may describe the position before our latest move
         * arrived. After that the PC is the truth (its own mouse, or an app
         * that moved the pointer).
         */
        const val HOLD_MS = 250L
    }

    var fx = 0.5f
        private set
    var fy = 0.5f
        private set
    private var lastLocalT = Long.MIN_VALUE / 2

    /** Move by PC pixels; returns the new position in protocol units. */
    fun moveBy(dxPc: Float, dyPc: Float, srcW: Int, srcH: Int, t: Long): Pair<Int, Int> {
        if (srcW > 1) fx = (fx + dxPc / (srcW - 1)).coerceIn(0f, 1f)
        if (srcH > 1) fy = (fy + dyPc / (srcH - 1)).coerceIn(0f, 1f)
        lastLocalT = t
        return normalized()
    }

    /** The PC reported its pointer. Returns whether it was adopted. */
    fun hostReport(nx: Int, ny: Int, t: Long): Boolean {
        if (t - lastLocalT < HOLD_MS) return false
        fx = nx.coerceIn(0, 65535) / 65535f
        fy = ny.coerceIn(0, 65535) / 65535f
        return true
    }

    fun normalized(): Pair<Int, Int> = (fx * 65535f).roundToInt() to (fy * 65535f).roundToInt()
}
