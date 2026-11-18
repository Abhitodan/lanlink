import AVFoundation
import Foundation
import UIKit

// Swift twin of android/.../session/RemoteSession.kt.

enum Phase { case connecting, starting, streaming, reconnecting, ended, failed }

struct DisplayItem: Equatable, Identifiable {
    let id: Int64
    let name: String
    let width: Int
    let height: Int
    let primary: Bool
}

struct SessionState: Equatable {
    var phase: Phase = .connecting
    var hostName = ""
    var message: String? = nil
    var notice: String? = nil
    var displays: [DisplayItem] = []
    var displayId: Int64 = Bridge.DISPLAY_PRIMARY
    var encoder = ""
    var capture = ""
    var viewOnly = false
    var reconnects = 0
    var diagnostics = Diagnostics()
    var modifiers: Set<Int> = []
}

/// Decides when a display list starts the stream: once per connection,
/// while the session is starting. The PC needs a new SessionStart on every
/// connection; the display list itself is kept across reconnects so the UI
/// does not flicker.
struct SessionRequestGate {
    private var requested = false

    /// A new connection is up (first connect, reconnect, retry).
    mutating func connectionStarted() { requested = false }

    /// A display list arrived; true when the stream should be requested now.
    mutating func onDisplays(phase: Phase) -> Bool {
        guard !requested && phase == .starting else { return false }
        requested = true
        return true
    }
}

/// What a connect loop does with the native handle it just opened.
enum ConnectDecision: Equatable {
    /// Publish it as the session's handle.
    case accept
    /// Not wanted: close it and end the loop.
    case close
}

/// Connection epochs (final review I-1). `start()`, `retry()` and `stop()`
/// each begin a new epoch. A connect loop, and the event loop and video pump
/// that follow from it, carry the epoch they were started in and write
/// nothing once it is no longer current. A trip to the background and back
/// calls `stop()` and then `retry()` while the old loop can still be inside
/// a blocking connect; without the epoch that loop would publish a second
/// handle, or later fail the healthy connection.
struct ConnectGate {
    private(set) var epoch = 0

    /// Begins a new epoch and returns it.
    mutating func advance() -> Int {
        epoch += 1
        return epoch
    }

    func isCurrent(_ e: Int) -> Bool { e == epoch }

    /// The loop of epoch `e` opened a handle. It may publish it only while
    /// its epoch is current, the session is not stopped and no handle is
    /// published yet; otherwise the new handle must be closed.
    func decide(epoch e: Int, stopped: Bool, handle: Int64) -> ConnectDecision {
        !stopped && isCurrent(e) && handle == 0 ? .accept : .close
    }
}

/// Receives pointer state for the local cursor overlay (main thread).
protocol CursorSink: AnyObject {
    func onCursorShape(bitmap: CGImage, hotX: Int, hotY: Int)
    func onCursorPosition(nx: Int, ny: Int, visible: Bool)
    func onVideoGeometry(width: Int, height: Int, sourceWidth: Int, sourceHeight: Int)
    /// The PC's text caret (normalised x, y, w, h), or not visible.
    func onCaret(visible: Bool, x: Int, y: Int, w: Int, h: Int)
}

extension CursorSink {
    func onCaret(visible: Bool, x: Int, y: Int, w: Int, h: Int) {}
}

/// The input surface the remote view drives. Coordinates are normalised
/// (0...65535) as on Android; x/y < 0 = "where the pointer is".
protocol RemoteInput: AnyObject {
    func pointer(x: Int, y: Int)
    func pointerBy(dx: Int, dy: Int)
    func button(_ button: Int, down: Bool, x: Int, y: Int)
    func scroll(dx: Int, dy: Int)
    func key(_ vk: Int, down: Bool)
    func tapKey(_ vk: Int)
    func text(_ t: String)
    func setTextInput(_ active: Bool)
    func releaseAll()
    /// Sticky Ctrl/Alt/Shift/Win for the next key (compact keyboard, shortcut bar).
    func toggleSticky(_ vk: Int)
    /// Press keys in order and release in reverse (e.g. Alt+Tab).
    func combo(vks: Int...)
}

extension RemoteInput {
    /// Click where the pointer is (Kotlin's default arguments).
    func button(_ button: Int, down: Bool) {
        self.button(button, down: down, x: -1, y: -1)
    }
}

