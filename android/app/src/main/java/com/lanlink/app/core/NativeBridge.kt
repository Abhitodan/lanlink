package com.lanlink.app.core

import java.nio.ByteBuffer

/**
 * JNI surface of `liblanlink.so` (Rust, `android/native-bridge`).
 *
 * Every function may throw [java.io.IOException] with a user-presentable
 * message. Blocking functions (`connect`, `pollEvent`, `nextVideo`) must be
 * called off the main thread.
 */
object NativeBridge {
    init {
        System.loadLibrary("lanlink")
    }

    // Identity and pairing links.
    @JvmStatic external fun generateIdentity(label: String): ByteArray
    @JvmStatic external fun identityFingerprint(identity: ByteArray): String
    @JvmStatic external fun parsePairingUri(uri: String): String

    // Pairing (two-phase so the confirmation code can be shown while the
    // PC user decides).
    @JvmStatic external fun pairStart(
        identity: ByteArray, deviceName: String, deviceModel: String,
        host: String, port: Int, fingerprintHex: String, code: String,
    ): Long
    @JvmStatic external fun pairPoll(job: Long): String
    @JvmStatic external fun pairCancel(job: Long)

    // Connection.
    @JvmStatic external fun connect(
        identity: ByteArray, deviceName: String, deviceModel: String,
        host: String, port: Int, fingerprintHex: String,
    ): Long
    @JvmStatic external fun close(handle: Long)
    @JvmStatic external fun sendCapabilities(
        handle: Long, maxW: Int, maxH: Int, maxFps: Int, hardware: Boolean,
        lowLatency: Boolean, displayW: Int, displayH: Int, refreshMilliHz: Int,
    )
    @JvmStatic external fun startSession(
        handle: Long, displayId: Long, quality: Int, maxW: Int, maxH: Int,
        fps: Int, kbps: Int, maxKbps: Int, viewOnly: Boolean,
    )
    @JvmStatic external fun setStreamParams(
        handle: Long, quality: Int, maxW: Int, maxH: Int, fps: Int, kbps: Int, maxKbps: Int,
    )
    @JvmStatic external fun endSession(handle: Long)
    @JvmStatic external fun selectDisplay(handle: Long, displayId: Long)
    @JvmStatic external fun bindSession(handle: Long, sessionId: Long)
    /** Phone keyboard open/closed; false if the PC is too old to report the caret. */
    @JvmStatic external fun setTextInput(handle: Long, active: Boolean): Boolean
    @JvmStatic external fun sendClientStats(
        handle: Long, receivedFpsX10: Int, renderedFpsX10: Int, decodeP50Us: Int,
        decodeP95Us: Int, dropped: Int, queue: Int,
    )
    @JvmStatic external fun pollEvent(handle: Long, timeoutMs: Int): String?
    @JvmStatic external fun cursorPixels(handle: Long, shapeId: Long): ByteArray?

    // Video. See the Rust docs for the return codes and `meta` layout.
    @JvmStatic external fun nextVideo(handle: Long, buffer: ByteBuffer, meta: LongArray, timeoutMs: Int): Int
    @JvmStatic external fun decoderReset(handle: Long)
    @JvmStatic external fun requestKeyframe(handle: Long)
    @JvmStatic external fun nowUs(): Long

    // Input.
    @JvmStatic external fun sendPointer(handle: Long, x: Int, y: Int)
    @JvmStatic external fun sendPointerRel(handle: Long, dx: Int, dy: Int)
    @JvmStatic external fun sendButton(handle: Long, button: Int, down: Boolean, x: Int, y: Int): Boolean
    @JvmStatic external fun sendScroll(handle: Long, dx: Int, dy: Int): Boolean
    @JvmStatic external fun sendKey(handle: Long, vk: Int, down: Boolean): Boolean
    @JvmStatic external fun sendText(handle: Long, text: String): Boolean
    @JvmStatic external fun releaseAll(handle: Long)

    const val VIDEO_TIMEOUT = -1
    const val VIDEO_CONFIG = -2
    const val VIDEO_CLOSED = -3
    const val VIDEO_BUFFER_SMALL = -4
    const val META_KEYFRAME = 1L
    const val META_CODEC_CONFIG = 2L
    const val META_DISCONTINUITY = 4L
    const val DISPLAY_PRIMARY = 0xFFFFFFFFL
}
