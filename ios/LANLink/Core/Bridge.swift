import Foundation
import LanLinkCore

/// A user-presentable error from the Rust core. `message` is the exact text
/// the core produced.
struct LANLinkError: Error, LocalizedError, Equatable {
    let message: String

    var errorDescription: String? { message }
}

/// Swift surface of the Rust core (`LanLinkCore.xcframework`, C ABI in
/// `ios/native-bridge/include/lanlink.h`). Mirrors the Kotlin `NativeBridge`.
///
/// Every throwing function throws `LANLinkError` with a user-presentable
/// message. Blocking functions (`connect`, `pollEvent`, `nextVideo`) must be
/// called off the main thread. Every buffer or string the core hands back is
/// freed here, exactly once.
enum Bridge {
    // Video return codes and `meta` flags. See the Rust docs for the layout.
    static let VIDEO_TIMEOUT: Int32 = -1
    static let VIDEO_CONFIG: Int32 = -2
    static let VIDEO_CLOSED: Int32 = -3
    static let VIDEO_BUFFER_SMALL: Int32 = -4
    static let META_KEYFRAME: Int64 = 1
    static let META_CODEC_CONFIG: Int64 = 2
    static let META_DISCONTINUITY: Int64 = 4
    static let DISPLAY_PRIMARY: Int64 = 0xFFFF_FFFF

    /// Number of `Int64` entries `nextVideo` may write into `meta`.
    static let META_COUNT = 8

    // MARK: - Identity and pairing links

    static func generateIdentity(label: String) throws -> Data {
        var out: UnsafeMutablePointer<UInt8>? = nil
        var len = 0
        defer { lanlink_bytes_free(out, len) }
        try call { lanlink_generate_identity(label, &out, &len, $0) }
        guard let p = out else { throw LANLinkError(message: "The identity could not be created.") }
        return Data(bytes: p, count: len)
    }

    static func identityFingerprint(_ identity: Data) throws -> String {
        var out: UnsafeMutablePointer<CChar>? = nil
        defer { lanlink_string_free(out) }
        try withBytes(identity) { id, n in
            try call { lanlink_identity_fingerprint(id, n, &out, $0) }
        }
        return try string(out)
    }

    static func parsePairingUri(_ uri: String) throws -> String {
        var out: UnsafeMutablePointer<CChar>? = nil
        defer { lanlink_string_free(out) }
        try call { lanlink_parse_pairing_uri(uri, &out, $0) }
        return try string(out)
    }

    // MARK: - Pairing (two-phase so the confirmation code can be shown while
    // the PC user decides)

    static func pairStart(
        identity: Data, deviceName: String, deviceModel: String,
        host: String, port: Int32, fingerprintHex: String, code: String
    ) throws -> Int64 {
        var job: Int64 = 0
        try withBytes(identity) { id, n in
            try call {
                lanlink_pair_start(id, n, deviceName, deviceModel, host, port, fingerprintHex, code, &job, $0)
            }
        }
        return job
    }

    static func pairPoll(job: Int64) throws -> String {
        var out: UnsafeMutablePointer<CChar>? = nil
        defer { lanlink_string_free(out) }
        try call { lanlink_pair_poll(job, &out, $0) }
        return try string(out)
    }

    static func pairCancel(job: Int64) {
        lanlink_pair_cancel(job)
    }

    // MARK: - Connection

    static func connect(
        identity: Data, deviceName: String, deviceModel: String,
        host: String, port: Int32, fingerprintHex: String
    ) throws -> Int64 {
        var handle: Int64 = 0
        try withBytes(identity) { id, n in
            try call {
                lanlink_connect(id, n, deviceName, deviceModel, host, port, fingerprintHex, &handle, $0)
            }
        }
        return handle
    }

    static func close(handle: Int64) {
        lanlink_close(handle)
    }

    static func sendCapabilities(
        handle: Int64, maxW: Int32, maxH: Int32, maxFps: Int32, hardware: Bool,
        lowLatency: Bool, displayW: Int32, displayH: Int32, refreshMilliHz: Int32
    ) throws {
        try call {
            lanlink_send_capabilities(
                handle, maxW, maxH, maxFps, hardware, lowLatency, displayW, displayH, refreshMilliHz, $0)
        }
    }

    static func startSession(
        handle: Int64, displayId: Int64, quality: Int32, maxW: Int32, maxH: Int32,
        fps: Int32, kbps: Int32, maxKbps: Int32, viewOnly: Bool
    ) throws {
        try call {
            lanlink_start_session(handle, displayId, quality, maxW, maxH, fps, kbps, maxKbps, viewOnly, $0)
        }
    }

    static func setStreamParams(
        handle: Int64, quality: Int32, maxW: Int32, maxH: Int32, fps: Int32, kbps: Int32, maxKbps: Int32
    ) throws {
        try call { lanlink_set_stream_params(handle, quality, maxW, maxH, fps, kbps, maxKbps, $0) }
    }

    static func endSession(handle: Int64) throws {
        try call { lanlink_end_session(handle, $0) }
    }

    static func selectDisplay(handle: Int64, displayId: Int64) throws {
        try call { lanlink_select_display(handle, displayId, $0) }
    }