/// One remote-desktop session with one PC, including automatic reconnection.
///
/// Threads: a connector thread (blocking connect), an event thread
/// (control-plane events), the video pump, and the main thread for UI calls.
/// Input calls are non-blocking and safe from the main thread. `state` is
/// published on the main thread; the other threads read and write a locked
/// copy of it.
///
/// Lifetime of the native handle: it is published under `lock` and taken
/// back (set to 0) under `lock` by exactly one of `stop()` or `onClosed`,
/// which then closes it, after the video pump (and, for `stop()`, the event
/// loop) has returned. A call that races with that close fails with a
/// "connection closed" error from the core: handles are registry ids, never
/// pointers.
final class RemoteSession: ObservableObject, SessionControl, RemoteInput {
    let host: PairedHost
    private let preferredAddress: String?
    private let settings: Settings
    private let hosts: HostRepository
    private let identity: IdentityStore
    private let deviceName: String
    private let deviceModel: String
    /// Screen pixels, long edge first (Kotlin's `displayMetrics()`).
    private let screen: (long: Int, short: Int)
    /// Highest refresh rate of the screen, Hz.
    private let refreshHz: Int
    private let caps = DecoderCaps.videoToolbox
    /// True when Discovery saw that Local Network access is denied (any
    /// thread). Connect errors then point to the permission.
    private let localNetworkDenied: () -> Bool

    @Published private(set) var state: SessionState

    /// Main thread only. Replays the last geometry, shape and position.
    weak var cursorSink: CursorSink? {
        didSet {
            guard let sink = cursorSink else { return }
            let (geom, shape, pos) = locked { (videoGeometry, lastCursorShape, lastCursorPos) }
            DispatchQueue.main.async {
                if let g = geom { sink.onVideoGeometry(width: g[0], height: g[1], sourceWidth: g[2], sourceHeight: g[3]) }
                if let s = shape { sink.onCursorShape(bitmap: s.image, hotX: s.hotX, hotY: s.hotY) }
                if let p = pos { sink.onCursorPosition(nx: p.x, ny: p.y, visible: p.visible) }
            }
        }
    }

    private typealias CursorShape = (image: CGImage, hotX: Int, hotY: Int)

    // Guarded by `lock`.
    private let lock = NSLock()
    private var shadow: SessionState
    private var shadowVersion = 0
    private var handle: Int64 = 0
    private var stopped = false
    private var layer: AVSampleBufferDisplayLayer?
    private var pump: VideoPump?
    private var eventLoopDone: DispatchSemaphore?
    private var requestGate = SessionRequestGate()
    private var connectGate = ConnectGate()
    private var cursorShapes: [Int64: CursorShape] = [:]
    private var lastShapeId: Int64 = -1
    private var lastCursorShape: CursorShape?
    private var lastCursorPos: (x: Int, y: Int, visible: Bool)?
    private var videoGeometry: [Int]?
    private var rttUs: Int64 = 0
    private var offsetUs: Int64 = 0
    /// Sticky modifiers in the order they were pressed.
    private var sticky: [Int] = []

    // Main thread only.
    private var appliedVersion = 0

    // Event thread only.
    private var lastStatsAt: UInt64 = 0
    private var lastReceived: Int64 = 0
    private var lastRendered: Int64 = 0

    /// Call on the main thread (reads the screen and the device name).
    init(
        host: PairedHost, preferredAddress: String?, settings: Settings,
        hosts: HostRepository, identity: IdentityStore,
        localNetworkDenied: @escaping () -> Bool = { false }
    ) {
        self.host = host
        self.localNetworkDenied = localNetworkDenied
        self.preferredAddress = preferredAddress
        self.settings = settings
        self.hosts = hosts
        self.identity = identity
        deviceName = settings.deviceName
        deviceModel = UIDevice.current.model
        let px = UIScreen.main.nativeBounds.size
        let w = Int(px.width), h = Int(px.height)
        screen = (max(max(w, h), 640), max(min(w, h), 480))
        let fps = UIScreen.main.maximumFramesPerSecond
        refreshHz = fps > 0 ? min(max(fps, 30), 240) : 60
        let initial = SessionState(hostName: host.name)
        state = initial
        shadow = initial
    }

    // MARK: - Lifecycle

    func start() {
        let e: Int = locked {
            stopped = false
            return connectGate.advance()
        }
        spawn("lanlink-connect") { self.connectLoop(first: true, epoch: e) }
    }

