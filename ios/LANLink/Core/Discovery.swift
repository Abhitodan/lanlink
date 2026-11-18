import Foundation
import Network
import dnssd

/// A PC seen on the network through mDNS. Nothing here is trusted.
struct FoundHost: Equatable {
    let serviceName: String
    let name: String
    let shortId: String
    let address: String
    let port: Int
    let `protocol`: Int
}

enum DiscoveryStatus: Equatable {
    case idle
    case searching
    case failed(String)
    case localNetworkDenied
}

/// Browses `_lanlink._udp` with `NWBrowser` while the host list is visible
/// and stops when it is not. Mirrors android/.../core/Discovery.kt.
@MainActor
final class Discovery: ObservableObject {
    @Published private(set) var hosts: [FoundHost] = []
    @Published private(set) var status: DiscoveryStatus = .idle

    private static let serviceType = "_lanlink._udp"
    private static let resolveTimeout: TimeInterval = 4

    private var browser: NWBrowser?
    private let queue = DispatchQueue(label: "lanlink.discovery")
    /// In-flight address lookups by service name.
    private var resolves: [String: Resolve] = [:]
    /// The browse result (endpoint) each service name is being resolved
    /// from, or was published from. A name is resolved once; it is resolved
    /// again only when this result vanishes or changes.
    private var sources: [String: NWEndpoint] = [:]
    private var generation = 0

    private final class Resolve {
        let connection: NWConnection
        let timeout: DispatchWorkItem
        init(_ c: NWConnection, _ t: DispatchWorkItem) { connection = c; timeout = t }
        func cancel() { timeout.cancel(); connection.cancel() }
    }

    // MARK: Pure helpers

    /// Cleans what a device advertises before it is shown (any device can
    /// advertise any name). An empty result falls back to "PC".
    nonisolated static func parseTXT(_ txt: [String: String], serviceName: String)
        -> (name: String, shortId: String, protocol: Int)
    {
        let cleaned = DisplayText.name(s: txt["n"] ?? serviceName)
        return (cleaned.isEmpty ? "PC" : cleaned,
                txt["id"] ?? "",
                txt["v"].flatMap { Int($0) } ?? 0)
    }

    /// Maps a browser error to a status. Policy-denied / no-auth is what the
    /// system reports when Local Network access is refused.
    nonisolated static func status(for error: NWError) -> DiscoveryStatus {
        if case .dns(let code) = error,
           code == DNSServiceErrorType(kDNSServiceErr_PolicyDenied)
            || code == DNSServiceErrorType(kDNSServiceErr_NoAuth) {
            return .localNetworkDenied
        }
        return .failed("\(error)")
    }

    /// A usable IP literal from a resolved endpoint, or nil (a hostname or
    /// an empty address is not usable). Zone suffixes are dropped.
    nonisolated static func addressString(_ host: NWEndpoint.Host) -> String? {
        let raw: String
        switch host {
        case .ipv4(let a): raw = a.debugDescription
        case .ipv6(let a):
            // The Rust core drops the zone, so a link-local address can
            // never be connected to. Do not publish it.
            if a.isLinkLocal { return nil }
            raw = a.debugDescription
        default: return nil
        }
        let noZone = raw.split(separator: "%", maxSplits: 1, omittingEmptySubsequences: false)
            .first.map(String.init) ?? raw
        let t = noZone.trimmingCharacters(in: CharacterSet(charactersIn: "[] "))
        return t.isEmpty ? nil : t
    }

    /// The service names present in a browse snapshot (endpoints of every
    /// result, however many interfaces report the same name).
    nonisolated static func liveNames(endpoints: [NWEndpoint]) -> Set<String> {
        var names = Set<String>()
        for e in endpoints {
            if case .service(let name, _, _, _) = e { names.insert(name) }
        }
        return names
    }

    // MARK: Start / stop

    func start() {
        if browser != nil { return }
        hosts = []
        sources = [:]
        generation += 1
        let b = NWBrowser(for: .bonjourWithTXTRecord(type: Self.serviceType, domain: nil), using: .udp)
        b.stateUpdateHandler = { [weak self, weak b] state in
            Task { @MainActor in
                guard let self, let b, self.browser === b else { return }
                self.handle(state: state, of: b)
            }
        }
        b.browseResultsChangedHandler = { [weak self, weak b] results, changes in
            Task { @MainActor in
                guard let self, let b, self.browser === b else { return }
                self.handle(results: results, changes: changes)
            }
        }
        browser = b
        status = .searching
        b.start(queue: queue)
    }

    func stop() {
        browser?.cancel()
        browser = nil
        cancelResolves()
        sources = [:]
        generation += 1
        status = .idle
    }

    private func cancelResolves() {
        for r in resolves.values { r.cancel() }
        resolves = [:]
    }

    private func handle(state: NWBrowser.State, of b: NWBrowser) {
        switch state {
        case .ready:
            status = .searching
        case .waiting(let e):
            // Waiting recovers on its own (for example once permission is
            // granted), so keep the browser; only a denial is surfaced.
            let s = Self.status(for: e)
            status = (s == .localNetworkDenied) ? s : .searching
        case .failed(let e):
            // Forget the failed browser so the next start() retries.
            status = Self.status(for: e)
            b.cancel()
            browser = nil
            cancelResolves()
            generation += 1
        default:
            break
        }
    }

