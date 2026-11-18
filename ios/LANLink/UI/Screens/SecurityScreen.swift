import SwiftUI

// Twin of SecurityScreen in android/.../ui/screens/SettingsScreens.kt.

struct SecurityScreen: View {
    @ObservedObject var model: AppModel
    @State private var confirmClear = false
    @State private var fp: String?
    @State private var loaded = false

    var body: some View {
        Page(title: "Security", onBack: { model.back() }) {
            SectionLabel("This phone's identity")
            Card {
                Text(fp.map(shortIdentity) ?? (loaded ? "Not created yet" : " "))
                    .textStyle(Theme.Typography.titleMedium)
                Note("PCs recognise this phone by this identity. Its private key is protected by the iOS Keychain and never leaves the phone.")
            }
            SectionLabel("Paired PCs")
            if model.hosts.isEmpty { Note("No paired PCs.") }
            ForEach(model.hosts, id: \.fingerprint) { h in
                Card {
                    HStack(spacing: 0) {
                        VStack(alignment: .leading, spacing: 0) {
                            Text(h.name).textStyle(Theme.Typography.titleMedium)
                            Text(shortIdentity(h.fingerprint))
                                .textStyle(Theme.Typography.labelMedium)
                                .foregroundColor(Theme.onSurfaceVariant)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        Button { model.forget(h.fingerprint) } label: {
                            Text("Forget")
                                .textStyle(Theme.Typography.labelLarge)
                                .foregroundColor(Theme.error)
                                .padding(.horizontal, 12)
                                .frame(minHeight: 40)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            SectionLabel("Sessions")
            Note("Every session is encrypted end to end on your network. The PC shows when it is being controlled and can stop the session at any time. To require approval on the PC for every connection, enable it in LANLink on the PC.")
            Note("Clipboard sharing is not implemented in this version, so nothing you copy is sent between devices.")
            SectionLabel("Reset")
            SecondaryButton("Remove identity and all paired PCs", danger: true) { confirmClear = true }
        }
        .onAppear {
            guard !loaded else { return }
            fp = try? model.identity.fingerprint(deviceLabel: model.settings.deviceName)
            loaded = true
        }
        .alert("Remove everything?", isPresented: $confirmClear) {
            Button("Remove", role: .destructive) {
                model.clearCredentials()
                // The identity is gone; a new one is made on next use.
                fp = nil
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This phone will forget all PCs and create a new identity. You will need to pair again, and PCs will see this phone as new.")
        }
        .tint(Theme.primary)
    }
}