    /// User ended the session, or the app went to the background.
    func stop() {
        // Sticky modifiers must not outlive the session (final review I-3):
        // after a trip to the background, the next letter would become a
        // shortcut on the PC.
        clearSticky()
        let taken: (Int64, VideoPump?, DispatchSemaphore?)? = locked {
            // Loops of the old epoch write nothing from now on.
            _ = connectGate.advance()
            if stopped { return nil }
            stopped = true
            let result = (handle, pump, eventLoopDone)
            handle = 0
            pump = nil
            eventLoopDone = nil
            clearCursorLocked()
            return result
        }
        guard let t = taken else { return }
        let (h, p, events) = t
        spawn("lanlink-stop") {
            if h != 0 {
                _ = try? Bridge.releaseAll(handle: h)
                _ = try? Bridge.endSession(handle: h)
                Thread.sleep(forTimeInterval: 0.08) // let the release and end reach the PC
            }
            p?.stop()
            _ = events?.wait(timeout: .now() + 1)
            if h != 0 { Bridge.close(handle: h) }
        }
        update { $0.phase = .ended; $0.message = nil }
    }

    private func addresses() -> [String] {
        var out: [String] = []
        for a in [preferredAddress].compactMap({ $0 }) + host.addresses where !out.contains(a) { out.append(a) }
        return out
    }

    /// `e` is the epoch this loop was started in; the loop ends, writing
    /// nothing, as soon as it is no longer current.
    private func connectLoop(first: Bool, epoch e: Int) {
        let deadline = Date().addingTimeInterval(first ? 15 : 60)
        var lastError = "The PC did not answer."
        var attempt = 0
        // Addresses where a different computer answered. Discovery names and
        // ids are public, so a LAN device can advertise itself as this PC;
        // that must not stop the real address from being tried.
        var impostors = Set<String>()
        while isLive(e) && Date() < deadline {
            let candidates = addresses().filter { !impostors.contains($0) }
            if candidates.isEmpty {
                fail(friendly(lastError), epoch: e)
                return
            }
            for addr in candidates {
                if !isLive(e) { return }
                do {
                    let id = try identity.get(deviceLabel: deviceName)
                    let h = try Bridge.connect(
                        identity: id, deviceName: deviceName, deviceModel: deviceModel,
                        host: addr, port: Int32(clamping: host.port), fingerprintHex: host.fingerprint)
                    let decision: ConnectDecision = locked {
                        let d = connectGate.decide(epoch: e, stopped: stopped, handle: handle)
                        if d == .accept { handle = h }
                        return d
                    }
                    if decision == .close {
                        // A newer loop owns the session (or it was stopped):
                        // this connection must not stay open on the PC.
                        Bridge.close(handle: h)
                        return
                    }
                    hosts.touch(fp: host.fingerprint, address: addr)
                    onConnected(h, epoch: e)
                    return
                } catch {
                    lastError = (error as? LANLinkError)?.message ?? error.localizedDescription
                    if lastError.contains("identity mismatch") {
                        impostors.insert(addr)
                        continue
                    }
                    // Only the real PC can say this: the connection already
                    // checked its pinned identity.
                    if lastError.contains("not paired") {
                        fail(friendly(lastError), epoch: e)
                        return
                    }
                }
            }
            attempt += 1
            Thread.sleep(forTimeInterval: attempt < 3 ? 0.7 : 1.5)
        }
        fail(friendly(lastError), epoch: e)
    }

    private func friendly(_ e: String) -> String {
        Self.friendlyConnectError(e, hostName: host.name, localNetworkDenied: localNetworkDenied())
    }

    /// The message shown when connecting fails. With Local Network access
    /// denied (iOS only), every failure that is not an answer from the PC
    /// points to the permission: without it the phone cannot reach any LAN
    /// address, and Wi-Fi or the firewall are not the cause.
    static func friendlyConnectError(_ e: String, hostName: String, localNetworkDenied: Bool) -> String {
        if e.contains("identity mismatch") {
            return "This is not the PC you paired with. If the PC was reinstalled, remove it and pair again."
        }
        if e.contains("not paired") { return "The PC no longer trusts this phone. Pair again from the PC." }
        if localNetworkDenied { return LocalNetworkText.cannotReach(hostName) }
        if e.contains("did not answer") || e.range(of: "timed out", options: .caseInsensitive) != nil {
            return "Cannot reach \(hostName). Check that both devices are on the same Wi-Fi, that LANLink is running on the PC, and that Windows Firewall allows it."
        }
        return e
    }

    /// Shows a failure, unless epoch `e` is no longer current.
    private func fail(_ msg: String, epoch e: Int) {
        guard update(epoch: e, { $0.phase = .failed; $0.message = msg }) else { return }
        stopDecoder()
    }

