import AVFoundation
import CoreMedia
import Foundation

// Swift twin of android/.../session/VideoDecoder.kt. Android feeds
// MediaCodec; here `VideoDecoder` hands AVCC samples to an
// `AVSampleBufferDisplayLayer` (hardware decode in VideoToolbox) and
// `VideoPump` is the feeder thread that pulls access units from the core.

/// What the decoder on this phone can do, reported to the PC (Kotlin's
/// `DecoderCaps`). iPhones on iOS 16 decode H.264 in hardware up to 4K.
struct DecoderCaps: Equatable {
    let name: String
    let hardware: Bool
    let lowLatency: Bool
    let maxWidth: Int
    let maxHeight: Int
    let maxFps: Int

    static let videoToolbox = DecoderCaps(
        name: "VideoToolbox (H.264)", hardware: true, lowLatency: false,
        maxWidth: 3840, maxHeight: 2160, maxFps: 60)
}

/// Live decoder measurements (Kotlin's `DecoderStats`). Thread-safe.
///
/// The display layer decodes internally and reports no per-frame decode
/// time, so decode percentiles are not measured on iPhone; "rendered" counts
/// frames handed to the layer for immediate display, and the latency
/// estimate is taken at that hand-off.
final class DecoderStats {
    private let lock = NSLock()
    private var receivedCount: Int64 = 0
    private var renderedCount: Int64 = 0
    private var droppedCount: Int64 = 0
    private var restartCount = 0
    private var captureToDecodedUs = Int64.min
    private var hostPipelineUs: Int64 = 0

    var received: Int64 { locked { receivedCount } }
    var rendered: Int64 { locked { renderedCount } }
    var dropped: Int64 { locked { droppedCount } }
    var restarts: Int { locked { restartCount } }
    /// Int64.min until the first frame is shown.
    var lastCaptureToDecodedUs: Int64 { locked { captureToDecodedUs } }
    var lastHostPipelineUs: Int64 { locked { hostPipelineUs } }

    func addReceived(hostPipelineUs us: Int64) {
        locked {
            receivedCount += 1
            hostPipelineUs = us
        }
    }

    func addRendered(captureToDecodedUs us: Int64) {
        locked {
            renderedCount += 1
            captureToDecodedUs = us
        }
    }

    func addDropped() { locked { droppedCount += 1 } }

    func addRestart() { locked { restartCount += 1 } }

    private func locked<T>(_ body: () -> T) -> T {
        lock.lock()
        defer { lock.unlock() }
        return body()
    }
}

enum DecodeResult: Equatable {
    /// Handed to the layer for immediate display.
    case displayed
    /// No usable parameter sets or keyframe yet; a keyframe was requested.
    case waitingForKeyframe
    /// The frame could not be used (malformed, or the layer failed); the
    /// layer was flushed and a keyframe requested.
    case failed
}

/// Turns H.264 access units (Annex B) into samples on an
/// `AVSampleBufferDisplayLayer`. Not thread-safe: every call, including
/// every call into the layer, comes from one thread (the video pump).
///
/// Frames are shown as soon as they are decoded
/// (`kCMSampleAttachmentKey_DisplayImmediately`): nothing is scheduled for
/// presentation, so the newest picture always wins, as on Android.
final class VideoDecoder {
    /// Seconds between repeated keyframe requests while still waiting.
    static let KEYFRAME_RETRY_S: TimeInterval = 1.0

    let layer: AVSampleBufferDisplayLayer
    private(set) var formatDescription: CMVideoFormatDescription?
    private let requestKeyframe: () -> Void
    private let clock: () -> TimeInterval
    private var sps: Data?
    private var pps: Data?
    private var needKeyframe = true
    private var lastRequestAt: TimeInterval?

