import Foundation
import UIKit

// Swift twin of android/.../ui/MainViewModel.kt: navigation, the pairing
// loop with its friendly errors, paired hosts and the session.

enum Screen: Hashable {
    case onboarding
    case home
    case pair(prefillAddress: String = "", prefillPort: Int = 47990)
    case host(fingerprint: String)
    case remote
    case settings
    case diagnostics
    case security
}

/// Pairing progress shown on the pairing screen.
enum PairState: Equatable {
    case idle
    case working(stage: String)
    /// A pairing link from outside the app, waiting for the user's yes.
    case offer(name: String, addresses: [String], port: Int, fingerprint: String, code: String)
    /// The phone proved the code; waiting for the PC user.
    case confirm(code: String)
    case done(host: PairedHost)
    case failed(message: String)
}

/// What the model needs from a remote session. `RemoteSession` conforms to
/// it; tests pass a recorder.
protocol SessionControl: AnyObject {
    func start()
    func stop()
}

@MainActor
final class AppModel: ObservableObject {
    let settings: Settings
    /// Paired PCs on disk (`app.hosts` on Android).
    let repository: HostRepository
    let identity: IdentityStore
    let discovery: Discovery
    private let makeSession: (PairedHost, String?) -> SessionControl

    @Published private(set) var screen: Screen
    @Published private(set) var pair: PairState = .idle
    @Published private(set) var hosts: [PairedHost]
    @Published private(set) var sessionVersion = 0
    private(set) var session: SessionControl?

    /// The live session for the remote and diagnostics screens (Kotlin's
    /// `vm.session`); nil without one. Views re-read it on `sessionVersion`.
    var remote: RemoteSession? { session as? RemoteSession }

    private var stack: [Screen] = []
    private var pairTask: Task<Void, Never>?
    private var pairHandle: Int64 = 0

    init(
        settings: Settings = Settings(),
        repository: HostRepository = HostRepository(),
        identity: IdentityStore = IdentityStore(),
        discovery: Discovery? = nil,
        makeSession: ((PairedHost, String?) -> SessionControl)? = nil
    ) {
        self.settings = settings
        self.repository = repository
        self.identity = identity
        let discovery = discovery ?? Discovery()
        self.discovery = discovery
        if let makeSession {
            self.makeSession = makeSession
        } else {
            self.makeSession = { host, address -> SessionControl in
                RemoteSession(
                    host: host, preferredAddress: address, settings: settings,
                    hosts: repository, identity: identity,
                    localNetworkDenied: { discovery.localNetworkDenied })
            }
        }
        screen = settings.onboarded ? .home : .onboarding
        hosts = repository.all()
    }

    func refreshHosts() {
        hosts = repository.all()
    }

    // MARK: - Navigation

    func go(_ s: Screen) {
        stack.append(screen)
        screen = s
    }

    /// Replace the current screen (no back entry).
    func replace(_ s: Screen) {
        screen = s
    }

    @discardableResult
    func back() -> Bool {
        if screen == .remote {
            endSession()
        }
        guard let prev = stack.popLast() else { return false }
        screen = prev
        return true
    }

    func finishOnboarding(next: Screen) {
        settings.onboarded = true
        stack.removeAll()
        screen = .home
        if next != .home { go(next) }
    }

    // MARK: - Pairing

    /// Handle a QR code the user just scanned in the app.
    func pairFromUri(_ uri: String) {
        guard case let .offer(name, addresses, port, fingerprint, code)? = parseOffer(uri) else { return }
        startPairing(addresses: addresses, port: port, fingerprint: fingerprint, code: code, nameHint: name)
    }

    /// A `lanlink://pair` link opened from elsewhere: ask before connecting.
    func offerPairFromLink(_ uri: String) {
        cancelPairing()
        if let offer = parseOffer(uri) { pair = offer }
    }

    func acceptOffer() {
        guard case let .offer(name, addresses, port, fingerprint, code) = pair else { return }
        settings.onboarded = true
        startPairing(addresses: addresses, port: port, fingerprint: fingerprint, code: code, nameHint: name)
    }

    /// The parsed link as an `.offer`, or nil after setting `.failed`.
    private func parseOffer(_ uri: String) -> PairState? {
        do {
            let json = try Bridge.parsePairingUri(uri)
            guard let o = Self.jsonObject(json),
                  let addresses = o["addresses"] as? [String],
                  let port = o["port"] as? Int,
                  let fingerprint = o["fingerprint"] as? String,
                  let code = o["code"] as? String
            else {
                pair = .failed(message: "That is not a LANLink pairing code.")
                return nil
            }
            return .offer(
                name: o["name"] as? String ?? "", addresses: addresses, port: port,
                fingerprint: fingerprint, code: code)
        } catch {
            pair = .failed(message: (error as? LANLinkError)?.message ?? "That is not a LANLink pairing code.")
            return nil
        }
    }

    func pairManual(address: String, port: Int, code: String) {
        startPairing(
            addresses: [address.trimmingCharacters(in: .whitespacesAndNewlines)],
            port: port, fingerprint: "", code: code, nameHint: "")
    }