    private func onConnected(_ h: Int64, epoch e: Int) {
        let accepted = update(epoch: e) { s in
            self.requestGate.connectionStarted()
            s.phase = .starting
            s.message = nil
        }
        // stop() took the handle in the meantime and closes it.
        guard accepted else { return }
        _ = try? Bridge.sendCapabilities(
            handle: h, maxW: Int32(caps.maxWidth), maxH: Int32(caps.maxHeight), maxFps: Int32(caps.maxFps),
            hardware: caps.hardware, lowLatency: caps.lowLatency,
            displayW: Int32(clamping: screen.long), displayH: Int32(clamping: screen.short),
            refreshMilliHz: Int32(clamping: refreshHz * 1000))
        update(epoch: e) { $0.diagnostics.decoder = self.caps.name }
        let done = DispatchSemaphore(value: 0)
        let live: Bool = locked {
            guard connectGate.isCurrent(e) && handle == h else { return false }
            eventLoopDone = done
            return true
        }
        guard live else { return }
        spawn("lanlink-events") {
            self.eventLoop(h, epoch: e)
            done.signal()
        }
        DispatchQueue.main.async { self.maybeStartDecoder() }
    }

    /// Stream parameters from the settings, shared by `startSession` and
    /// `applyQuality`. Never asks for more pixels than the phone screen can
    /// show, unless the user chose Sharp (useful when zooming into small
    /// text).
    private func streamPlan() -> (quality: Int32, maxW: Int32, fps: Int32, kbps: Int32, maxKbps: Int32) {
        let s = settings
        let q = s.quality
        let custom = q == .custom
        let maxW: Int
        if custom && s.customMaxWidth > 0 {
            maxW = s.customMaxWidth
        } else if q == .sharp {
            maxW = 0
        } else {
            maxW = screen.long
        }
        return (
            Int32(q.wire), Int32(clamping: maxW),
            Int32(clamping: custom ? s.customFps : min(60, refreshHz)),
            Int32(clamping: custom ? s.customKbps : 0), Int32(clamping: s.maxKbps))
    }

    private func requestSession(_ h: Int64) {
        let p = streamPlan()
        _ = try? Bridge.startSession(
            handle: h, displayId: current.displayId, quality: p.quality, maxW: p.maxW, maxH: p.maxW,
            fps: p.fps, kbps: p.kbps, maxKbps: p.maxKbps, viewOnly: settings.viewOnly)
    }

    // MARK: - Events