    // MARK: Results

    private static func serviceName(_ r: NWBrowser.Result) -> String? {
        if case .service(let name, _, _, _) = r.endpoint { return name }
        return nil
    }

    private func handle(results: Set<NWBrowser.Result>, changes: Set<NWBrowser.Result.Change>) {
        // The snapshot is the truth; nothing is counted, so nothing drifts.
        var byName: [String: [NWBrowser.Result]] = [:]
        for r in results {
            if let n = Self.serviceName(r) { byName[n, default: []].append(r) }
        }
        let live = Self.liveNames(endpoints: results.map { $0.endpoint })

        // Hosts that are gone from the snapshot.
        for n in Array(sources.keys) where !live.contains(n) {
            sources[n] = nil
            resolves.removeValue(forKey: n)?.cancel()
        }
        hosts = hosts.filter { live.contains($0.serviceName) }

        // A changed result is re-resolved when it is the one in use.
        for change in changes {
            if case .changed(let old, let new, _) = change,
               let n = Self.serviceName(new), sources[n] == old.endpoint {
                resolve(new, name: n, v4Only: true)
            }
        }

        for (n, rs) in byName {
            if let src = sources[n], rs.contains(where: { $0.endpoint == src }) { continue }
            // Not yet resolved, or the result we used is gone: use a
            // remaining one, preferring Wi-Fi or wired.
            let best = rs.min { Self.rank($0) < Self.rank($1) }!
            resolve(best, name: n, v4Only: true)
        }
    }

    private static func rank(_ r: NWBrowser.Result) -> Int {
        r.interfaces.contains { $0.type == .wifi || $0.type == .wiredEthernet } ? 0 : 1
    }

    // MARK: Resolving

    /// Connects to the service endpoint only to learn the address the
    /// system resolved, then drops the connection. IPv4 is tried first (as
    /// Android prefers an Inet4Address), then any address.
    private func resolve(_ r: NWBrowser.Result, name: String, v4Only: Bool) {
        resolves.removeValue(forKey: name)?.cancel()
        sources[name] = r.endpoint
        let params = NWParameters.udp
        if v4Only, let ip = params.defaultProtocolStack.internetProtocol as? NWProtocolIP.Options {
            ip.version = .v4
        }
        let conn = NWConnection(to: r.endpoint, using: params)
        let gen = generation
        let txt = Self.txtDictionary(r)

        let timeout = DispatchWorkItem { [weak self, weak conn] in
            Task { @MainActor in
                guard let self, let conn, self.isCurrent(name, conn, gen) else { return }
                self.fail(r, name: name, v4Only: v4Only)
            }
        }
        resolves[name] = Resolve(conn, timeout)
        conn.stateUpdateHandler = { [weak self, weak conn] state in
            Task { @MainActor in
                guard let self, let conn, self.isCurrent(name, conn, gen) else { return }
                switch state {
                case .ready, .preparing:
                    var isReady = false
                    if case .ready = state { isReady = true }
                    if let ep = conn.currentPath?.remoteEndpoint,
                       case .hostPort(let host, let port) = ep,
                       let addr = Self.addressString(host), port.rawValue != 0 {
                        self.resolves.removeValue(forKey: name)?.cancel()
                        self.publish(serviceName: name, txt: txt, address: addr, port: Int(port.rawValue))
                    } else if isReady {
                        // Ready but no usable IP literal: skip this result.
                        self.fail(r, name: name, v4Only: v4Only)
                    }
                case .failed:
                    self.fail(r, name: name, v4Only: v4Only)
                default:
                    break
                }
            }
        }
        conn.start(queue: queue)
        queue.asyncAfter(deadline: .now() + Self.resolveTimeout, execute: timeout)
    }

    private func isCurrent(_ name: String, _ conn: NWConnection, _ gen: Int) -> Bool {
        gen == generation && resolves[name]?.connection === conn
    }

    private func fail(_ r: NWBrowser.Result, name: String, v4Only: Bool) {
        resolves.removeValue(forKey: name)?.cancel()
        if v4Only {
            resolve(r, name: name, v4Only: false)
        } else {
            // Gave up: allow a later browse event to try again.
            sources[name] = nil
        }
    }

    private static func txtDictionary(_ r: NWBrowser.Result) -> [String: String] {
        if case .bonjour(let rec) = r.metadata { return rec.dictionary }
        return [:]
    }

    private func publish(serviceName: String, txt: [String: String], address: String, port: Int) {
        guard sources[serviceName] != nil else { return }
        let p = Self.parseTXT(txt, serviceName: serviceName)
        let h = FoundHost(serviceName: serviceName, name: p.name, shortId: p.shortId,
                          address: address, port: port, protocol: p.protocol)
        hosts = (hosts.filter { $0.serviceName != serviceName } + [h]).sorted { $0.name < $1.name }
    }
}
