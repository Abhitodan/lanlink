import AVFoundation
import SwiftUI
import UIKit

// The in-app QR scan (Android uses the ZXing scan activity). Full screen,
// with the same prompt; the first QR payload is returned.

struct QRScannerSheet: View {
    let onResult: (String) -> Void
    let onClose: () -> Void

    @State private var access = AVCaptureDevice.authorizationStatus(for: .video)
    @State private var unavailable = false

    var body: some View {
        Group {
            switch access {
            case .authorized where !unavailable:
                ZStack {
                    QRScanner(onResult: onResult, onUnavailable: { unavailable = true })
                        .ignoresSafeArea()
                    VStack {
                        HStack {
                            Button(action: onClose) {
                                GlyphIcon(.close)
                                    .frame(width: 48, height: 48)
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .foregroundColor(.white)
                            .accessibilityLabel("Close")
                            Spacer()
                        }
                        Spacer()
                        Text("Point at the QR code on your PC")
                            .textStyle(Theme.Typography.bodyLarge)
                            .foregroundColor(.white)
                            .padding(.horizontal, 16)
                            .padding(.vertical, 10)
                            .background(Capsule().fill(Color.black.opacity(0.55)))
                            .padding(.bottom, 32)
                    }
                    .padding(8)
                }
                .background(Color.black.ignoresSafeArea())
            case .notDetermined:
                Color.black.ignoresSafeArea()
                    .task {
                        _ = await AVCaptureDevice.requestAccess(for: .video)
                        access = AVCaptureDevice.authorizationStatus(for: .video)
                    }
            default:
                Page(title: "Scan QR code", onBack: onClose) {
                    if unavailable {
                        Note("The camera is not available on this device. Go back and enter the code instead.")
                    } else {
                        Note(
                            "LANLink is not allowed to use the camera. To scan the code, allow Camera for LANLink "
                                + "in Settings › LANLink, or go back and enter the code instead."
                        )
                    }
                }
            }
        }
    }
}

/// `AVCaptureSession` + `AVCaptureMetadataOutput` (QR only).
struct QRScanner: UIViewControllerRepresentable {
    let onResult: (String) -> Void
    let onUnavailable: () -> Void

    func makeUIViewController(context: Context) -> ScannerViewController {
        let vc = ScannerViewController()
        vc.onResult = onResult
        vc.onUnavailable = onUnavailable
        return vc
    }

    func updateUIViewController(_ vc: ScannerViewController, context: Context) {
        vc.onResult = onResult
        vc.onUnavailable = onUnavailable
    }

    final class ScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
        var onResult: ((String) -> Void)?
        var onUnavailable: (() -> Void)?
        private let session = AVCaptureSession()
        private let sessionQueue = DispatchQueue(label: "lanlink.qrscanner")
        private var preview: AVCaptureVideoPreviewLayer?
        private var delivered = false

        override func viewDidLoad() {
            super.viewDidLoad()
            view.backgroundColor = .black
            guard let device = AVCaptureDevice.default(for: .video),
                  let input = try? AVCaptureDeviceInput(device: device),
                  session.canAddInput(input)
            else {
                DispatchQueue.main.async { [weak self] in self?.onUnavailable?() }
                return
            }
            session.addInput(input)
            let output = AVCaptureMetadataOutput()
            guard session.canAddOutput(output) else {
                DispatchQueue.main.async { [weak self] in self?.onUnavailable?() }
                return
            }
            session.addOutput(output)
            output.setMetadataObjectsDelegate(self, queue: .main)
            if output.availableMetadataObjectTypes.contains(.qr) {
                output.metadataObjectTypes = [.qr]
            }
            let layer = AVCaptureVideoPreviewLayer(session: session)
            layer.videoGravity = .resizeAspectFill
            view.layer.addSublayer(layer)
            preview = layer
        }

        override func viewDidLayoutSubviews() {
            super.viewDidLayoutSubviews()
            preview?.frame = view.bounds
        }

        override func viewWillAppear(_ animated: Bool) {
            super.viewWillAppear(animated)
            let session = session
            sessionQueue.async { if !session.isRunning { session.startRunning() } }
        }

        override func viewWillDisappear(_ animated: Bool) {
            super.viewWillDisappear(animated)
            let session = session
            sessionQueue.async { if session.isRunning { session.stopRunning() } }
        }

        nonisolated func metadataOutput(
            _ output: AVCaptureMetadataOutput, didOutput metadataObjects: [AVMetadataObject],
            from connection: AVCaptureConnection
        ) {
            guard let code = metadataObjects.compactMap({ $0 as? AVMetadataMachineReadableCodeObject })
                      .first(where: { $0.type == .qr }),
                  let payload = code.stringValue
            else { return }
            Task { @MainActor [weak self] in self?.deliver(payload) }
        }

        /// Hands over the first payload only.
        private func deliver(_ payload: String) {
            guard !delivered else { return }
            delivered = true
            let session = session
            sessionQueue.async { session.stopRunning() }
            onResult?(payload)
        }
    }
}