    /// `requestKeyframe` asks the PC for a keyframe (the session passes
    /// `Bridge.requestKeyframe`).
    init(
        layer: AVSampleBufferDisplayLayer = AVSampleBufferDisplayLayer(),
        requestKeyframe: @escaping () -> Void,
        clock: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }
    ) {
        self.layer = layer
        self.requestKeyframe = requestKeyframe
        self.clock = clock
    }

    /// A codec-config buffer from the core (`META_CODEC_CONFIG`): SPS and
    /// PPS in Annex B.
    func handle(config: Data) {
        if let ps = AnnexB.parameterSets(AnnexB.nalUnits(config)) {
            updateFormat(ps)
        }
    }

    func handle(frame: Data, keyframe: Bool, discontinuity: Bool) -> DecodeResult {
        if layer.status == .failed {
            restart(request: true)
        } else if discontinuity {
            // Frames were dropped on the PC. The protocol always sends this
            // flag on a keyframe, so drop what is queued and start from this
            // frame; a keyframe is requested below only if it cannot be used.
            restart(request: false)
        }
        let nals = AnnexB.nalUnits(frame)
        if let ps = AnnexB.parameterSets(nals) {
            updateFormat(ps)
        }
        let isKey = keyframe || nals.contains { AnnexB.nalType($0) == AnnexB.NAL_IDR }
        guard let format = formatDescription, isKey || !needKeyframe else {
            askForKeyframe()
            return .waitingForKeyframe
        }
        // Parameter sets travel in the format description, not the sample.
        let units = nals.filter {
            let t = AnnexB.nalType($0)
            return t != AnnexB.NAL_SPS && t != AnnexB.NAL_PPS && t != AnnexB.NAL_AUD
        }
        let hasSlice = units.contains { (AnnexB.NAL_SLICE...AnnexB.NAL_IDR).contains(AnnexB.nalType($0)) }
        guard hasSlice, let sample = Self.makeSample(AnnexB.avcc(units), format: format) else {
            restart(request: true)
            return .failed
        }
        layer.enqueue(sample)
        if layer.status == .failed {
            restart(request: true)
            return .failed
        }
        needKeyframe = false
        lastRequestAt = nil
        return .displayed
    }

    /// Start over (new stream configuration): forget the parameter sets and
    /// wait for the next keyframe.
    func reset() {
        layer.flush()
        formatDescription = nil
        sps = nil
        pps = nil
        needKeyframe = true
        lastRequestAt = nil
    }

    // MARK: - Internals

    /// Drops what the layer holds and waits for a keyframe. Requests stay
    /// throttled, so a run of bad frames cannot flood the PC.
    private func restart(request: Bool) {
        layer.flush()
        needKeyframe = true
        if request { askForKeyframe() }
    }

    /// Requests a keyframe once per wait, and again every
    /// `KEYFRAME_RETRY_S` while still waiting.
    private func askForKeyframe() {
        let now = clock()
        if let last = lastRequestAt, now - last < Self.KEYFRAME_RETRY_S { return }
        lastRequestAt = now
        requestKeyframe()
    }

    private func updateFormat(_ ps: (sps: Data, pps: Data)) {
        if formatDescription != nil && ps.sps == sps && ps.pps == pps { return }
        guard let f = Self.makeFormat(sps: ps.sps, pps: ps.pps) else {
            formatDescription = nil
            sps = nil
            pps = nil
            needKeyframe = true
            return
        }
        if formatDescription != nil {
            // New parameter sets: only a keyframe can follow them.
            needKeyframe = true
        }
        formatDescription = f
        sps = ps.sps
        pps = ps.pps
    }

    private static func makeFormat(sps: Data, pps: Data) -> CMVideoFormatDescription? {
        var format: CMFormatDescription?
        let status = sps.withUnsafeBytes { (s: UnsafeRawBufferPointer) -> OSStatus in
            pps.withUnsafeBytes { (p: UnsafeRawBufferPointer) -> OSStatus in
                guard let sp = s.bindMemory(to: UInt8.self).baseAddress,
                      let pp = p.bindMemory(to: UInt8.self).baseAddress
                else { return kCMFormatDescriptionError_InvalidParameter }
                let pointers: [UnsafePointer<UInt8>] = [sp, pp]
                let sizes: [Int] = [s.count, p.count]
                return CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    allocator: kCFAllocatorDefault,
                    parameterSetCount: 2,
                    parameterSetPointers: pointers,
                    parameterSetSizes: sizes,
                    nalUnitHeaderLength: 4,
                    formatDescriptionOut: &format)
            }
        }
        return status == noErr ? format : nil
    }

    private static func makeSample(_ avcc: Data, format: CMVideoFormatDescription) -> CMSampleBuffer? {
        let length = avcc.count
        guard length > 0 else { return nil }
        var block: CMBlockBuffer?
        var status = CMBlockBufferCreateWithMemoryBlock(
            allocator: kCFAllocatorDefault, memoryBlock: nil, blockLength: length,
            blockAllocator: kCFAllocatorDefault, customBlockSource: nil, offsetToData: 0,
            dataLength: length, flags: kCMBlockBufferAssureMemoryNowFlag, blockBufferOut: &block)
        guard status == kCMBlockBufferNoErr, let block else { return nil }
        status = avcc.withUnsafeBytes { raw -> OSStatus in
            guard let base = raw.baseAddress else { return kCMBlockBufferBadPointerParameterErr }
            return CMBlockBufferReplaceDataBytes(
                with: base, blockBuffer: block, offsetIntoDestination: 0, dataLength: length)
        }
        guard status == kCMBlockBufferNoErr else { return nil }

        var sample: CMSampleBuffer?
        var size = length
        status = CMSampleBufferCreateReady(
            allocator: kCFAllocatorDefault, dataBuffer: block, formatDescription: format,
            sampleCount: 1, sampleTimingEntryCount: 0, sampleTimingArray: nil,
            sampleSizeEntryCount: 1, sampleSizeArray: &size, sampleBufferOut: &sample)
        guard status == noErr, let sample else { return nil }
        if let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: true),
           CFArrayGetCount(attachments) > 0 {
            let dict = unsafeBitCast(CFArrayGetValueAtIndex(attachments, 0), to: CFMutableDictionary.self)
            CFDictionarySetValue(
                dict,
                Unmanaged.passUnretained(kCMSampleAttachmentKey_DisplayImmediately).toOpaque(),
                Unmanaged.passUnretained(kCFBooleanTrue).toOpaque())
        }
        return sample
    }
}

