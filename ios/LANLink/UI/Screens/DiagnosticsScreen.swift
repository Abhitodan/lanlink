import SwiftUI

// Twin of DiagnosticsScreen in android/.../ui/screens/SettingsScreens.kt.

/// Live measurements, as in Kotlin's `Diagnostics` (RemoteSession.kt).
/// nil means "not measured".
struct Diagnostics: Equatable {
    var rttMs: Float? = nil
    var receivedFps: Float? = nil
    var renderedFps: Float? = nil
    var capturedFps: Float? = nil
    var encodedFps: Float? = nil
    var bitrateKbps: Int? = nil
    var targetKbps: Int? = nil
    var decodeP50Ms: Float? = nil
    var decodeP95Ms: Float? = nil
    var encodeP50Ms: Float? = nil
    var encodeP95Ms: Float? = nil
    var droppedOnPc: Int? = nil
    var skippedOnPhone: Int64? = nil
    var queueDepth: Int? = nil
    /// Capture on the PC → decoded on the phone, using the Ping/Pong clock
    /// offset; ± `latencyUncertaintyMs`. An estimate, not a measurement.
    var latencyEstimateMs: Float? = nil
    var latencyUncertaintyMs: Float? = nil
    /// PC side only: capture → handed to the network (single clock).
    var hostPipelineMs: Float? = nil
    var lostPackets: Int64? = nil
    var decoder: String? = nil
    var resolution: String? = nil
}

/// What the screen reads from a live session's state (`SessionState` on
/// Android: diagnostics, reconnects, encoder, capture).
struct DiagnosticsReadout: Equatable {
    var diagnostics = Diagnostics()
    var reconnects = 0
    var encoder = ""
    var capture = ""
}

struct DiagnosticsScreen: View {
    @ObservedObject var model: AppModel
    /// nil without a session (see `LiveDiagnosticsScreen`).
    let readout: DiagnosticsReadout?

    var body: some View {
        Page(title: "Performance", onBack: { model.back() }) {
            if let st = readout {
                let d = st.diagnostics
                SectionLabel("Network")
                Card {
                    MetricRow("Round trip", value: ms(d.rttMs), hint: "Ping to the PC and back")
                    MetricRow("Video bitrate", value: d.bitrateKbps.map { mbps($0) })
                    MetricRow("Target bitrate", value: d.targetKbps.map { mbps($0) }, hint: "Adapts to network conditions")
                    MetricRow("Packets lost (total)", value: d.lostPackets.map { String($0) })
                    MetricRow("Reconnections", value: String(st.reconnects))
                }
                SectionLabel("PC")
                Card {
                    MetricRow("Captured", value: fps(d.capturedFps), hint: "Changes on the PC screen; a still screen sends no frames")
                    MetricRow("Encoded", value: fps(d.encodedFps))
                    MetricRow("Encode time (median / 95%)", value: d.encodeP50Ms.map { pair($0, d.encodeP95Ms) })
                    MetricRow("Capture → sent", value: ms(d.hostPipelineMs), hint: "Measured on the PC's own clock")
                    MetricRow("Frames dropped", value: d.droppedOnPc.map { String($0) }, hint: "Last second")
                    MetricRow("Send queue", value: d.queueDepth.map { String($0) })
                    MetricRow("Encoder", value: st.encoder.isEmpty ? nil : st.encoder)
                    MetricRow("Capture", value: st.capture.isEmpty ? nil : st.capture)
                }
                SectionLabel("Phone")
                Card {
                    MetricRow("Received", value: fps(d.receivedFps))
                    MetricRow("Displayed", value: fps(d.renderedFps))
                    MetricRow("Decode time (median / 95%)", value: d.decodeP50Ms.map { pair($0, d.decodeP95Ms) })
                    MetricRow("Frames skipped", value: d.skippedOnPhone.map { String($0) }, hint: "Waiting for a keyframe after loss")
                    MetricRow("Resolution", value: d.resolution)
                    MetricRow("Decoder", value: d.decoder)
                }
                SectionLabel("End to end")
                Card {
                    MetricRow(
                        "PC capture → phone decoded",
                        value: d.latencyEstimateMs.map {
                            String(format: "≈ %.0f ms ± %.0f", $0, d.latencyUncertaintyMs ?? 0)
                        },
                        hint: "Estimated: the two clocks are aligned using the round trip, so the uncertainty is half the round trip.")
                    MetricRow("Touch → visible on phone", value: nil, hint: "Needs a high-speed camera; not measured in the app")
                }
            } else {
                Note("Connect to a PC to see live measurements.")
            }
        }
    }

    private func ms(_ v: Float?) -> String? { v.map { String(format: "%.1f ms", $0) } }
    private func fps(_ v: Float?) -> String? { v.map { String(format: "%.0f fps", $0) } }
    private func mbps(_ kbps: Int) -> String { String(format: "%.1f Mb/s", Double(kbps) / 1000) }
    private func pair(_ p50: Float, _ p95: Float?) -> String {
        String(format: "%.1f / %.1f ms", p50, p95 ?? 0)
    }
}

/// `DiagnosticsScreen` fed from the live session's state, as Kotlin
/// collects `vm.session?.state`.
struct LiveDiagnosticsScreen: View {
    @ObservedObject var model: AppModel

    var body: some View {
        if let session = model.remote {
            SessionDiagnostics(model: model, session: session)
        } else {
            DiagnosticsScreen(model: model, readout: nil)
        }
    }
}

private struct SessionDiagnostics: View {
    let model: AppModel
    @ObservedObject var session: RemoteSession

    var body: some View {
        let st = session.state
        DiagnosticsScreen(
            model: model,
            readout: DiagnosticsReadout(
                diagnostics: st.diagnostics, reconnects: st.reconnects, encoder: st.encoder, capture: st.capture))
    }
}
