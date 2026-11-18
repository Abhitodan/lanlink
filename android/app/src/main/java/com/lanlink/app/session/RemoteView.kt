package com.lanlink.app.session

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.os.Build
import android.os.SystemClock
import android.util.Log
import android.text.InputType
import android.view.HapticFeedbackConstants
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.ViewConfiguration
import android.view.WindowInsets
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import android.widget.FrameLayout
import com.lanlink.app.core.Box
import com.lanlink.app.core.Feedback
import com.lanlink.app.core.KeyMap
import com.lanlink.app.core.PointerMode
import com.lanlink.app.core.TouchActions
import com.lanlink.app.core.TouchInterpreter
import com.lanlink.app.core.TrackpadCursor
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import com.lanlink.app.core.Viewport
import kotlin.math.roundToInt

/**
 * The remote desktop surface: video, local cursor, touch and keyboard.
 *
 * Plain Android views on purpose: video frames and cursor motion must never
 * trigger Compose recomposition. The video goes to a [SurfaceView] whose
 * layout box implements letterboxing, zoom and pan; the cursor is a light
 * overlay view that redraws only itself.
 */
@SuppressLint("ViewConstructor")
class RemoteView(context: Context, private val session: RemoteSession) :
    FrameLayout(context), SurfaceHolder.Callback, TouchActions, CursorSink {

    private val surfaceView = SurfaceView(context)
    private val cursorView = CursorView(context)
    val keyboard = KeyboardInputView(context, session)
    private val density = resources.displayMetrics.density
    private val interpreter = TouchInterpreter(
        this,
        slop = ViewConfiguration.get(context).scaledTouchSlop.toFloat(),
        scrollStepPx = 28f * density,
        density = density,
    )
    private var videoW = 0
    private var videoH = 0
    private var sourceW = 0
    private var sourceH = 0
    private var base = Box(0f, 0f, 0f, 0f)
    private var box = base
    var fill = false
        set(v) { field = v; box = Box(0f, 0f, 0f, 0f); requestLayout() }
    /** Trackpad mode: the phone owns the pointer position (see [TrackpadCursor]). */
    private val cursor = TrackpadCursor()
    private var movedThisTouch = false
    private val timer = Runnable { interpreter.onTimer(SystemClock.uptimeMillis()) }

    /** Zoom the view starts at; 1 = the whole desktop. */
    var initialZoom = 1f
    /** Three-finger tap (the screen uses it to toggle the keyboard). */
    var onThreeFingerTap: (() -> Unit)? = null
    /** Called after every layout with the current zoom level. */
    var onZoomChanged: ((Float) -> Unit)? = null

    /** Pixels at the bottom covered by the phone keyboard (0 = closed). */
    private var keyboardInset = 0
    /** Last caret reported by the PC while the keyboard is open (x, y, w, h). */
    private var caret: IntArray? = null
    /** Height of the part of the view not covered by the keyboard. */
    private val visibleH: Float
        get() = if (keyboardInset <= 0) height.toFloat()
        else (height - keyboardInset).toFloat().coerceAtLeast(height * 0.25f)

    var pointerMode: PointerMode
        get() = interpreter.mode
        set(v) { interpreter.reset(); interpreter.mode = v; cursorView.trackpad = v == PointerMode.TRACKPAD }

    fun setSensitivity(sens: Float, accel: Float) {
        interpreter.sensitivity = sens
        interpreter.acceleration = accel
    }

    init {
        setBackgroundColor(Color.BLACK)
        clipChildren = true
        surfaceView.holder.addCallback(this)
        addView(surfaceView)
        addView(cursorView)
        addView(keyboard, LayoutParams(1, 1))
        isFocusable = true
        session.cursorSink = this
    }

    // ------------------------------------------------------------ layout

    override fun onLayout(changed: Boolean, l: Int, t: Int, r: Int, b: Int) {
        val w = (r - l).toFloat()
        val h = (b - t).toFloat()
        val newBase = if (videoW > 0) Viewport.base(w, h, videoW, videoH, fill) else Box(0f, 0f, w, h)
        if (newBase != base || box.width <= 0f) {
            // Orientation/size change: keep the zoom level, re-clamp. A fresh
            // stream starts at the configured zoom.
            val z = if (box.width <= 0f || base.width <= 0f) initialZoom else Viewport.zoomOf(box, base)
            base = newBase
            box = Viewport.clamp(
                Box(base.left - (base.width * (z - 1)) / 2f, base.top - (base.height * (z - 1)) / 2f, base.width * z, base.height * z),
                base, w, h,
            )
        }
        surfaceView.layout(box.left.roundToInt(), box.top.roundToInt(), box.right.roundToInt(), box.bottom.roundToInt())
        cursorView.layout(0, 0, r - l, b - t)
        keyboard.layout(0, 0, 1, 1)
        cursorView.box = box
        cursorView.scale = if (sourceW > 0) box.width / sourceW else 1f
        cursorView.invalidate()
        onZoomChanged?.invoke(Viewport.zoomOf(box, base))
    }

    private fun relayoutBox(newBox: Box) {
        box = newBox
        // The cursor overlay must see the new box at once: the follow camera
        // reads it on the next move, which can arrive before the layout pass.
        cursorView.box = newBox
        cursorView.scale = if (sourceW > 0) newBox.width / sourceW else 1f
        cursorView.invalidate()
        requestLayout()
    }

    /** Current zoom (1 = whole desktop visible). */
    val zoomLevel: Float get() = Viewport.zoomOf(box, base)

    /** Where zoom presets centre: the pointer if it is on screen, else the view centre. */
    private fun zoomFocus(): Pair<Float, Float> {
        val p = cursorView.viewPosition()
        return if (p != null && p.first in 0f..width.toFloat() && p.second in 0f..height.toFloat()) p
        else (width / 2f) to (height / 2f)
    }

    fun zoomFit() = relayoutBox(base)

    fun zoomTo(level: Float) {
        val (fx, fy) = zoomFocus()
        relayoutBox(Viewport.zoomTo(box, base, fx, fy, level, width.toFloat(), height.toFloat()))
    }

    /** One PC pixel per phone pixel. */
    fun zoomActualPixels() = zoomTo(Viewport.actualPixelsZoom(base, sourceW))

    // ------------------------------------------------------------ surface

    override fun surfaceCreated(holder: SurfaceHolder) = session.attachSurface(holder.surface)
    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {}
    override fun surfaceDestroyed(holder: SurfaceHolder) = session.detachSurface()

    // ------------------------------------------------------------ touch

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(e: MotionEvent): Boolean {
        val t = e.eventTime
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                keyboard.resetContext()
                movedThisTouch = false
                interpreter.onDown(e.x, e.y, t)
            }
            MotionEvent.ACTION_POINTER_DOWN -> when (e.pointerCount) {
                2 -> interpreter.onSecondDown(e.getX(0), e.getY(0), e.getX(1), e.getY(1), t)
                3 -> interpreter.onThirdDown(t)
            }
            MotionEvent.ACTION_MOVE -> {
                if (e.pointerCount >= 2) interpreter.onTwoMove(e.getX(0), e.getY(0), e.getX(1), e.getY(1), t)
                else interpreter.onMove(e.x, e.y, t)
            }
            MotionEvent.ACTION_POINTER_UP -> when (e.pointerCount) {
                2 -> interpreter.onSecondUp(t)
                3 -> interpreter.onThirdUp(t)
            }
            MotionEvent.ACTION_UP -> {
                interpreter.onUp(e.x, e.y, t)
                // Pointer moves are datagrams; repeat the final position so a
                // lost last packet cannot leave the PC's pointer short.
                if (movedThisTouch) cursor.normalized().let { (nx, ny) -> session.pointer(nx, ny) }
            }
            MotionEvent.ACTION_CANCEL -> interpreter.onCancel()
        }
        removeCallbacks(timer)
        if (interpreter.timerAt != 0L) postDelayed(timer, (interpreter.timerAt - SystemClock.uptimeMillis()).coerceAtLeast(0))
        return true
    }

    /** Mice and touchpads attached to the phone: hover moves the pointer. */
    override fun onGenericMotionEvent(e: MotionEvent): Boolean {
        if (e.actionMasked == MotionEvent.ACTION_HOVER_MOVE) {
            pointerTo(e.x, e.y)
            return true
        }
        if (e.actionMasked == MotionEvent.ACTION_SCROLL) {
            val v = e.getAxisValue(MotionEvent.AXIS_VSCROLL)
            val h = e.getAxisValue(MotionEvent.AXIS_HSCROLL)
            session.scroll((h * 120).roundToInt(), (v * 120).roundToInt())
            return true
        }
        return super.onGenericMotionEvent(e)
    }

    override fun pointerTo(x: Float, y: Float) {
        val (nx, ny) = Viewport.toRemote(x, y, box)
        session.pointer(nx, ny)
        cursorView.localAt(nx, ny)
    }

    override fun pointerBy(dx: Float, dy: Float) {
        // Phone pixels → PC pixels at the un-zoomed scale, so the pointer
        // speed does not depend on the zoom level. The phone integrates the
        // motion and sends the absolute result (see TrackpadCursor).
        val k = if (base.width > 0f && sourceW > 0) sourceW / base.width else 1f
        val before = cursor.normalized()
        val p = cursor.moveBy(dx * k, dy * k, sourceW, sourceH, SystemClock.uptimeMillis())
        if (p == before) return
        movedThisTouch = true
        session.pointer(p.first, p.second)
        cursorView.localAt(p.first, p.second)
        followCursor()
    }

    /**
     * Follow camera: when zoomed, the pointer moves freely inside the middle
     * of the view; once it leaves that zone the view moves with it, so the
     * pointer never runs off screen and the user never pans by hand.
     */
    private fun followCursor() {
        val vh = visibleH
        // With the keyboard open the picture may extend below the visible
        // part even at Fit, so follow there too.
        if (Viewport.zoomOf(box, base) <= 1.01f && box.height <= vh + 1f) return
        val (cx, cy) = cursorView.viewPosition() ?: return
        val (dx, dy) = Viewport.followPan(cx, cy, width.toFloat(), vh, FOLLOW_ZONE)
        if (dx != 0f || dy != 0f) relayoutBox(Viewport.pan(box, dx, dy, width.toFloat(), vh))
    }

    // ------------------------------------------------------------ keyboard overlay

    /**
     * A keyboard (LANLink's compact one, or the phone's own plus the
     * shortcut row) now covers [px] at the bottom (0 = closed). The
     * picture keeps its size; the view slides so the place being typed into
     * stays visible above the keyboard. The PC is asked to report its text
     * caret while the keyboard is open.
     */
    fun setKeyboardInset(px: Int) {
        val wasOpen = keyboardInset > 0
        keyboardInset = px.coerceAtLeast(0)
        val open = keyboardInset > 0
        if (open != wasOpen) {
            session.setTextInput(open)
            if (!open) {
                caret = null
                // Back to the full view: re-centre/re-clamp.
                relayoutBox(Viewport.clamp(box, base, width.toFloat(), height.toFloat()))
                return
            }
        }
        if (open) revealTypingSpot()
    }

    override fun onCaret(visible: Boolean, x: Int, y: Int, w: Int, h: Int) {
        if (keyboardInset <= 0) return
        val next = if (visible) intArrayOf(x, y, w, h) else null
        if (next?.contentEquals(caret) == true || (next == null && caret == null)) return
        caret = next
        revealTypingSpot()
    }

    /** View coordinates (x, top, bottom) of the caret, or of the pointer. */
    private fun typingSpot(b: Box): Triple<Float, Float, Float>? {
        val c = caret
        if (c != null) {
            val (x, top) = Viewport.toView(c[0], c[1], b)
            val (_, bottom) = Viewport.toView(c[0], (c[1] + c[3]).coerceAtMost(65535), b)
            return Triple(x, top, maxOf(bottom, top + 8 * density))
        }
        val (nx, ny) = cursorView.normalized()
        val (x, y) = Viewport.toView(nx, ny, b)
        return Triple(x, y - 16 * density, y + 16 * density)
    }

    private fun revealTypingSpot() {
        if (keyboardInset <= 0 || box.width <= 0f || base.width <= 0f) return
        val w = width.toFloat()
        val vh = visibleH
        var b = box
        var spot = typingSpot(b) ?: return
        // Text should be legible while typing.
        if (Viewport.zoomOf(b, base) < TYPING_ZOOM) {
            b = Viewport.zoomTo(b, base, spot.first, (spot.second + spot.third) / 2f, TYPING_ZOOM, w, vh)
            spot = typingSpot(b) ?: return
        }
        val (dx, dy) = Viewport.revealPan(spot.first, spot.second, spot.third, w, vh)
        b = Viewport.pan(b, dx, dy, w, vh)
        if (b != box) relayoutBox(b)
    }

    override fun button(button: Int, down: Boolean, atX: Float, atY: Float) {
        if (down) performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
        if (atX.isNaN()) {
            // Trackpad: click exactly where the phone shows the pointer. The
            // position travels with the click on the reliable stream.
            if (pointerMode == PointerMode.TRACKPAD && sourceW > 0) {
                val (nx, ny) = cursor.normalized()
                session.button(button, down, nx, ny)
            } else {
                session.button(button, down)
            }
        } else {
            val (nx, ny) = Viewport.toRemote(atX, atY, box)
            session.button(button, down, nx, ny)
            cursorView.localAt(nx, ny)
        }
    }

    override fun scroll(dx: Int, dy: Int) = session.scroll(dx, dy)

    override fun zoom(focusX: Float, focusY: Float, factor: Float) =
        relayoutBox(Viewport.zoomAround(box, base, focusX, focusY, factor, width.toFloat(), height.toFloat()))

    override fun pan(dx: Float, dy: Float) = relayoutBox(Viewport.pan(box, dx, dy, width.toFloat(), height.toFloat()))

    fun resetZoom() = zoomFit()

    override fun threeFingerTap() {
        performHapticFeedback(HapticFeedbackConstants.CONTEXT_CLICK)
        onThreeFingerTap?.invoke()
    }

    val zoomed: Boolean get() = Viewport.zoomOf(box, base) > 1.01f

    override fun feedback(kind: Feedback) {
        performHapticFeedback(if (kind == Feedback.DRAG_START) HapticFeedbackConstants.LONG_PRESS else HapticFeedbackConstants.CONTEXT_CLICK)
    }

    // ------------------------------------------------------------ cursor sink

    override fun onCursorShape(bitmap: Bitmap, hotX: Int, hotY: Int) = cursorView.setShape(bitmap, hotX, hotY)
    override fun onCursorPosition(nx: Int, ny: Int, visible: Boolean) {
        cursor.hostReport(nx, ny, SystemClock.uptimeMillis())
        cursorView.hostAt(nx, ny, visible)
    }
    override fun onVideoGeometry(width: Int, height: Int, sourceWidth: Int, sourceHeight: Int) {
        videoW = width; videoH = height; sourceW = sourceWidth; sourceH = sourceHeight
        box = Box(0f, 0f, 0f, 0f)
        requestLayout()
        // A new or restarted stream: the PC's session starts without caret
        // reporting, so ask again if the keyboard is open.
        if (keyboardInset > 0) session.setTextInput(true)
    }

    // ------------------------------------------------------------ keyboard

    fun showKeyboard() = keyboard.show()
    fun hideKeyboard() = keyboard.hide()

    /** Whether the phone's on-screen keyboard is showing right now. */
    val keyboardVisible: Boolean
        get() = ViewCompat.getRootWindowInsets(this)?.isVisible(WindowInsetsCompat.Type.ime()) == true

    fun toggleKeyboard() = if (keyboardVisible) hideKeyboard() else showKeyboard()

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        isFocusableInTouchMode = true
        requestFocus()
    }

    /**
     * Physical keyboards (Bluetooth/USB) reach the focused view hierarchy;
     * route them to the PC before any child sees them. Soft-keyboard input
     * arrives through [KeyboardInputView]'s input connection instead.
     */
    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        val hardware = event.device?.isVirtual == false &&
            event.source and android.view.InputDevice.SOURCE_KEYBOARD == android.view.InputDevice.SOURCE_KEYBOARD
        if (hardware && handleHardwareKey(event)) return true
        return super.dispatchKeyEvent(event)
    }

    /** Physical keyboard keys (Bluetooth/USB), forwarded as key presses. */
    fun handleHardwareKey(e: KeyEvent): Boolean {
        if (e.keyCode == KeyEvent.KEYCODE_BACK) return false
        val vk = KeyMap.vkFor(e.keyCode) ?: return false
        when (e.action) {
            KeyEvent.ACTION_DOWN -> session.key(vk, true)
            KeyEvent.ACTION_UP -> session.key(vk, false)
        }
        return true
    }

    companion object {
        /** Fraction of the view (per axis) the pointer roams before the view follows. */
        const val FOLLOW_ZONE = 0.6f
        /** Minimum zoom while typing, so the text is legible. */
        const val TYPING_ZOOM = 1.5f
    }

    override fun onDetachedFromWindow() {
        super.onDetachedFromWindow()
        removeCallbacks(timer)
        interpreter.reset()
        if (session.cursorSink === this) session.cursorSink = null
    }
}