/// The feeder thread of one session's video (the `feedLoop` of Kotlin's
/// `VideoDecoder`): pulls access units from the core into a reused buffer
/// and feeds them to a `VideoDecoder`. Every layer call happens on this
/// thread.
final class VideoPump {
    /// Start size of the access-unit buffer (Kotlin's `maxInput`).
    static let INITIAL_BUFFER = 1 << 20
    /// The buffer never grows past this (as `maxInput` on Android).
    static let MAX_BUFFER = 16 << 20
    /// `nextVideo` timeout: how long `stop()` may wait for a blocked call.
    static let POLL_MS: Int32 = 100

    let stats = DecoderStats()
    let decoder: VideoDecoder
    private let handle: Int64
    private let initialGeometry: [Int]?
    private let onVideoSize: (_ width: Int, _ height: Int, _ sourceWidth: Int, _ sourceHeight: Int) -> Void

    private let cond = NSCondition()
    private var running = false
    private var started = false
    private var exited = false
    private var clockOffsetUs: Int64 = 0

    // Pump thread only.
    private var width = 0
    private var height = 0
    private var configured = false

    /// `initialGeometry` = `[width, height, sourceWidth, sourceHeight]` of a
    /// stream that is already running (decoder recreated mid-stream).
    init(
        handle: Int64, layer: AVSampleBufferDisplayLayer, initialGeometry: [Int]?,
        onVideoSize: @escaping (_ width: Int, _ height: Int, _ sourceWidth: Int, _ sourceHeight: Int) -> Void
    ) {
        self.handle = handle
        self.initialGeometry = initialGeometry
        self.onVideoSize = onVideoSize
        decoder = VideoDecoder(layer: layer, requestKeyframe: { _ = try? Bridge.requestKeyframe(handle: handle) })
    }

    /// host_clock − client_clock, from the session's Ping/Pong.
    func setClockOffset(_ us: Int64) {
        cond.lock()
        clockOffsetUs = us
        cond.unlock()
    }

