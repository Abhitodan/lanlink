package com.lanlink.app.core

import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/** Axis-aligned rectangle in view pixels (floats; may extend past the view). */
data class Box(val left: Float, val top: Float, val width: Float, val height: Float) {
    val right get() = left + width
    val bottom get() = top + height
}

/**
 * Where the remote picture sits on the phone screen, and conversions between
 * phone coordinates and LANLink's normalized remote coordinates (0..65535
 * across the captured display).
 *
 * The *base* box fits the picture into the view preserving aspect ratio
 * (letterboxing) or fills it (cropping). Zoom scales the box around a focal
 * point; pan moves it. Both are clamped so the picture never drifts away:
 * when larger than the view it always covers the view, when smaller it is
 * centered.
 */
object Viewport {
    const val MAX_ZOOM = 6f

    fun base(viewW: Float, viewH: Float, videoW: Int, videoH: Int, fill: Boolean): Box {
        if (viewW <= 0f || viewH <= 0f || videoW <= 0 || videoH <= 0) return Box(0f, 0f, viewW, viewH)
        val sx = viewW / videoW
        val sy = viewH / videoH
        val s = if (fill) max(sx, sy) else min(sx, sy)
        val w = videoW * s
        val h = videoH * s
        return Box((viewW - w) / 2f, (viewH - h) / 2f, w, h)
    }

    /** Zoom factor of [box] relative to [base]. */
    fun zoomOf(box: Box, base: Box): Float = if (base.width > 0f) box.width / base.width else 1f

    fun clamp(box: Box, base: Box, viewW: Float, viewH: Float): Box {
        val z = zoomOf(box, base).coerceIn(1f, MAX_ZOOM)
        val w = base.width * z
        val h = base.height * z
        fun axis(pos: Float, size: Float, view: Float): Float =
            if (size <= view) (view - size) / 2f else pos.coerceIn(view - size, 0f)
        // Keep the box centered on the same content point when zoom was clamped.
        val cx = box.left + box.width / 2f
        val cy = box.top + box.height / 2f
        return Box(axis(cx - w / 2f, w, viewW), axis(cy - h / 2f, h, viewH), w, h)
    }

    /** Scale [box] by [factor] around ([fx],[fy]), then clamp. */
    fun zoomAround(box: Box, base: Box, fx: Float, fy: Float, factor: Float, viewW: Float, viewH: Float): Box {
        val target = (zoomOf(box, base) * factor).coerceIn(1f, MAX_ZOOM)
        val f = target / zoomOf(box, base)
        val scaled = Box(
            fx - (fx - box.left) * f,
            fy - (fy - box.top) * f,
            box.width * f,
            box.height * f,
        )
        return clampPosition(scaled, viewW, viewH)
    }

    fun pan(box: Box, dx: Float, dy: Float, viewW: Float, viewH: Float): Box =
        clampPosition(box.copy(left = box.left + dx, top = box.top + dy), viewW, viewH)

    private fun clampPosition(box: Box, viewW: Float, viewH: Float): Box {
        fun axis(pos: Float, size: Float, view: Float): Float =
            if (size <= view) (view - size) / 2f else pos.coerceIn(view - size, 0f)
        return box.copy(left = axis(box.left, box.width, viewW), top = axis(box.top, box.height, viewH))
    }

    /** Scale [box] to an absolute zoom of [target] around ([fx],[fy]). */
    fun zoomTo(box: Box, base: Box, fx: Float, fy: Float, target: Float, viewW: Float, viewH: Float): Box {
        val current = zoomOf(box, base)
        if (current <= 0f) return box
        return zoomAround(box, base, fx, fy, target / current, viewW, viewH)
    }

    /**
     * Zoom at which one PC pixel covers one phone pixel ("actual pixels"),
     * never below 1 (fit).
     */
    fun actualPixelsZoom(base: Box, sourceW: Int): Float =
        if (base.width > 0f && sourceW > 0) max(1f, sourceW / base.width) else 1f

    /**
     * Follow camera: how far to pan so the point ([px],[py]) sits inside the
     * central dead zone of the view. The zone spans [zone] of each axis
     * (0.6 = the middle 60 %); inside it the camera stays still, outside it
     * the view moves just enough to bring the point back to the zone edge.
     */
    fun followPan(px: Float, py: Float, viewW: Float, viewH: Float, zone: Float): Pair<Float, Float> {
        val mx = viewW * (1f - zone) / 2f
        val my = viewH * (1f - zone) / 2f
        val dx = when {
            px < mx -> mx - px
            px > viewW - mx -> (viewW - mx) - px
            else -> 0f
        }
        val dy = when {
            py < my -> my - py
            py > viewH - my -> (viewH - my) - py
            else -> 0f
        }
        return dx to dy
    }

    /** Where the bottom of the typing spot is placed in the visible region. */
    const val REVEAL_AT = 0.75f

    /**
     * Keyboard open: how far to pan so the typing spot (x, and the span
     * [top]..[bottom]) is comfortably visible in the region above the
     * keyboard ([viewW] × [visibleH]). Inside the comfortable band (15–85 %
     * vertically, 10–90 % horizontally) nothing moves, so typing along a
     * line does not shake the view. Outside it, the spot's bottom goes to
     * [REVEAL_AT] of the visible height (close to the keyboard, leaving the
     * text above it in view), and x goes to 40 % / 60 % of the width.
     */
    fun revealPan(x: Float, top: Float, bottom: Float, viewW: Float, visibleH: Float): Pair<Float, Float> {
        val dy = if (top < visibleH * 0.15f || bottom > visibleH * 0.85f) visibleH * REVEAL_AT - bottom else 0f
        val dx = when {
            x < viewW * 0.1f -> viewW * 0.4f - x
            x > viewW * 0.9f -> viewW * 0.6f - x
            else -> 0f
        }
        return dx to dy
    }

    /** Phone point → normalized remote point, clamped to the picture edges. */
    fun toRemote(x: Float, y: Float, box: Box): Pair<Int, Int> {
        if (box.width <= 1f || box.height <= 1f) return 0 to 0
        val nx = ((x - box.left) / (box.width - 1f)).coerceIn(0f, 1f)
        val ny = ((y - box.top) / (box.height - 1f)).coerceIn(0f, 1f)
        return (nx * 65535f).roundToInt() to (ny * 65535f).roundToInt()
    }

    /** Normalized remote point → phone point. */
    fun toView(nx: Int, ny: Int, box: Box): Pair<Float, Float> =
        (box.left + nx / 65535f * (box.width - 1f)) to (box.top + ny / 65535f * (box.height - 1f))

    /** Whether the phone point lies on the picture (not on letterbox bars). */
    fun contains(x: Float, y: Float, box: Box): Boolean =
        x >= box.left && x < box.right && y >= box.top && y < box.bottom
}