/**
 * Draws the PC's pointer on the phone. In direct-touch mode the pointer
 * jumps to the finger immediately (local feedback) and the PC's own report
 * takes over again shortly after; in trackpad mode motion is predicted
 * locally and reconciled with the PC's position.
 */
class CursorView(context: Context) : View(context) {
    var box: Box = Box(0f, 0f, 0f, 0f)
    var scale = 1f
    var trackpad = false
    private var shape: Bitmap? = null
    private var hotX = 0
    private var hotY = 0
    private var nx = 32768
    private var ny = 32768
    private var visible = false
    private var lastLocal = 0L
    private val paint = Paint(Paint.FILTER_BITMAP_FLAG)
    private val arrow = Path()
    private val fillPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.WHITE }
    private val strokePaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.BLACK; style = Paint.Style.STROKE; strokeWidth = 1.5f * resources.displayMetrics.density
    }
    private val dst = RectF()

    init {
        val d = resources.displayMetrics.density
        arrow.moveTo(0f, 0f); arrow.lineTo(0f, 17 * d); arrow.lineTo(4.5f * d, 13 * d)
        arrow.lineTo(8 * d, 20 * d); arrow.lineTo(10.5f * d, 19 * d); arrow.lineTo(7.5f * d, 12 * d)
        arrow.lineTo(13 * d, 12 * d); arrow.close()
        isClickable = false
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
    }

    fun setShape(b: Bitmap, hx: Int, hy: Int) { shape = b; hotX = hx; hotY = hy; invalidate() }

    fun hostAt(x: Int, y: Int, vis: Boolean) {
        visible = vis
        // Do not fight the user's finger: local feedback wins briefly.
        if (SystemClock.uptimeMillis() - lastLocal < 250) return
        nx = x; ny = y
        invalidate()
    }

    fun localAt(x: Int, y: Int) {
        nx = x; ny = y; visible = true
        lastLocal = SystemClock.uptimeMillis()
        invalidate()
    }

    fun normalized(): Pair<Int, Int> = nx to ny

    fun viewPosition(): Pair<Float, Float>? =
        if (box.width > 0f) Viewport.toView(nx, ny, box) else null

    override fun onDraw(canvas: Canvas) {
        if (!visible || box.width <= 0f) return
        val (x, y) = Viewport.toView(nx, ny, box)
        val b = shape
        if (b != null) {
            // Show the pointer at the PC's size, but never smaller than
            // comfortably visible on a phone.
            val s = maxOf(scale, 0.6f * resources.displayMetrics.density / 2f)
            dst.set(x - hotX * s, y - hotY * s, x + (b.width - hotX) * s, y + (b.height - hotY) * s)
            canvas.drawBitmap(b, null, dst, paint)
        } else {
            canvas.save(); canvas.translate(x, y)
            canvas.drawPath(arrow, fillPaint); canvas.drawPath(arrow, strokePaint)
            canvas.restore()
        }
    }
}