    private func startPairing(addresses: [String], port: Int, fingerprint: String, code: String, nameHint: String) {
        cancelPairing()
        let deviceName = settings.deviceName
        let deviceModel = UIDevice.current.model
        let identityStore = identity
        pairTask = Task { [weak self] in
            if Task.isCancelled { return }
            let identity: Data
            do {
                identity = try await Self.offMain { try identityStore.get(deviceLabel: deviceName) }
            } catch {
                guard let self, !Task.isCancelled else { return }
                self.pair = .failed(message: "Could not create this phone's identity: \(Self.message(error))")
                return
            }
            var lastError = "The PC did not answer."
            for addr in addresses {
                guard let self, !Task.isCancelled else { return }
                self.pair = .working(stage: "Connecting to \(nameHint.isEmpty ? addr : nameHint)…")
                let h: Int64
                do {
                    h = try await Self.offMain {
                        try Bridge.pairStart(
                            identity: identity, deviceName: deviceName, deviceModel: deviceModel,
                            host: addr, port: Int32(clamping: port), fingerprintHex: fingerprint, code: code)
                    }
                } catch {
                    guard !Task.isCancelled else { return }
                    self.pair = .failed(message: (error as? LANLinkError)?.message ?? "Invalid pairing details.")
                    return
                }
                if Task.isCancelled {
                    // Cancelled while the job was starting: stop it here.
                    Bridge.pairCancel(job: h)
                    return
                }
                self.pairHandle = h
                while !Task.isCancelled {
                    // Throws only when the job was cancelled meanwhile.
                    guard let json = try? Bridge.pairPoll(job: h), let o = Self.jsonObject(json) else { return }
                    switch o["state"] as? String {
                    case "confirm":
                        self.pair = .confirm(code: o["code"] as? String ?? "")
                    case "done":
                        let hostName = o["hostName"] as? String ?? ""
                        var addrs: [String] = []
                        for a in [addr] + addresses where !addrs.contains(a) { addrs.append(a) }
                        let host = PairedHost(
                            fingerprint: o["fingerprint"] as? String ?? "",
                            name: !hostName.isEmpty ? hostName : (!nameHint.isEmpty ? nameHint : "PC"),
                            addresses: addrs,
                            port: port,
                            pairedAt: Int64(Date().timeIntervalSince1970 * 1000),
                            lastConnected: 0)
                        self.repository.put(host)
                        self.refreshHosts()
                        self.pair = .done(host: host)
                        self.pairHandle = 0
                        return
                    case "failed":
                        lastError = o["error"] as? String ?? ""
                        self.pairHandle = 0
                    default:
                        break // "connecting"
                    }
                    if self.pairHandle == 0 { break }
                    do {
                        try await Task.sleep(nanoseconds: 200_000_000)
                    } catch {
                        return
                    }
                }
                if Task.isCancelled { return }
                // Try the next advertised address only for reachability
                // failures, never after the PC answered.
                if !lastError.contains("reach") && !lastError.contains("did not answer") { break }
            }
            guard let self, !Task.isCancelled else { return }
            self.pair = .failed(
                message: Self.friendlyPairError(lastError, localNetworkDenied: self.discovery.localNetworkDenied))
        }
    }

    /// With Local Network access denied (iOS only), a failure that is not
    /// an answer from the PC points to the permission instead of Wi-Fi and
    /// the firewall: QR and address pairing need the local network too.
    nonisolated static func friendlyPairError(_ e: String, localNetworkDenied: Bool = false) -> String {
        if e.contains("wrong") {
            return "The code was not accepted. Check the 8-digit code on the PC and try again."
        }
        if e.contains("declined") {
            return "Pairing was declined on the PC."
        }
        if e.contains("not open") {
            return "Pairing is not open on the PC. Choose \"Pair a phone\" on the PC first."
        }
        if e.contains("identity mismatch") {
            return "The PC at that address is not the one in the QR code. Pairing stopped for your safety."
        }
        if e.contains("unverified") {
            return "That PC says it already knows this phone, but without its QR code LANLink cannot check "
                + "that it is your PC. Scan the QR code on the PC instead."
        }
        if localNetworkDenied { return LocalNetworkText.cannotReach("the PC") }
        if e.contains("did not answer") || e.contains("reach") {
            return "Cannot reach the PC. Make sure both are on the same Wi-Fi and Windows Firewall allows LANLink."
        }
        return e
    }

    func cancelPairing() {
        pairTask?.cancel()
        pairTask = nil
        if pairHandle != 0 { Bridge.pairCancel(job: pairHandle) }
        pairHandle = 0
        pair = .idle
    }

    // MARK: - Hosts

    func forget(_ fp: String) {
        repository.remove(fp)
        refreshHosts()
    }

    func clearCredentials() {
        endSession()
        repository.clear()
        identity.clear()
        refreshHosts()
    }

    // MARK: - Session

    func connect(_ host: PairedHost, address: String? = nil) {
        endSession()
        let s = makeSession(host, address)
        session = s
        sessionVersion += 1
        s.start()
        go(.remote)
    }

    func endSession() {
        session?.stop()
        session = nil
        sessionVersion += 1
        refreshHosts()
    }

    // MARK: - Helpers

    private nonisolated static func jsonObject(_ json: String) -> [String: Any]? {
        guard let data = json.data(using: .utf8) else { return nil }
        return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
    }

    private nonisolated static func message(_ error: Error) -> String {
        (error as? LANLinkError)?.message ?? error.localizedDescription
    }

    /// Runs blocking work (Keychain, key generation, the core) off the main
    /// thread, as the Kotlin loop runs on `Dispatchers.IO`.
    private nonisolated static func offMain<T: Sendable>(
        _ work: @escaping @Sendable () throws -> T
    ) async throws -> T {
        try await Task.detached(priority: .userInitiated) { try work() }.value
    }
}
