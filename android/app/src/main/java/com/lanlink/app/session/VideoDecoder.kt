package com.lanlink.app.session

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import android.view.Surface
import com.lanlink.app.core.NativeBridge
import java.nio.ByteBuffer
import java.util.concurrent.LinkedBlockingDeque
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

private const val TAG = "LANLinkDecoder"
private const val MIME = MediaFormat.MIMETYPE_VIDEO_AVC

/** What the hardware decoder on this phone can do, reported to the PC. */
data class DecoderCaps(
    val name: String,
    val hardware: Boolean,
    val lowLatency: Boolean,
    val maxWidth: Int,
    val maxHeight: Int,
    val maxFps: Int,
)

object DecoderProbe {
    /** Best H.264 decoder: hardware first, then the largest supported size. */
    fun best(): Pair<MediaCodecInfo, DecoderCaps>? {
        val list = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
            .filter { !it.isEncoder && it.supportedTypes.any { t -> t.equals(MIME, true) } }
        val scored = list.mapNotNull { info ->
            val caps = runCatching { info.getCapabilitiesForType(MIME) }.getOrNull() ?: return@mapNotNull null
            val v = caps.videoCapabilities ?: return@mapNotNull null
            val hw = if (Build.VERSION.SDK_INT >= 29) info.isHardwareAccelerated else !info.name.startsWith("OMX.google")
            val ll = Build.VERSION.SDK_INT >= 30 && caps.isFeatureSupported(MediaCodecInfo.CodecCapabilities.FEATURE_LowLatency)
            val w = v.supportedWidths.upper
            val h = v.supportedHeights.upper
            val fps = runCatching { v.getSupportedFrameRatesFor(minOf(w, 1920), minOf(h, 1080)).upper.toInt() }
                .getOrDefault(60).coerceIn(30, 240)
            info to DecoderCaps(info.name, hw, ll, w, h, fps)
        }
        return scored.maxWithOrNull(compareBy({ it.second.hardware }, { it.second.lowLatency }, { it.second.maxWidth * it.second.maxHeight }))
    }
}

/** Live decoder measurements. */
class DecoderStats {
    val received = AtomicLong()
    val rendered = AtomicLong()
    val dropped = AtomicLong()
    val restarts = AtomicInteger()
    /** Recent decode times (queue → output), microseconds. */
    private val decodeUs = IntArray(256)
    private var n = 0
    @Volatile var lastCaptureToDecodedUs = Long.MIN_VALUE
    @Volatile var lastHostPipelineUs = 0L

    @Synchronized fun addDecode(us: Int) { decodeUs[n % decodeUs.size] = us; n++ }

    /** p50/p95 of recent decode times, then reset. */
    @Synchronized fun takeDecodePercentiles(): Pair<Int, Int> {
        val count = minOf(n, decodeUs.size)
        if (count == 0) return 0 to 0
        val s = decodeUs.copyOf(count).sorted()
        n = 0
        return s[(count - 1) / 2] to s[(count - 1) * 95 / 100]
    }
}

/**
 * Decodes the stream of one session onto a [Surface].
 *
 * MediaCodec runs in asynchronous mode on its own handler thread. A feeder
 * thread takes free input buffers and asks the native library to write the
 * next access unit straight into them. Decoded frames are released to the
 * surface immediately ("render as soon as decoded"): nothing is queued for
 * presentation, so the newest picture always wins.
 */