    static func bindSession(handle: Int64, sessionId: Int64) throws {
        try call { lanlink_bind_session(handle, sessionId, $0) }
    }

    /// Phone keyboard open/closed; false if the PC is too old to report the caret.
    @discardableResult
    static func setTextInput(handle: Int64, active: Bool) throws -> Bool {
        var result = false
        try call { lanlink_set_text_input(handle, active, &result, $0) }
        return result
    }

    static func sendClientStats(
        handle: Int64, receivedFpsX10: Int32, renderedFpsX10: Int32, decodeP50Us: Int32,
        decodeP95Us: Int32, dropped: Int32, queue: Int32
    ) throws {
        try call {
            lanlink_send_client_stats(
                handle, receivedFpsX10, renderedFpsX10, decodeP50Us, decodeP95Us, dropped, queue, $0)
        }
    }

    /// The next event as JSON, or nil on timeout.
    static func pollEvent(handle: Int64, timeoutMs: Int32) throws -> String? {
        var out: UnsafeMutablePointer<CChar>? = nil
        defer { lanlink_string_free(out) }
        try call { lanlink_poll_event(handle, timeoutMs, &out, $0) }
        return out.map { String(cString: $0) }
    }

    /// The cursor shape's pixels, or nil when the shape is unknown.
    static func cursorPixels(handle: Int64, shapeId: Int64) throws -> Data? {
        var out: UnsafeMutablePointer<UInt8>? = nil
        var len = 0
        defer { lanlink_bytes_free(out, len) }
        try call { lanlink_cursor_pixels(handle, shapeId, &out, &len, $0) }
        return out.map { Data(bytes: $0, count: len) }
    }

    // MARK: - Video

    /// Writes the next decoder input into `buffer`. Returns the byte count
    /// (>= 0) or a negative `VIDEO_*` code. `meta` must hold at least
    /// `META_COUNT` entries; only the entries the result defines are written.
    static func nextVideo(
        handle: Int64, buffer: UnsafeMutableRawBufferPointer, meta: inout [Int64], timeoutMs: Int32
    ) throws -> Int32 {
        precondition(meta.count >= META_COUNT, "meta needs \(META_COUNT) entries")
        var code: Int32 = 0
        let buf = buffer.baseAddress?.assumingMemoryBound(to: UInt8.self)
        let cap = buffer.count
        try meta.withUnsafeMutableBufferPointer { m in
            let base = m.baseAddress
            try call { lanlink_next_video(handle, buf, cap, base, timeoutMs, &code, $0) }
        }
        return code
    }

    static func decoderReset(handle: Int64) throws {
        try call { lanlink_decoder_reset(handle, $0) }
    }

    static func requestKeyframe(handle: Int64) throws {
        try call { lanlink_request_keyframe(handle, $0) }
    }

    static func nowUs() -> Int64 {
        lanlink_now_us()
    }

    // MARK: - Input

    static func sendPointer(handle: Int64, x: Int32, y: Int32) throws {
        try call { lanlink_send_pointer(handle, x, y, $0) }
    }

    static func sendPointerRel(handle: Int64, dx: Int32, dy: Int32) throws {
        try call { lanlink_send_pointer_rel(handle, dx, dy, $0) }
    }

    @discardableResult
    static func sendButton(handle: Int64, button: Int32, down: Bool, x: Int32, y: Int32) throws -> Bool {
        var result = false
        try call { lanlink_send_button(handle, button, down, x, y, &result, $0) }
        return result
    }

    @discardableResult
    static func sendScroll(handle: Int64, dx: Int32, dy: Int32) throws -> Bool {
        var result = false
        try call { lanlink_send_scroll(handle, dx, dy, &result, $0) }
        return result
    }

    @discardableResult
    static func sendKey(handle: Int64, vk: Int32, down: Bool) throws -> Bool {
        var result = false
        try call { lanlink_send_key(handle, vk, down, &result, $0) }
        return result
    }

    @discardableResult
    static func sendText(handle: Int64, text: String) throws -> Bool {
        var result = false
        try call { lanlink_send_text(handle, text, &result, $0) }
        return result
    }

    static func releaseAll(handle: Int64) throws {
        try call { lanlink_release_all(handle, $0) }
    }

    // MARK: - Helpers

    private typealias ErrorOut = UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>

    /// Runs one C call; on -1 throws its error text, which is always freed.
    private static func call(_ body: (ErrorOut) -> Int32) throws {
        var err: UnsafeMutablePointer<CChar>? = nil
        defer { lanlink_string_free(err) }
        let rc = withUnsafeMutablePointer(to: &err) { body($0) }
        if rc != 0 {
            let message = err.map { String(cString: $0) } ?? "Unknown error in the LANLink core."
            throw LANLinkError(message: message)
        }
    }

    private static func withBytes<R>(
        _ data: Data, _ body: (UnsafePointer<UInt8>?, Int) throws -> R
    ) rethrows -> R {
        try data.withUnsafeBytes { raw in
            try body(raw.bindMemory(to: UInt8.self).baseAddress, raw.count)
        }
    }

    private static func string(_ p: UnsafeMutablePointer<CChar>?) throws -> String {
        guard let p else { throw LANLinkError(message: "The LANLink core returned no result.") }
        return String(cString: p)
    }
}
