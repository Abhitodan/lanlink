package com.lanlink.app.session

import android.graphics.Bitmap
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.Surface
import com.lanlink.app.LanLinkApp
import com.lanlink.app.core.KeyMap
import com.lanlink.app.core.NativeBridge
import com.lanlink.app.core.PairedHost
import com.lanlink.app.core.Quality
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import org.json.JSONObject

private const val TAG = "LANLinkSession"

enum class Phase { CONNECTING, STARTING, STREAMING, RECONNECTING, ENDED, FAILED }

data class DisplayItem(val id: Long, val name: String, val width: Int, val height: Int, val primary: Boolean)

/** Measured numbers for the diagnostics screen. `null` = not measured. */
data class Diagnostics(
    val rttMs: Float? = null,
    val receivedFps: Float? = null,
    val renderedFps: Float? = null,
    val capturedFps: Float? = null,
    val encodedFps: Float? = null,
    val bitrateKbps: Int? = null,
    val targetKbps: Int? = null,
    val decodeP50Ms: Float? = null,
    val decodeP95Ms: Float? = null,
    val encodeP50Ms: Float? = null,
    val encodeP95Ms: Float? = null,
    val droppedOnPc: Int? = null,
    val skippedOnPhone: Long? = null,
    val queueDepth: Int? = null,
    /** Capture on the PC → decoded on the phone, using the Ping/Pong clock
     *  offset; ± [latencyUncertaintyMs]. An estimate, not a measurement. */
    val latencyEstimateMs: Float? = null,
    val latencyUncertaintyMs: Float? = null,
    /** PC side only: capture → handed to the network (single clock). */
    val hostPipelineMs: Float? = null,
    val lostPackets: Long? = null,
    val decoder: String? = null,
    val resolution: String? = null,
)

data class SessionState(
    val phase: Phase = Phase.CONNECTING,
    val hostName: String = "",
    val message: String? = null,
    val notice: String? = null,
    val displays: List<DisplayItem> = emptyList(),
    val displayId: Long = NativeBridge.DISPLAY_PRIMARY,
    val encoder: String = "",
    val capture: String = "",
    val viewOnly: Boolean = false,
    val reconnects: Int = 0,
    val diagnostics: Diagnostics = Diagnostics(),
    val modifiers: Set<Int> = emptySet(),
)

/** Receives pointer state for the local cursor overlay (main thread). */
interface CursorSink {
    fun onCursorShape(bitmap: Bitmap, hotX: Int, hotY: Int)
    fun onCursorPosition(nx: Int, ny: Int, visible: Boolean)
    fun onVideoGeometry(width: Int, height: Int, sourceWidth: Int, sourceHeight: Int)
    /** The PC's text caret (normalised x, y, w, h), or not visible. */
    fun onCaret(visible: Boolean, x: Int, y: Int, w: Int, h: Int) {}
}

/**
 * One remote-desktop session with one PC, including automatic reconnection.
 *
 * Threads: a connector thread (blocking connect), an event thread
 * (control-plane events from native), the decoder's own threads, and the
 * main thread for UI calls. Input calls are non-blocking and safe from the
 * main thread.
 */