    /// Runs while epoch `epoch` is current and `h` is the session's handle.
    private func eventLoop(_ h: Int64, epoch: Int) {
        while isLive(epoch) && currentHandle == h {
            let json: String?
            do {
                json = try Bridge.pollEvent(handle: h, timeoutMs: 250)
            } catch {
                onClosed(h, reason: (error as? LANLinkError)?.message ?? "connection lost", epoch: epoch)
                return
            }
            // The session may have been stopped during the poll.
            if !isLive(epoch) { return }
            // A malformed event is ignored, never fatal.
            if let json, let data = json.data(using: .utf8),
               let o = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] {
                if !handleEvent(h, Event(o), epoch: epoch) { return }
            }
            maybeSendStats(h, epoch: epoch)
        }
    }

    /// Typed reads of an event's JSON fields.
    private struct Event {
        let o: [String: Any]
        init(_ o: [String: Any]) { self.o = o }
        func int(_ k: String) -> Int? { (o[k] as? NSNumber)?.intValue }
        func long(_ k: String) -> Int64? { (o[k] as? NSNumber)?.int64Value }
        func bool(_ k: String) -> Bool? { (o[k] as? NSNumber)?.boolValue }
        func string(_ k: String) -> String? { o[k] as? String }
        /// Kotlin's `optInt` etc.: 0 / false / "" when missing.
        func optInt(_ k: String) -> Int { int(k) ?? 0 }
        func optLong(_ k: String) -> Int64 { long(k) ?? 0 }
        func optString(_ k: String) -> String { string(k) ?? "" }
    }

    /// Returns false when the connection is gone. Writes nothing once
    /// `epoch` is no longer current.
    private func handleEvent(_ h: Int64, _ e: Event, epoch: Int) -> Bool {
        switch e.optString("type") {
        case "displays":
            guard let arr = e.o["displays"] as? [[String: Any]] else { break }
            var list: [DisplayItem] = []
            for d in arr.map(Event.init) {
                guard let id = d.long("id"), let name = d.string("name"), let w = d.int("width"),
                      let ht = d.int("height"), let primary = d.bool("primary")
                else { return true }
                list.append(DisplayItem(id: id, name: name, width: w, height: ht, primary: primary))
            }
            var request = false
            update(epoch: epoch) { s in
                s.displays = list
                request = self.requestGate.onDisplays(phase: s.phase)
            }
            if request { requestSession(h) }
        case "sessionStarted":
            guard let sid = e.long("sessionId"), let displayId = e.long("displayId") else { break }
            _ = try? Bridge.bindSession(handle: h, sessionId: sid)
            update(epoch: epoch) {
                $0.phase = .streaming
                $0.message = nil
                $0.displayId = displayId
                $0.encoder = e.optString("encoder")
                $0.capture = e.optString("capture")
                $0.viewOnly = e.bool("viewOnly") ?? false
            }
        case "sessionEnded":
            update(epoch: epoch) { $0.phase = .ended; $0.message = "The PC ended the session." }
        case "stats":
            update(epoch: epoch) {
                $0.diagnostics.capturedFps = Float(e.optInt("capturedFpsX10")) / 10
                $0.diagnostics.encodedFps = Float(e.optInt("encodedFpsX10")) / 10
                $0.diagnostics.bitrateKbps = e.optInt("bitrateKbps")
                $0.diagnostics.targetKbps = e.optInt("targetKbps")
                $0.diagnostics.encodeP50Ms = Float(e.optInt("encodeP50")) / 1000
                $0.diagnostics.encodeP95Ms = Float(e.optInt("encodeP95")) / 1000
                $0.diagnostics.droppedOnPc = e.optInt("droppedEncode") + e.optInt("droppedSend")
                $0.diagnostics.queueDepth = e.optInt("queue")
                $0.diagnostics.lostPackets = e.optLong("lostPackets")
                $0.diagnostics.skippedOnPhone = e.optLong("framesSkipped")
            }
        case "rtt":
            let p: VideoPump? = locked {
                guard connectGate.isCurrent(epoch) else { return nil }
                rttUs = e.optLong("rttUs")
                offsetUs = e.optLong("offsetUs")
                return pump
            }
            p?.setClockOffset(e.optLong("offsetUs"))
        case "cursorShape":
            guard let id = e.long("id"), let w = e.int("width"), let ht = e.int("height"),
                  let hotX = e.int("hotX"), let hotY = e.int("hotY"), w > 0, ht > 0
            else { break }
            guard let px = try? Bridge.cursorPixels(handle: h, shapeId: id), px.count == w * ht * 4,
                  let image = Self.cursorImage(px, width: w, height: ht)
            else { break }
            let entry: CursorShape = (image, hotX, hotY)
            let live: Bool = locked {
                guard connectGate.isCurrent(epoch) else { return false }
                if cursorShapes.count > 32 { cursorShapes.removeAll() }
                cursorShapes[id] = entry
                lastShapeId = id
                lastCursorShape = entry
                return true
            }
            guard live else { return false }
            DispatchQueue.main.async { self.cursorSink?.onCursorShape(bitmap: image, hotX: hotX, hotY: hotY) }
        case "cursor":
            guard let x = e.int("x"), let y = e.int("y"), let vis = e.bool("visible") else { break }
            let shapeId = e.long("shape") ?? -1
            let cached: CursorShape?? = locked {
                guard connectGate.isCurrent(epoch) else { return nil }
                var found: CursorShape?
                if shapeId >= 0 && shapeId != lastShapeId, let s = cursorShapes[shapeId] {
                    lastShapeId = shapeId
                    lastCursorShape = s
                    found = s
                }
                lastCursorPos = (x, y, vis)
                return .some(found)
            }
            guard let cached else { return false }
            DispatchQueue.main.async {
                if let s = cached { self.cursorSink?.onCursorShape(bitmap: s.image, hotX: s.hotX, hotY: s.hotY) }
                self.cursorSink?.onCursorPosition(nx: x, ny: y, visible: vis)
            }
        case "caret":
            guard let vis = e.bool("visible"), let x = e.int("x"), let y = e.int("y"),
                  let w = e.int("w"), let ch = e.int("h")
            else { break }
            DispatchQueue.main.async { self.cursorSink?.onCaret(visible: vis, x: x, y: y, w: w, h: ch) }
        case "notice":
            let msg = e.optString("message")
            update(epoch: epoch) { $0.notice = msg }
            DispatchQueue.main.asyncAfter(deadline: .now() + 6) { [weak self] in
                self?.update { if $0.notice == msg { $0.notice = nil } }
            }
        case "error":
            let code = e.optInt("code")
            let msg = e.optString("message")
            // Session-level errors (busy, denied, capture failed) end the
            // attempt but keep the reason visible.
            update(epoch: epoch) {
                if $0.phase == .streaming && code == 12 {
                    $0.notice = msg
                } else {
                    $0.phase = .failed
                    $0.message = msg
                }
            }
        case "closed":
            onClosed(h, reason: e.optString("message"), epoch: epoch)
            return false
        default:
            break
        }
        return true
    }

    /// The event thread saw the connection end. Closes `h` unless `stop()`
    /// took it first. Reconnects within the same epoch.
    private func onClosed(_ h: Int64, reason: String, epoch: Int) {
        let taken: VideoPump?? = locked {
            if stopped || handle != h || !connectGate.isCurrent(epoch) { return nil }
            handle = 0
            let running = pump
            pump = nil
            eventLoopDone = nil
            clearCursorLocked()
            return .some(running)
        }
        guard let oldPump = taken else { return }
        oldPump?.stop()
        Bridge.close(handle: h)
        let phase = current.phase
        if phase == .failed || phase == .ended { return }
        if reason.contains("stopped on the PC") || reason.contains("revoked") {
            update(epoch: epoch) {
                $0.phase = .ended
                $0.message = reason.contains("revoked") ? "The PC removed this phone." : "The session was stopped on the PC."
            }
            return
        }
        // Network blip, PC asleep, Wi-Fi switch: try to come back.
        let stillCurrent = update(epoch: epoch) {
            $0.phase = .reconnecting
            $0.message = "Connection lost. Reconnecting…"
            $0.reconnects += 1
        }
        // stop() ran in the meantime: no reconnect.
        guard stillCurrent else { return }
        spawn("lanlink-reconnect") { self.connectLoop(first: false, epoch: epoch) }
    }

    private func maybeSendStats(_ h: Int64, epoch: Int) {
        if !isLive(epoch) { return }
        let now = DispatchTime.now().uptimeNanoseconds / 1_000_000
        if now - lastStatsAt < 1000 { return }
        let dt: Float = lastStatsAt == 0 ? 1 : Float(now - lastStatsAt) / 1000
        lastStatsAt = now
        let (p, rtt): (VideoPump?, Int64) = locked { (pump, rttUs) }
        guard let stats = p?.stats else { return }
        let rec = stats.received
        let ren = stats.rendered
        // A new pump starts counting from zero.
        let recFps = Float(max(0, rec - lastReceived)) / dt
        let renFps = Float(max(0, ren - lastRendered)) / dt
        lastReceived = rec
        lastRendered = ren
        // Decode time is not measured on iPhone (see DecoderStats): 0 / 0.
        _ = try? Bridge.sendClientStats(
            handle: h, receivedFpsX10: Int32(clamping: Int(recFps * 10)),
            renderedFpsX10: Int32(clamping: Int(renFps * 10)), decodeP50Us: 0, decodeP95Us: 0,
            dropped: Int32(clamping: stats.dropped), queue: 0)
        let lat = stats.lastCaptureToDecodedUs
        let pipeline = stats.lastHostPipelineUs
        update(epoch: epoch) {
            $0.diagnostics.rttMs = rtt > 0 ? Float(rtt) / 1000 : nil
            $0.diagnostics.receivedFps = recFps
            $0.diagnostics.renderedFps = renFps
            $0.diagnostics.latencyEstimateMs = lat != Int64.min && rtt > 0 ? Float(lat) / 1000 : nil
            $0.diagnostics.latencyUncertaintyMs = rtt > 0 ? Float(rtt) / 2000 : nil
            $0.diagnostics.hostPipelineMs = pipeline > 0 ? Float(pipeline) / 1000 : nil
        }
    }

    // MARK: - Video

    /// The view's display layer appeared (Kotlin's `attachSurface`). Main thread.
    func attachLayer(_ l: AVSampleBufferDisplayLayer) {
        locked { layer = l }
        maybeStartDecoder()
    }

    /// The view's display layer is going away (Kotlin's `detachSurface`). Main thread.
    func detachLayer() {
        locked { layer = nil }
        stopDecoder()
        let (h, viewOnly) = locked { (handle, shadow.viewOnly) }
        if h != 0 && !viewOnly {
            _ = try? releaseSticky(h)
            _ = try? Bridge.releaseAll(handle: h)
        }
    }

    private func maybeStartDecoder() {
        let started: (VideoPump, Int64)? = locked {
            guard let l = layer, handle != 0, pump == nil, !stopped else { return nil }
            let epoch = connectGate.epoch
            let p = VideoPump(handle: handle, layer: l, initialGeometry: videoGeometry) { [weak self] w, h, sw, sh in
                self?.onVideoSize(width: w, height: h, sourceWidth: sw, sourceHeight: sh, epoch: epoch)
            }
            p.setClockOffset(offsetUs)
            pump = p
            return (p, handle)
        }
        guard let s = started else { return }
        let (p, h) = s
        p.start()
        // A new decoder needs a keyframe even if the stream is running.
        _ = try? Bridge.decoderReset(handle: h)
    }

    /// Pump thread. A pump of an old epoch writes nothing.
    private func onVideoSize(width: Int, height: Int, sourceWidth: Int, sourceHeight: Int, epoch: Int) {
        let stillCurrent = update(epoch: epoch) { s in
            self.videoGeometry = [width, height, sourceWidth, sourceHeight]
            s.diagnostics.resolution = "\(width)×\(height)"
        }
        guard stillCurrent else { return }
        DispatchQueue.main.async {
            self.cursorSink?.onVideoGeometry(
                width: width, height: height, sourceWidth: sourceWidth, sourceHeight: sourceHeight)
        }
    }

    private func stopDecoder() {
        let old: VideoPump? = locked {
            let running = pump
            pump = nil
            return running
        }
        guard let old else { return }
        DispatchQueue.global(qos: .userInitiated).async { old.stop() }
    }

    // MARK: - Control

    func selectDisplay(id: Int64) {
        let h = currentHandle
        if h != 0 { _ = try? Bridge.selectDisplay(handle: h, displayId: id) }
        update { $0.displayId = id }
    }

    func applyQuality() {
        let h = currentHandle
        if h == 0 { return }
        let p = streamPlan()
        _ = try? Bridge.setStreamParams(
            handle: h, quality: p.quality, maxW: p.maxW, maxH: p.maxW, fps: p.fps, kbps: p.kbps,
            maxKbps: p.maxKbps)
    }

    func retry() {
        let epoch: Int? = locked {
            if handle != 0 { return nil }
            stopped = false
            // A connect loop still running from before (for example a
            // reconnect when the app went to the background) is now stale.
            return connectGate.advance()
        }
        guard let epoch else { return }
        update { $0.phase = .connecting; $0.message = nil }
        spawn("lanlink-connect") { self.connectLoop(first: true, epoch: epoch) }
    }

    // MARK: - Input

    private func withHandle(_ f: (Int64) throws -> Void) {
        let h: Int64 = locked { shadow.viewOnly ? 0 : handle }
        if h != 0 { _ = try? f(h) }
    }

    func pointer(x: Int, y: Int) {
        withHandle { try Bridge.sendPointer(handle: $0, x: Int32(clamping: x), y: Int32(clamping: y)) }
    }

    func pointerBy(dx: Int, dy: Int) {
        withHandle { try Bridge.sendPointerRel(handle: $0, dx: Int32(clamping: dx), dy: Int32(clamping: dy)) }
    }

    /// Tell the PC the phone keyboard opened/closed so it reports the caret.
    func setTextInput(_ active: Bool) {
        withHandle { _ = try Bridge.setTextInput(handle: $0, active: active) }
    }

    func scroll(dx: Int, dy: Int) {
        withHandle { _ = try Bridge.sendScroll(handle: $0, dx: Int32(clamping: dx), dy: Int32(clamping: dy)) }
    }

    /// x/y < 0 = click where the pointer is.
    func button(_ button: Int, down: Bool, x: Int, y: Int) {
        withHandle { h in
            if down { try pressSticky(h) }
            try Bridge.sendButton(
                handle: h, button: Int32(clamping: button), down: down, x: Int32(clamping: x), y: Int32(clamping: y))
            if !down { try releaseSticky(h) }
        }
    }

    func key(_ vk: Int, down: Bool) {
        withHandle { _ = try Bridge.sendKey(handle: $0, vk: Int32(clamping: vk), down: down) }
    }

    /// A full key press, combined with any sticky modifiers.
    func tapKey(_ vk: Int) {
        withHandle { h in
            try pressSticky(h)
            try Bridge.sendKey(handle: h, vk: Int32(clamping: vk), down: true)
            try Bridge.sendKey(handle: h, vk: Int32(clamping: vk), down: false)
            try releaseSticky(h)
        }
    }

    /// Press keys in order and release in reverse (e.g. Alt+Tab).
    func combo(vks: Int...) {
        withHandle { h in
            for vk in vks { try Bridge.sendKey(handle: h, vk: Int32(clamping: vk), down: true) }
            for vk in vks.reversed() { try Bridge.sendKey(handle: h, vk: Int32(clamping: vk), down: false) }
        }
    }

    /// Committed text from the on-screen keyboard. With a sticky modifier
    /// active (Ctrl, Alt, Win) characters become key presses so shortcuts
    /// like Ctrl+C work; otherwise text is sent as Unicode.
    func text(_ t: String) {
        withHandle { h in
            let shortcut = locked { sticky.contains { $0 != KeyMap.VK_LSHIFT } }
            if !shortcut {
                try Bridge.sendText(handle: h, text: t)
                return
            }
            try pressSticky(h)
            for c in t {
                if let vk = KeyMap.vkForShortcutChar(c: c) {
                    try Bridge.sendKey(handle: h, vk: Int32(vk), down: true)
                    try Bridge.sendKey(handle: h, vk: Int32(vk), down: false)
                } else {
                    try Bridge.sendText(handle: h, text: String(c))
                }
            }
            try releaseSticky(h)
        }
    }

    func toggleSticky(_ vk: Int) {
        update { s in
            if let i = self.sticky.firstIndex(of: vk) {
                self.sticky.remove(at: i)
            } else {
                self.sticky.append(vk)
            }
            s.modifiers = Set(self.sticky)
        }
    }

    private func pressSticky(_ h: Int64) throws {
        for vk in locked({ sticky }) { try Bridge.sendKey(handle: h, vk: Int32(vk), down: true) }
    }

    private func releaseSticky(_ h: Int64) throws {
        var mods: [Int] = []
        update { s in
            mods = self.sticky
            self.sticky.removeAll()
            if !mods.isEmpty { s.modifiers = [] }
        }
        for vk in mods.reversed() { try Bridge.sendKey(handle: h, vk: Int32(vk), down: false) }
    }

    /// Releases everything held on the PC and drops the sticky modifiers
    /// (Android's `detachSurface` does both).
    func releaseAll() {
        clearSticky()
        withHandle { try Bridge.releaseAll(handle: $0) }
    }

    /// Drops the sticky modifiers without sending anything: they are only
    /// pressed on the PC for the duration of one key or click.
    private func clearSticky() {
        update { s in
            self.sticky.removeAll()
            s.modifiers = []
        }
    }

    /// The sticky modifiers in press order (tests).
    var stickyModifiers: [Int] { locked { sticky } }

    // MARK: - Helpers

    private func locked<T>(_ body: () throws -> T) rethrows -> T {
        lock.lock()
        defer { lock.unlock() }
        return try body()
    }

    private var current: SessionState { locked { shadow } }
    private var currentHandle: Int64 { locked { handle } }
    /// Epoch `e` is current and the session is not stopped.
    private func isLive(_ e: Int) -> Bool { locked { !stopped && connectGate.isCurrent(e) } }

    /// Changes the state under the lock and publishes the result on the main
    /// thread. Versions keep the newest state when updates from several
    /// threads reach the main queue out of order. `f` runs under the lock
    /// and may touch other locked fields.
    private func update(_ f: (inout SessionState) -> Void) {
        let (snapshot, version): (SessionState, Int) = locked {
            f(&shadow)
            shadowVersion += 1
            return (shadow, shadowVersion)
        }
        publish(snapshot, version)
    }

    /// `update` for the connect, event and pump threads: changes nothing
    /// and returns false when epoch `e` is no longer current (the check and
    /// the change happen under one lock).
    @discardableResult
    private func update(epoch e: Int, _ f: (inout SessionState) -> Void) -> Bool {
        let changed: (SessionState, Int)? = locked {
            guard connectGate.isCurrent(e) else { return nil }
            f(&shadow)
            shadowVersion += 1
            return (shadow, shadowVersion)
        }
        guard let c = changed else { return false }
        publish(c.0, c.1)
        return true
    }

    private func publish(_ snapshot: SessionState, _ version: Int) {
        DispatchQueue.main.async { [weak self] in
            guard let self, version > self.appliedVersion else { return }
            self.appliedVersion = version
            self.state = snapshot
        }
    }

    /// Caller holds the lock.
    private func clearCursorLocked() {
        videoGeometry = nil
        lastCursorShape = nil
        lastCursorPos = nil
        lastShapeId = -1
    }

    private func spawn(_ name: String, _ body: @escaping () -> Void) {
        let t = Thread(block: body)
        t.name = name
        t.start()
    }

    /// Straight (non-premultiplied) RGBA pixels as an image.
    private static func cursorImage(_ px: Data, width: Int, height: Int) -> CGImage? {
        guard let provider = CGDataProvider(data: px as CFData) else { return nil }
        return CGImage(
            width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
            provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
    }
}