    func start() {
        cond.lock()
        guard !started else { return cond.unlock() }
        started = true
        running = true
        cond.unlock()
        let t = Thread { [self] in
            self.run()
            self.cond.lock()
            self.exited = true
            self.cond.broadcast()
            self.cond.unlock()
        }
        t.name = "lanlink-video"
        t.qualityOfService = .userInteractive
        t.start()
    }

    /// Ends the pump and waits up to `timeout` seconds for its thread to
    /// return (a blocked `nextVideo` returns within `POLL_MS`). Returns true
    /// once the thread is gone. Safe to call more than once, from any thread
    /// but the pump's own.
    @discardableResult
    func stop(timeout: TimeInterval = 1) -> Bool {
        cond.lock()
        defer { cond.unlock() }
        running = false
        let deadline = Date(timeIntervalSinceNow: timeout)
        while started && !exited {
            if !cond.wait(until: deadline) { break }
        }
        return !started || exited
    }

    private var isRunning: Bool {
        cond.lock()
        defer { cond.unlock() }
        return running
    }

    private var offsetUs: Int64 {
        cond.lock()
        defer { cond.unlock() }
        return clockOffsetUs
    }

    private func run() {
        if let g = initialGeometry, g.count == 4, g[0] > 0, g[1] > 0 {
            onConfig([Int64(g[0]), Int64(g[1]), 60, 0, 0, Int64(g[2]), Int64(g[3]), 0])
        }
        var meta = [Int64](repeating: 0, count: Bridge.META_COUNT)
        var capacity = Self.INITIAL_BUFFER
        var buffer = UnsafeMutableRawBufferPointer.allocate(byteCount: capacity, alignment: 16)
        defer { buffer.deallocate() }

        while isRunning {
            let r: Int32
            do {
                r = try Bridge.nextVideo(handle: handle, buffer: buffer, meta: &meta, timeoutMs: Self.POLL_MS)
            } catch {
                return // the session is gone
            }
            guard isRunning else { return }
            switch r {
            case 1...:
                guard let base = buffer.baseAddress else { return }
                let data = Data(bytes: base, count: Int(r))
                let flags = meta[0]
                if flags & Bridge.META_CODEC_CONFIG != 0 {
                    decoder.handle(config: data)
                    continue
                }
                stats.addReceived(hostPipelineUs: meta[4])
                let result = decoder.handle(
                    frame: data,
                    keyframe: flags & Bridge.META_KEYFRAME != 0,
                    discontinuity: flags & Bridge.META_DISCONTINUITY != 0)
                if result == .displayed {
                    // Capture time is on the host clock; convert "now" to it.
                    stats.addRendered(captureToDecodedUs: Bridge.nowUs() + offsetUs - meta[1])
                } else {
                    stats.addDropped()
                }
            case Bridge.VIDEO_CONFIG:
                onConfig(meta)
            case Bridge.VIDEO_BUFFER_SMALL:
                // The frame is kept in the core; grow and ask again.
                let grown = min(Int(meta[0]) * 3 / 2, Self.MAX_BUFFER)
                if grown > capacity {
                    buffer.deallocate()
                    capacity = grown
                    buffer = UnsafeMutableRawBufferPointer.allocate(byteCount: capacity, alignment: 16)
                } else {
                    // Larger than any buffer we allow (Android behaves the
                    // same); drop a held delta frame and wait.
                    try? Bridge.decoderReset(handle: handle)
                    Thread.sleep(forTimeInterval: 0.02)
                }
            case Bridge.VIDEO_CLOSED:
                return
            default:
                continue // VIDEO_TIMEOUT
            }
        }
    }

    /// A new stream configuration: `meta` = `[width, height, fps, configId,
    /// displayId, sourceWidth, sourceHeight, 0]`.
    private func onConfig(_ meta: [Int64]) {
        let w = Int(meta[0])
        let h = Int(meta[1])
        onVideoSize(w, h, Int(meta[5]), Int(meta[6]))
        if configured && w == width && h == height {
            // Same size: keep the decoder; new parameter sets arrive in-band.
            try? Bridge.decoderReset(handle: handle)
            return
        }
        decoder.reset()
        width = w
        height = h
        configured = true
        stats.addRestart()
        try? Bridge.decoderReset(handle: handle)
    }
}
