import SwiftUI

// Twin of PairScreen in android/.../ui/screens/PairScreen.kt.

struct PairScreen: View {
    @ObservedObject var model: AppModel
    @State private var address: String
    @State private var port: String
    @State private var code = ""
    @State private var manual: Bool
    @State private var scanning = false

    init(model: AppModel, prefillAddress: String, prefillPort: Int) {
        self.model = model
        _address = State(initialValue: prefillAddress)
        _port = State(initialValue: String(prefillPort))
        _manual = State(initialValue: !prefillAddress.isEmpty)
    }

    var body: some View {
        Page(title: "Pair a PC", onBack: { model.cancelPairing(); model.back() }) {
            switch model.pair {
            case .idle, .failed:
                idle
            case let .offer(name, addresses, _, fingerprint, _):
                Text("Pair with \(name.isEmpty ? "this PC" : name)?")
                    .textStyle(Theme.Typography.headlineMedium)
                Note("A pairing link was opened. Continue only if you just chose \"Pair a phone\" on your own PC.")
                Card {
                    Text("Address: \(addresses.joined(separator: ", "))")
                        .textStyle(Theme.Typography.bodyMedium)
                    Text("PC identity: \(grouped(String(fingerprint.prefix(16)), by: 4))")
                        .font(.system(size: 14, design: .monospaced))
                        .lineSpacing(3)
                }
                PrimaryButton("Pair") { model.acceptOffer() }
                SecondaryButton("Cancel") {
                    model.cancelPairing()
                    model.back()
                }
            case let .working(stage):
                CenteredProgress(stage)
            case let .confirm(code):
                Text("Check the PC").textStyle(Theme.Typography.headlineMedium)
                Note("The PC is asking whether to trust this phone. Approve it only if the PC shows this same number:")
                Text(grouped(code, by: 3))
                    .font(.system(size: 44, weight: .semibold, design: .monospaced))
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity)
                    .accessibilityLabel("Confirmation number \(code.map(String.init).joined(separator: " "))")
                CenteredProgress("Waiting for approval on the PC…")
                SecondaryButton("Cancel") { model.cancelPairing() }
            case let .done(host):
                Text("Paired with \(host.name)").textStyle(Theme.Typography.headlineMedium)
                Note("This phone can now connect to the PC whenever both are on the same network.")
                PrimaryButton("Connect now", icon: .monitor) {
                    model.cancelPairing()
                    model.replace(.home)
                    model.connect(host)
                }
                SecondaryButton("Done") {
                    model.cancelPairing()
                    model.replace(.home)
                }
            }
        }
        .onDisappear {
            // Leaving the screen withdraws a pairing in progress (and so the
            // PC's prompt), unless it already finished. Presenting the
            // scanner also counts as disappearing; that is not leaving.
            if scanning { return }
            if case .done = model.pair { return }
            model.cancelPairing()
        }
        .fullScreenCover(isPresented: $scanning) {
            QRScannerSheet(
                onResult: { payload in
                    scanning = false
                    model.pairFromUri(payload)
                },
                onClose: { scanning = false })
        }
    }

    @ViewBuilder
    private var idle: some View {
        if case let .failed(message) = model.pair {
            Card {
                Text("Pairing did not complete")
                    .textStyle(Theme.Typography.titleMedium)
                    .foregroundColor(Theme.error)
                Note(message)
            }
        }
        Note("On the PC, open LANLink and choose \"Pair a phone\". A QR code and an 8-digit code appear for two minutes.")
        PrimaryButton("Scan QR code", icon: .qrScan) { scanning = true }
        if !manual {
            SecondaryButton("Enter the code instead") { manual = true }
        } else {
            SectionLabel("Enter manually")
            OutlinedField(
                "PC address (for example 192.168.1.20)",
                text: Binding(get: { address }, set: { address = $0.trimmingCharacters(in: .whitespaces) }),
                keyboard: .URL)
            HStack(alignment: .top, spacing: 12) {
                OutlinedField(
                    "8-digit code",
                    text: Binding(get: { code }, set: { code = String(digits($0).prefix(8)) }),
                    keyboard: .numberPad)
                OutlinedField(
                    "Port",
                    text: Binding(get: { port }, set: { port = String(digits($0).prefix(5)) }),
                    keyboard: .numberPad)
                    .frame(width: 110)
            }
            PrimaryButton(
                "Pair",
                enabled: !address.trimmingCharacters(in: .whitespaces).isEmpty && code.count == 8
            ) {
                model.pairManual(address: address, port: Int(port) ?? 47990, code: code)
            }
            Note("Without the QR code, the phone checks the PC's identity using the code itself and the confirmation number shown next.")
        }
    }

    private func digits(_ s: String) -> String {
        String(s.filter { $0.isASCII && $0.isNumber })
    }

    /// "123456" → "123 456" (Kotlin's `chunked(n).joinToString(" ")`).
    private func grouped(_ s: String, by n: Int) -> String {
        let a = Array(s)
        return stride(from: 0, to: a.count, by: n)
            .map { String(a[$0..<min($0 + n, a.count)]) }
            .joined(separator: " ")
    }
}