/**
 * Invisible view that owns the soft-keyboard connection.
 *
 * Composition (predictive text, CJK input methods) stays on the phone until
 * the IME commits it, so nothing is sent twice when the IME revises its
 * composing region. The IME is shown the text typed so far in this burst,
 * so auto-capitalisation and auto-correction behave normally; corrections
 * arrive as delete + commit and are replayed on the PC as Backspace + text.
 * Tapping the remote screen resets that context (the PC's caret moved).
 */
class KeyboardInputView(context: Context, private val session: RemoteSession) : View(context) {
    private val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
    var onComposing: ((String) -> Unit)? = null
    private val typed = StringBuilder()
    private var composing = ""

    init {
        isFocusable = true
        isFocusableInTouchMode = true
    }

    override fun onCheckIsTextEditor() = true

    /**
     * Ask for the soft keyboard. The view must hold focus and be the IME's
     * served view before showSoftInput is honoured, which happens a frame
     * after requestFocus, hence the post. SHOW_IMPLICIT (used before) may be
     * ignored by the system, so it is not used. The insets controller is a
     * second route that some keyboards respond to more reliably.
     */
    fun show() {
        val focused = requestFocus()
        post {
            val accepted = imm.showSoftInput(this, 0)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                windowInsetsController?.show(WindowInsets.Type.ime())
            }
            Log.i(TAG, "keyboard requested: focus=$focused hasFocus=${hasFocus()} accepted=$accepted")
        }
    }

    fun hide() {
        imm.hideSoftInputFromWindow(windowToken, 0)
        clearFocus()
        Log.i(TAG, "keyboard hidden")
    }

    private companion object {
        const val TAG = "LANLinkKeyboard"
        /** Most keys one IME delete request may send; the count is the IME's. */
        const val MAX_IME_DELETE = 256
    }

    fun resetContext() {
        if (typed.isEmpty() && composing.isEmpty()) return
        typed.clear()
        composing = ""
        onComposing?.invoke("")
        if (hasFocus()) imm.restartInput(this)
    }

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE or
            InputType.TYPE_TEXT_FLAG_AUTO_CORRECT
        // Whatever is typed here goes to the PC (often passwords): ask the
        // keyboard not to learn or remember it (incognito mode in Gboard).
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI or EditorInfo.IME_FLAG_NO_FULLSCREEN or
            EditorInfo.IME_ACTION_NONE or EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
        outAttrs.initialSelStart = typed.length
        outAttrs.initialSelEnd = typed.length
        return Connection()
    }

    private fun commit(text: String) {
        if (text.isEmpty()) return
        session.text(text)
        typed.append(text)
        if (typed.length > 256) typed.delete(0, typed.length - 256)
    }

    private fun backspace(n: Int) {
        repeat(n) { session.tapKey(KeyMap.VK_BACK) }
        typed.setLength((typed.length - n).coerceAtLeast(0))
    }

    private inner class Connection : BaseInputConnection(this@KeyboardInputView, false) {
        override fun commitText(text: CharSequence, newCursorPosition: Int): Boolean {
            composing = ""
            onComposing?.invoke("")
            commit(text.toString())
            return true
        }

        override fun setComposingText(text: CharSequence, newCursorPosition: Int): Boolean {
            composing = text.toString()
            onComposing?.invoke(composing)
            return true
        }

        override fun setComposingRegion(start: Int, end: Int): Boolean {
            // The IME wants to re-compose already committed text. Pull it
            // back from the PC so the final commit replaces it exactly once.
            val len = (end - start).coerceAtLeast(0)
            if (len in 1..typed.length && end == typed.length) {
                val region = typed.substring(typed.length - len)
                backspace(len)
                composing = region
                onComposing?.invoke(composing)
            }
            return true
        }

        override fun finishComposingText(): Boolean {
            if (composing.isNotEmpty()) commit(composing)
            composing = ""
            onComposing?.invoke("")
            return true
        }

        override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
            if (composing.isNotEmpty() && beforeLength > 0) {
                // Deleting inside the composition: purely local.
                composing = composing.dropLast(beforeLength)
                onComposing?.invoke(composing)
                return true
            }
            backspace(beforeLength.coerceIn(0, MAX_IME_DELETE))
            repeat(afterLength.coerceIn(0, MAX_IME_DELETE)) { session.tapKey(KeyMap.VK_DELETE) }
            return true
        }

        override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int): Boolean {
            // Convert code points to UTF-16 units for our shadow buffer.
            var units = 0
            var cp = 0
            while (cp < beforeLength && units < typed.length) {
                val c = typed[typed.length - 1 - units]
                units += if (Character.isLowSurrogate(c) && units + 1 < typed.length) 2 else 1
                cp++
            }
            repeat(beforeLength.coerceIn(0, MAX_IME_DELETE)) { session.tapKey(KeyMap.VK_BACK) }
            typed.setLength((typed.length - units).coerceAtLeast(0))
            repeat(afterLength.coerceIn(0, MAX_IME_DELETE)) { session.tapKey(KeyMap.VK_DELETE) }
            return true
        }

        override fun sendKeyEvent(event: KeyEvent): Boolean {
            val vk = KeyMap.vkFor(event.keyCode)
            val printable = event.unicodeChar != 0 && vk != KeyMap.VK_RETURN && vk != KeyMap.VK_TAB &&
                vk != KeyMap.VK_BACK && vk != KeyMap.VK_DELETE
            if (printable) {
                if (event.action == KeyEvent.ACTION_DOWN) commit(String(Character.toChars(event.unicodeChar)))
                return true
            }
            if (vk == null) return false
            when (event.action) {
                KeyEvent.ACTION_DOWN -> {
                    session.key(vk, true)
                    if (vk == KeyMap.VK_BACK) typed.setLength((typed.length - 1).coerceAtLeast(0))
                }
                KeyEvent.ACTION_UP -> session.key(vk, false)
            }
            return true
        }

        override fun performEditorAction(actionCode: Int): Boolean {
            session.tapKey(KeyMap.VK_RETURN)
            typed.clear()
            return true
        }

        override fun getTextBeforeCursor(n: Int, flags: Int): CharSequence =
            typed.substring((typed.length - n).coerceAtLeast(0))

        override fun getTextAfterCursor(n: Int, flags: Int): CharSequence = ""
        override fun getSelectedText(flags: Int): CharSequence? = null
    }
}
