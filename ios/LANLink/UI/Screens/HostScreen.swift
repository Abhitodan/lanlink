import SwiftUI

// Twin of HostScreen in android/.../ui/screens/HomeScreens.kt.

struct HostScreen: View {
    @ObservedObject var model: AppModel
    @ObservedObject var discovery: Discovery
    let fingerprint: String

    init(model: AppModel, fingerprint: String) {
        self.model = model
        self.discovery = model.discovery
        self.fingerprint = fingerprint
    }

    var body: some View {
        if let h = model.hosts.first(where: { $0.fingerprint == fingerprint }) {
            details(h, live: discovery.hosts.first { $0.shortId == h.shortId })
        } else {
            Page(title: "PC", onBack: { model.back() }) {
                Note("This PC is no longer paired.")
            }
        }
    }

    private func details(_ h: PairedHost, live: FoundHost?) -> some View {
        Page(title: h.name, onBack: { model.back() }) {
            Card {
                HStack(spacing: 0) {
                    StatusDot(live != nil ? Theme.ok : Theme.outline)
                    Spacer().frame(width: 8)
                    Text(live != nil ? "Available on this network" : "Not seen on this network right now")
                        .textStyle(Theme.Typography.bodyMedium)
                }
                Text(lastSeen(h))
                    .textStyle(Theme.Typography.labelMedium)
                    .foregroundColor(Theme.onSurfaceVariant)
            }
            PrimaryButton("Connect", icon: .monitor) { model.connect(h, address: live?.address) }
            SectionLabel("Details")
            Card {
                MetricRow("Identity", value: shortIdentity(h.fingerprint))
                MetricRow("Addresses", value: distinct([live?.address].compactMap { $0 } + h.addresses).joined(separator: ", "))
                MetricRow("Port", value: String(h.port))
            }
            Note("Displays and stream quality can be changed while connected.")
            SectionLabel("Manage")
            SecondaryButton("Connection settings", icon: .quality) { model.go(.settings) }
            SecondaryButton("Forget this PC", danger: true) {
                model.forget(h.fingerprint)
                model.back()
            }
            Note("Forgetting removes the PC from this phone. To stop the PC trusting this phone, remove it on the PC as well.")
        }
    }

    private func distinct(_ list: [String]) -> [String] {
        var out: [String] = []
        for a in list where !out.contains(a) { out.append(a) }
        return out
    }
}
