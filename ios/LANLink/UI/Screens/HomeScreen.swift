import SwiftUI

// Twin of HomeScreen, PairedHostCard, lastSeen and CenteredProgress in
// android/.../ui/screens/HomeScreens.kt.

struct HomeScreen: View {
    @ObservedObject var model: AppModel
    @ObservedObject var discovery: Discovery

    init(model: AppModel) {
        self.model = model
        self.discovery = model.discovery
    }

    var body: some View {
        let hosts = model.hosts
        let found = discovery.hosts
        Page(title: "LANLink", onBack: nil, actions: {
            Button { model.go(.settings) } label: {
                Image(systemName: "gearshape.fill")
                    .font(.system(size: 22))
                    .frame(width: 48, height: 48)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Settings")
        }) {
            if !hosts.isEmpty {
                SectionLabel("Your PCs")
                ForEach(hosts, id: \.fingerprint) { h in
                    let live = found.first { !$0.shortId.isEmpty && $0.shortId == h.shortId }
                    PairedHostCard(
                        h: h, live: live,
                        onOpen: { model.go(.host(fingerprint: h.fingerprint)) },
                        onConnect: { model.connect(h, address: live?.address) })
                }
            }

            let unpaired = found.filter { f in !hosts.contains { $0.shortId == f.shortId } }
            SectionLabel("On this network")
            if !unpaired.isEmpty {
                ForEach(unpaired, id: \.serviceName) { f in
                    Card(onClick: { model.go(.pair(prefillAddress: f.address, prefillPort: f.port)) }) {
                        HStack(spacing: 0) {
                            GlyphIcon(.laptop, size: 22)
                            Spacer().frame(width: 12)
                            VStack(alignment: .leading, spacing: 0) {
                                Text(f.name).textStyle(Theme.Typography.titleMedium)
                                Text("Not paired · tap to pair")
                                    .textStyle(Theme.Typography.labelMedium)
                                    .foregroundColor(Theme.onSurfaceVariant)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                }
            } else if case .failed = discovery.status {
                Note(
                    "Automatic discovery is not available on this network (some routers block it). "
                        + "You can still pair by scanning the QR code or entering the PC's address."
                )
            } else if discovery.status == .localNetworkDenied {
                // iOS only: Local Network access was refused.
                Note(
                    "LANLink is not allowed to look for PCs on this network. Turn on Local Network for LANLink "
                        + "in Settings › Privacy & Security › Local Network. You can still pair by scanning the "
                        + "QR code or entering the PC's address."
                )
            } else {
                HStack(spacing: 0) {
                    ProgressView()
                        .tint(Theme.primary)
                        .scaleEffect(0.7)
                        .frame(width: 16, height: 16)
                    Spacer().frame(width: 12)
                    Note(hosts.isEmpty ? "Looking for PCs running LANLink…" : "Looking for other PCs…")
                }
            }

            PrimaryButton("Pair a PC", icon: .add) { model.go(.pair()) }
            Note("Not seeing your PC? Both devices must be on the same Wi-Fi, and guest networks often block devices from seeing each other.")
        }
        .onAppear {
            // Browse only while this screen is visible.
            model.refreshHosts()
            discovery.start()
        }
        .onDisappear { discovery.stop() }
    }
}

/// A paired PC. The card opens the PC's page; Connect is its own button
/// (not nested inside the card's button, so both stay tappable and
/// visible to accessibility).
private struct PairedHostCard: View {
    let h: PairedHost
    let live: FoundHost?
    let onOpen: () -> Void
    let onConnect: () -> Void

    var body: some View {
        Card {
            HStack(spacing: 0) {
                GlyphIcon(.laptop, size: 24)
                    .foregroundColor(Theme.primary)
                Spacer().frame(width: 12)
                VStack(alignment: .leading, spacing: 0) {
                    Text(h.name).textStyle(Theme.Typography.titleMedium)
                    HStack(spacing: 0) {
                        StatusDot(live != nil ? Theme.ok : Theme.outline)
                        Spacer().frame(width: 6)
                        Text(live != nil ? "Available" : lastSeen(h))
                            .textStyle(Theme.Typography.labelMedium)
                            .foregroundColor(Theme.onSurfaceVariant)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Button("Connect", action: onConnect)
                    .buttonStyle(TonalButtonStyle())
            }
        }
        .contentShape(Rectangle())
        .onTapGesture(perform: onOpen)
        .accessibilityElement(children: .contain)
        .accessibilityAction(named: Text(h.name), onOpen)
    }
}

/// Material 3 filled tonal button: secondary container, full pill.
struct TonalButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .textStyle(Theme.Typography.labelLarge)
            .foregroundColor(Theme.onSecondaryContainer)
            .padding(.horizontal, 24)
            .frame(minHeight: 40)
            .background(Capsule().fill(Theme.secondaryContainer))
            .contentShape(Capsule())
            .opacity(configuration.isPressed ? 0.85 : 1)
    }
}

/// "Paired, not connected yet" or "Last used <relative time>", like
/// `DateUtils.getRelativeTimeSpanString` with minute resolution.
func lastSeen(_ h: PairedHost) -> String {
    if h.lastConnected <= 0 { return "Paired, not connected yet" }
    let then = Date(timeIntervalSince1970: TimeInterval(h.lastConnected) / 1000)
    let now = Date()
    if now.timeIntervalSince(then) >= 7 * 24 * 3600 {
        let f = DateFormatter()
        f.setLocalizedDateFormatFromTemplate("MMMd")
        return "Last used " + f.string(from: then)
    }
    let f = RelativeDateTimeFormatter()
    f.dateTimeStyle = .named
    f.unitsStyle = .full
    f.formattingContext = .beginningOfSentence
    let span = now.timeIntervalSince(then) < 60 ? "0 minutes ago" : f.localizedString(for: then, relativeTo: now)
    return "Last used " + span
}

/// First 16 hex digits, upper case, in groups of four ("5A3C-9E17-…").
func shortIdentity(_ fingerprint: String) -> String {
    let s = Array(fingerprint.prefix(16).uppercased())
    return stride(from: 0, to: s.count, by: 4)
        .map { String(s[$0..<min($0 + 4, s.count)]) }
        .joined(separator: "-")
}

struct CenteredProgress: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        HStack(spacing: 0) {
            ProgressView()
                .tint(Theme.primary)
                .frame(width: 20, height: 20)
            Spacer().frame(width: 12)
            Text(text).textStyle(Theme.Typography.bodyMedium)
        }
        .frame(maxWidth: .infinity)
        .padding(24)
    }
}

extension Glyph {
    /// Material's "Add" (a plus), drawn on the same 24-unit grid.
    static let add = Glyph(name: "add", unitPath: {
        var p = Path()
        p.move(to: CGPoint(x: 5, y: 12)); p.addLine(to: CGPoint(x: 19, y: 12))
        p.move(to: CGPoint(x: 12, y: 5)); p.addLine(to: CGPoint(x: 12, y: 19))
        return p
    }())
}