class VideoDecoder(
    private val handle: Long,
    private val surface: Surface,
    private val listener: Listener,
) {
    interface Listener {
        /** Coded picture size changed (also called for the first frame). */
        fun onVideoSize(width: Int, height: Int, sourceWidth: Int, sourceHeight: Int)
        fun onDecoderError(message: String)
    }

    val stats = DecoderStats()
    @Volatile private var running = false
    private var feeder: Thread? = null
    private val codecThread = HandlerThread("lanlink-codec").apply { start() }
    private val codecHandler = Handler(codecThread.looper)

    private var codec: MediaCodec? = null
    @Volatile private var generation = 0
    private val freeInputs = LinkedBlockingDeque<Int>()
    private var width = 0
    private var height = 0
    private var maxInput = 1 shl 20
    @Volatile private var clockOffsetUs = 0L
    private val queuedAtUs = LongArray(64)
    private val queuedPts = LongArray(64)

    /** host_clock − client_clock, from the session's Ping/Pong. */
    fun setClockOffset(offsetUs: Long) { clockOffsetUs = offsetUs }

    fun start() {
        running = true
        feeder = Thread({ feedLoop() }, "lanlink-feed").apply {
            priority = Thread.MAX_PRIORITY
            start()
        }
    }

    fun stop() {
        running = false
        feeder?.join(1000)
        feeder = null
        releaseCodec()
        codecThread.quitSafely()
    }

    private fun releaseCodec() {
        val c = codec ?: return
        codec = null
        generation++
        freeInputs.clear()
        runCatching { c.stop() }
        runCatching { c.release() }
    }

    private fun createCodec(w: Int, h: Int) {
        releaseCodec()
        val (info, _) = DecoderProbe.best() ?: run {
            listener.onDecoderError("This phone has no H.264 decoder.")
            return
        }
        val format = MediaFormat.createVideoFormat(MIME, w, h).apply {
            setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, maxInput)
            setInteger(MediaFormat.KEY_PRIORITY, 0) // realtime
            if (Build.VERSION.SDK_INT >= 30) {
                val caps = info.getCapabilitiesForType(MIME)
                if (caps.isFeatureSupported(MediaCodecInfo.CodecCapabilities.FEATURE_LowLatency)) {
                    setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
                }
            }
        }
        val c = MediaCodec.createByCodecName(info.name)
        val gen = generation + 1
        c.setCallback(object : MediaCodec.Callback() {
            override fun onInputBufferAvailable(mc: MediaCodec, index: Int) {
                if (generation == gen) freeInputs.offer(index)
            }

            override fun onOutputBufferAvailable(mc: MediaCodec, index: Int, info: MediaCodec.BufferInfo) {
                if (generation != gen) return
                val now = NativeBridge.nowUs()
                val slot = (info.presentationTimeUs % queuedPts.size).toInt().let { if (it < 0) it + queuedPts.size else it }
                if (queuedPts[slot] == info.presentationTimeUs) {
                    stats.addDecode((now - queuedAtUs[slot]).toInt().coerceAtLeast(0))
                }
                // capture time is on the host clock; convert "now" to it.
                stats.lastCaptureToDecodedUs = now + clockOffsetUs - info.presentationTimeUs
                runCatching { mc.releaseOutputBuffer(index, true) }
                stats.rendered.incrementAndGet()
            }

            override fun onError(mc: MediaCodec, e: MediaCodec.CodecException) {
                if (generation != gen) return
                Log.w(TAG, "decoder error", e)
                listener.onDecoderError(e.diagnosticInfo ?: "decoder error")
            }

            override fun onOutputFormatChanged(mc: MediaCodec, format: MediaFormat) {}
        }, codecHandler)
        c.configure(format, surface, null, 0)
        c.setVideoScalingMode(MediaCodec.VIDEO_SCALING_MODE_SCALE_TO_FIT)
        generation = gen
        codec = c
        c.start()
        width = w
        height = h
        stats.restarts.incrementAndGet()
        NativeBridge.decoderReset(handle)
    }

    /** Called from the UI when the decoder must start over (error, surface). */
    fun requestRestart() {
        codecHandler.post {
            if (width > 0) {
                runCatching { createCodec(width, height) }
                    .onFailure { listener.onDecoderError(it.message ?: "decoder restart failed") }
            }
        }
    }

    private fun feedLoop() {
        val meta = LongArray(8)
        val scratch = ByteBuffer.allocateDirect(64 * 1024)
        while (running) {
            val c = codec
            if (c == null) {
                // Before the first configuration: only a config record can come.
                val r = try {
                    NativeBridge.nextVideo(handle, scratch, meta, 100)
                } catch (e: Exception) {
                    return
                }
                when (r) {
                    NativeBridge.VIDEO_CONFIG -> onConfig(meta)
                    NativeBridge.VIDEO_CLOSED -> return
                    else -> {}
                }
                continue
            }
            val gen = generation
            val index = freeInputs.poll(50, TimeUnit.MILLISECONDS) ?: continue
            if (gen != generation) continue
            val buf = try { c.getInputBuffer(index) } catch (e: IllegalStateException) { null } ?: continue
            buf.clear()
            val r = try {
                NativeBridge.nextVideo(handle, buf, meta, 50)
            } catch (e: Exception) {
                return
            }
            when {
                r > 0 -> {
                    val flags = meta[0]
                    val pts = meta[1]
                    var mcFlags = 0
                    if (flags and NativeBridge.META_CODEC_CONFIG != 0L) mcFlags = mcFlags or MediaCodec.BUFFER_FLAG_CODEC_CONFIG
                    else {
                        stats.received.incrementAndGet()
                        stats.lastHostPipelineUs = meta[4]
                        if (flags and NativeBridge.META_KEYFRAME != 0L) mcFlags = mcFlags or MediaCodec.BUFFER_FLAG_KEY_FRAME
                        val slot = (pts % queuedPts.size).toInt().let { if (it < 0) it + queuedPts.size else it }
                        queuedPts[slot] = pts
                        queuedAtUs[slot] = NativeBridge.nowUs()
                    }
                    try {
                        c.queueInputBuffer(index, 0, r, pts, mcFlags)
                    } catch (e: IllegalStateException) {
                        // Codec was released underneath us; next loop sees it.
                    }
                }
                r == NativeBridge.VIDEO_TIMEOUT -> freeInputs.offerFirst(index)
                r == NativeBridge.VIDEO_CONFIG -> {
                    freeInputs.offerFirst(index)
                    onConfig(meta)
                }
                r == NativeBridge.VIDEO_BUFFER_SMALL -> {
                    freeInputs.offerFirst(index)
                    maxInput = (meta[0].toInt() * 3 / 2).coerceAtMost(16 shl 20)
                    Log.i(TAG, "access unit larger than decoder buffers; recreating with $maxInput bytes")
                    codecHandler.post { runCatching { createCodec(width, height) } }
                    Thread.sleep(20)
                }
                r == NativeBridge.VIDEO_CLOSED -> return
            }
        }
    }

    private fun onConfig(meta: LongArray) {
        val w = meta[0].toInt()
        val h = meta[1].toInt()
        val srcW = meta[5].toInt()
        val srcH = meta[6].toInt()
        listener.onVideoSize(w, h, srcW, srcH)
        if (w == width && h == height && codec != null) {
            // Same size: keep the decoder; new parameter sets arrive in-band.
            NativeBridge.decoderReset(handle)
            return
        }
        val done = java.util.concurrent.CountDownLatch(1)
        codecHandler.post {
            try {
                createCodec(w, h)
            } catch (e: Exception) {
                Log.e(TAG, "cannot create decoder", e)
                listener.onDecoderError("Cannot start the video decoder: ${e.message}")
            } finally {
                done.countDown()
            }
        }
        done.await(2, TimeUnit.SECONDS)
    }
}