class RemoteSession(
    private val app: LanLinkApp,
    val host: PairedHost,
    private val preferredAddress: String?,
) {
    private val main = Handler(Looper.getMainLooper())
    private val _state = MutableStateFlow(SessionState(hostName = host.name))
    val state: StateFlow<SessionState> = _state

    @Volatile private var handle = 0L
    @Volatile private var stopped = false
    @Volatile private var surface: Surface? = null
    private var decoder: VideoDecoder? = null
    private var eventThread: Thread? = null
    var cursorSink: CursorSink? = null
    private val cursorShapes = HashMap<Long, Triple<Bitmap, Int, Int>>()
    private var lastStatsAt = 0L
    private var lastReceived = 0L
    private var lastRendered = 0L
    @Volatile private var rttUs = 0L
    @Volatile private var offsetUs = 0L
    private val sticky = linkedSetOf<Int>()

    // ------------------------------------------------------------ lifecycle

    fun start() {
        stopped = false
        Thread({ connectLoop(first = true) }, "lanlink-connect").start()
    }

    /** User ended the session. */
    fun stop() {
        if (stopped) return
        stopped = true
        val h = handle
        handle = 0
        Thread({
            if (h != 0L) {
                runCatching { NativeBridge.releaseAll(h) }
                runCatching { NativeBridge.endSession(h) }
                Thread.sleep(80) // let the release and end reach the PC
                runCatching { NativeBridge.close(h) }
            }
        }, "lanlink-stop").start()
        stopDecoder()
        _state.update { it.copy(phase = Phase.ENDED, message = null) }
    }

    private fun addresses(): List<String> =
        (listOfNotNull(preferredAddress) + host.addresses).distinct()

    private fun connectLoop(first: Boolean) {
        val deadline = System.currentTimeMillis() + if (first) 15_000 else 60_000
        var lastError = "The PC did not answer."
        var attempt = 0
        // Addresses where a different computer answered. Discovery names and
        // ids are public, so a LAN device can advertise itself as this PC;
        // that must not stop the real address from being tried.
        val impostors = mutableSetOf<String>()
        while (!stopped && System.currentTimeMillis() < deadline) {
            val candidates = addresses().filter { it !in impostors }
            if (candidates.isEmpty()) {
                fail(friendly(lastError))
                return
            }
            for (addr in candidates) {
                if (stopped) return
                try {
                    val identity = app.identity.get(app.settings.deviceName)
                    val h = NativeBridge.connect(
                        identity, app.settings.deviceName, android.os.Build.MODEL ?: "Android",
                        addr, host.port, host.fingerprint,
                    )
                    if (stopped) { NativeBridge.close(h); return }
                    handle = h
                    app.hosts.touch(host.fingerprint, addr)
                    onConnected(h)
                    return
                } catch (e: Exception) {
                    lastError = e.message ?: lastError
                    Log.i(TAG, "connect to $addr failed: $lastError")
                    if (lastError.contains("identity mismatch")) {
                        impostors += addr
                        continue
                    }
                    // Only the real PC can say this: the connection already
                    // checked its pinned identity.
                    if (lastError.contains("not paired")) {
                        fail(friendly(lastError))
                        return
                    }
                }
            }
            attempt++
            Thread.sleep(if (attempt < 3) 700L else 1500L)
        }
        if (!stopped) fail(friendly(lastError))
    }

    private fun friendly(e: String): String = when {
        e.contains("identity mismatch") ->
            "This is not the PC you paired with. If the PC was reinstalled, remove it and pair again."
        e.contains("not paired") -> "The PC no longer trusts this phone. Pair again from the PC."
        e.contains("did not answer") || e.contains("timed out", true) ->
            "Cannot reach ${host.name}. Check that both devices are on the same Wi-Fi, that LANLink is running on the PC, and that Windows Firewall allows it."
        else -> e
    }

    private fun fail(msg: String) {
        stopDecoder()
        _state.update { it.copy(phase = Phase.FAILED, message = msg) }
    }

    private fun onConnected(h: Long) {
        _state.update { it.copy(phase = Phase.STARTING, message = null) }
        val caps = DecoderProbe.best()?.second
        val metrics = app.displayMetrics()
        runCatching {
            NativeBridge.sendCapabilities(
                h, caps?.maxWidth ?: 1920, caps?.maxHeight ?: 1080, caps?.maxFps ?: 60,
                caps?.hardware ?: false, caps?.lowLatency ?: false,
                metrics.first, metrics.second, (app.refreshRateHz() * 1000).toInt(),
            )
        }
        _state.update { it.copy(diagnostics = it.diagnostics.copy(decoder = caps?.name)) }
        eventThread = Thread({ eventLoop(h) }, "lanlink-events").also { it.start() }
        main.post { maybeStartDecoder() }
    }

    private fun requestSession(h: Long) {
        val s = app.settings
        val q = s.quality
        val custom = q == Quality.CUSTOM
        val (pw, ph) = app.displayMetrics()
        // Never ask for more pixels than the phone screen can show, unless
        // the user chose Sharp (useful when zooming into small text).
        val longEdge = maxOf(pw, ph)
        val maxW = when {
            custom && s.customMaxWidth > 0 -> s.customMaxWidth
            q == Quality.SHARP -> 0
            else -> longEdge
        }
        runCatching {
            NativeBridge.startSession(
                h, _state.value.displayId, q.wire, maxW, maxW,
                if (custom) s.customFps else minOf(60, app.refreshRateHz().toInt()),
                if (custom) s.customKbps else 0, s.maxKbps, s.viewOnly,
            )
        }
    }

    // ------------------------------------------------------------ events

    private fun eventLoop(h: Long) {
        while (!stopped && handle == h) {
            val json = try {
                NativeBridge.pollEvent(h, 250)
            } catch (e: Exception) {
                onClosed(h, e.message ?: "connection lost")
                return
            }
            if (json != null) {
                val o = runCatching { JSONObject(json) }.getOrNull() ?: continue
                // A malformed event must not crash the app.
                val keepGoing = try {
                    handleEvent(h, o)
                } catch (e: org.json.JSONException) {
                    Log.w(TAG, "ignored malformed event: ${e.message}")
                    true
                }
                if (!keepGoing) return
            }
            maybeSendStats(h)
        }
    }

    /** Returns false when the connection is gone. */
    private fun handleEvent(h: Long, o: JSONObject): Boolean {
        when (o.optString("type")) {
            "displays" -> {
                val arr = o.getJSONArray("displays")
                val list = List(arr.length()) {
                    val d = arr.getJSONObject(it)
                    DisplayItem(d.getLong("id"), d.getString("name"), d.getInt("width"), d.getInt("height"), d.getBoolean("primary"))
                }
                val firstList = _state.value.displays.isEmpty()
                _state.update { it.copy(displays = list) }
                if (firstList && _state.value.phase == Phase.STARTING) requestSession(h)
            }
            "sessionStarted" -> {
                val sid = o.getLong("sessionId")
                NativeBridge.bindSession(h, sid)
                _state.update {
                    it.copy(
                        phase = Phase.STREAMING, message = null,
                        displayId = o.getLong("displayId"),
                        encoder = o.optString("encoder"), capture = o.optString("capture"),
                        viewOnly = o.optBoolean("viewOnly"),
                    )
                }
            }
            "sessionEnded" -> {
                if (!stopped) _state.update { it.copy(phase = Phase.ENDED, message = "The PC ended the session.") }
            }
            "stats" -> {
                val d = _state.value.diagnostics
                _state.update {
                    it.copy(
                        diagnostics = d.copy(
                            capturedFps = o.optInt("capturedFpsX10") / 10f,
                            encodedFps = o.optInt("encodedFpsX10") / 10f,
                            bitrateKbps = o.optInt("bitrateKbps"),
                            targetKbps = o.optInt("targetKbps"),
                            encodeP50Ms = o.optInt("encodeP50") / 1000f,
                            encodeP95Ms = o.optInt("encodeP95") / 1000f,
                            droppedOnPc = o.optInt("droppedEncode") + o.optInt("droppedSend"),
                            queueDepth = o.optInt("queue"),
                            lostPackets = o.optLong("lostPackets"),
                            skippedOnPhone = o.optLong("framesSkipped"),
                        ),
                    )
                }
            }
            "rtt" -> {
                rttUs = o.optLong("rttUs")
                offsetUs = o.optLong("offsetUs")
                decoder?.setClockOffset(offsetUs)
            }
            "cursorShape" -> {
                val id = o.getLong("id")
                val w = o.getInt("width"); val hgt = o.getInt("height")
                val px = runCatching { NativeBridge.cursorPixels(h, id) }.getOrNull()
                if (px != null && px.size == w * hgt * 4) {
                    val colors = IntArray(w * hgt) { i ->
                        val r = px[i * 4].toInt() and 0xff
                        val g = px[i * 4 + 1].toInt() and 0xff
                        val b = px[i * 4 + 2].toInt() and 0xff
                        val a = px[i * 4 + 3].toInt() and 0xff
                        (a shl 24) or (r shl 16) or (g shl 8) or b
                    }
                    val bmp = Bitmap.createBitmap(colors, w, hgt, Bitmap.Config.ARGB_8888)
                    val entry = Triple(bmp, o.getInt("hotX"), o.getInt("hotY"))
                    synchronized(cursorShapes) {
                        if (cursorShapes.size > 32) cursorShapes.clear()
                        cursorShapes[id] = entry
                    }
                    main.post { cursorSink?.onCursorShape(entry.first, entry.second, entry.third) }
                }
            }
            "cursor" -> {
                val x = o.getInt("x"); val y = o.getInt("y"); val vis = o.getBoolean("visible")
                main.post { cursorSink?.onCursorPosition(x, y, vis) }
            }
            "caret" -> {
                val vis = o.getBoolean("visible")
                val x = o.getInt("x"); val y = o.getInt("y"); val w = o.getInt("w"); val ch = o.getInt("h")
                main.post { cursorSink?.onCaret(vis, x, y, w, ch) }
            }
            "notice" -> {
                val msg = o.optString("message")
                _state.update { it.copy(notice = msg) }
                main.postDelayed({ _state.update { s -> if (s.notice == msg) s.copy(notice = null) else s } }, 6000)
            }
            "error" -> {
                val code = o.optInt("code")
                val msg = o.optString("message")
                // Session-level errors (busy, denied, capture failed) end the
                // attempt but keep the reason visible.
                _state.update {
                    if (it.phase == Phase.STREAMING && code == 12) it.copy(notice = msg)
                    else it.copy(phase = Phase.FAILED, message = msg)
                }
            }
            "closed" -> {
                onClosed(h, o.optString("message"))
                return false
            }
        }
        return true
    }

    private fun onClosed(h: Long, reason: String) {
        if (stopped || handle != h) return
        handle = 0
        runCatching { NativeBridge.close(h) }
        main.post { stopDecoder() }
        val phase = _state.value.phase
        if (phase == Phase.FAILED || phase == Phase.ENDED) return
        if (reason.contains("stopped on the PC") || reason.contains("revoked")) {
            _state.update {
                it.copy(phase = Phase.ENDED, message = if (reason.contains("revoked")) "The PC removed this phone." else "The session was stopped on the PC.")
            }
            return
        }
        // Network blip, PC asleep, Wi-Fi switch: try to come back.
        _state.update { it.copy(phase = Phase.RECONNECTING, message = "Connection lost. Reconnecting…", reconnects = it.reconnects + 1) }
        Thread({ connectLoop(first = false) }, "lanlink-reconnect").start()
    }

    private fun maybeSendStats(h: Long) {
        val now = System.currentTimeMillis()
        if (now - lastStatsAt < 1000) return
        val dt = if (lastStatsAt == 0L) 1f else (now - lastStatsAt) / 1000f
        lastStatsAt = now
        val dec = decoder ?: return
        val rec = dec.stats.received.get()
        val ren = dec.stats.rendered.get()
        val recFps = (rec - lastReceived) / dt
        val renFps = (ren - lastRendered) / dt
        lastReceived = rec; lastRendered = ren
        val (p50, p95) = dec.stats.takeDecodePercentiles()
        runCatching {
            NativeBridge.sendClientStats(h, (recFps * 10).toInt(), (renFps * 10).toInt(), p50, p95, dec.stats.dropped.get().toInt(), 0)
        }
        val lat = dec.stats.lastCaptureToDecodedUs
        _state.update {
            it.copy(
                diagnostics = it.diagnostics.copy(
                    rttMs = if (rttUs > 0) rttUs / 1000f else null,
                    receivedFps = recFps,
                    renderedFps = renFps,
                    decodeP50Ms = if (p50 > 0) p50 / 1000f else it.diagnostics.decodeP50Ms,
                    decodeP95Ms = if (p95 > 0) p95 / 1000f else it.diagnostics.decodeP95Ms,
                    latencyEstimateMs = if (lat != Long.MIN_VALUE && rttUs > 0) lat / 1000f else null,
                    latencyUncertaintyMs = if (rttUs > 0) rttUs / 2000f else null,
                    hostPipelineMs = dec.stats.lastHostPipelineUs.takeIf { v -> v > 0 }?.let { v -> v / 1000f },
                ),
            )
        }
    }

    // ------------------------------------------------------------ video

    fun attachSurface(s: Surface) {
        surface = s
        maybeStartDecoder()
    }

    fun detachSurface() {
        surface = null
        stopDecoder()
    }

    private fun maybeStartDecoder() {
        val s = surface ?: return
        val h = handle
        if (h == 0L || decoder != null) return
        val d = VideoDecoder(h, s, object : VideoDecoder.Listener {
            override fun onVideoSize(width: Int, height: Int, sourceWidth: Int, sourceHeight: Int) {
                _state.update { it.copy(diagnostics = it.diagnostics.copy(resolution = "${width}×$height")) }
                main.post { cursorSink?.onVideoGeometry(width, height, sourceWidth, sourceHeight) }
            }

            override fun onDecoderError(message: String) {
                Log.w(TAG, "decoder: $message")
                main.post { decoder?.requestRestart() }
            }
        })
        d.setClockOffset(offsetUs)
        decoder = d
        d.start()
        // A new decoder needs a keyframe even if the stream is running.
        runCatching { NativeBridge.decoderReset(h) }
    }

    private fun stopDecoder() {
        val d = decoder ?: return
        decoder = null
        Thread({ d.stop() }, "lanlink-decoder-stop").start()
    }

    // ------------------------------------------------------------ control

    fun selectDisplay(id: Long) {
        val h = handle
        if (h != 0L) runCatching { NativeBridge.selectDisplay(h, id) }
        _state.update { it.copy(displayId = id) }
    }

    fun applyQuality() {
        val h = handle
        if (h == 0L) return
        val s = app.settings
        val custom = s.quality == Quality.CUSTOM
        val (pw, ph) = app.displayMetrics()
        val longEdge = maxOf(pw, ph)
        val maxW = when {
            custom && s.customMaxWidth > 0 -> s.customMaxWidth
            s.quality == Quality.SHARP -> 0
            else -> longEdge
        }
        runCatching {
            NativeBridge.setStreamParams(
                h, s.quality.wire, maxW, maxW,
                if (custom) s.customFps else minOf(60, app.refreshRateHz().toInt()),
                if (custom) s.customKbps else 0, s.maxKbps,
            )
        }
    }

    fun retry() {
        if (handle != 0L) return
        stopped = false
        _state.update { it.copy(phase = Phase.CONNECTING, message = null) }
        Thread({ connectLoop(first = true) }, "lanlink-connect").start()
    }

    // ------------------------------------------------------------ input

    private inline fun withHandle(f: (Long) -> Unit) {
        val h = handle
        if (h != 0L && !_state.value.viewOnly) runCatching { f(h) }
    }

    fun pointer(nx: Int, ny: Int) = withHandle { NativeBridge.sendPointer(it, nx, ny) }
    fun pointerRel(dx: Int, dy: Int) = withHandle { NativeBridge.sendPointerRel(it, dx, dy) }

    /** Tell the PC the phone keyboard opened/closed so it reports the caret. */
    fun setTextInput(active: Boolean) = withHandle { NativeBridge.setTextInput(it, active) }
    fun scroll(dx: Int, dy: Int) = withHandle { NativeBridge.sendScroll(it, dx, dy) }

    /** [nx]/[ny] < 0 = click where the pointer is. */
    fun button(button: Int, down: Boolean, nx: Int = -1, ny: Int = -1) = withHandle {
        if (down) pressSticky(it)
        NativeBridge.sendButton(it, button, down, nx, ny)
        if (!down) releaseSticky(it)
    }

    fun key(vk: Int, down: Boolean) = withHandle { NativeBridge.sendKey(it, vk, down) }

    /** A full key press, combined with any sticky modifiers. */
    fun tapKey(vk: Int) = withHandle {
        pressSticky(it)
        NativeBridge.sendKey(it, vk, true)
        NativeBridge.sendKey(it, vk, false)
        releaseSticky(it)
    }

    /** Press keys in order and release in reverse (e.g. Alt+Tab). */
    fun combo(vararg vks: Int) = withHandle { h ->
        vks.forEach { NativeBridge.sendKey(h, it, true) }
        vks.reversed().forEach { NativeBridge.sendKey(h, it, false) }
    }

    /**
     * Committed text from the on-screen keyboard. With a sticky modifier
     * active (Ctrl, Alt, Win) characters become key presses so shortcuts
     * like Ctrl+C work; otherwise text is sent as Unicode.
     */
    fun text(t: String) = withHandle { h ->
        val shortcut = synchronized(sticky) { sticky.any { it != KeyMap.VK_LSHIFT } }
        if (!shortcut) {
            NativeBridge.sendText(h, t)
            return@withHandle
        }
        pressSticky(h)
        for (c in t) {
            val vk = KeyMap.vkForShortcutChar(c)
            if (vk != null) {
                NativeBridge.sendKey(h, vk, true)
                NativeBridge.sendKey(h, vk, false)
            } else {
                NativeBridge.sendText(h, c.toString())
            }
        }
        releaseSticky(h)
    }

    fun toggleSticky(vk: Int) {
        synchronized(sticky) { if (!sticky.remove(vk)) sticky.add(vk) }
        _state.update { it.copy(modifiers = synchronized(sticky) { sticky.toSet() }) }
    }

    private fun pressSticky(h: Long) {
        synchronized(sticky) { sticky.toList() }.forEach { NativeBridge.sendKey(h, it, true) }
    }

    private fun releaseSticky(h: Long) {
        val mods = synchronized(sticky) { sticky.toList().also { sticky.clear() } }
        mods.reversed().forEach { NativeBridge.sendKey(h, it, false) }
        if (mods.isNotEmpty()) _state.update { it.copy(modifiers = emptySet()) }
    }

    fun releaseAll() = withHandle { NativeBridge.releaseAll(it) }
}
