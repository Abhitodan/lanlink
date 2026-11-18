import AVFoundation
import Combine
import SwiftUI
import UIKit

// Twin of android/.../ui/screens/RemoteScreen.kt: the remote view with the
// control rail and panel, the shortcut bar, the compact keyboard and the
// stats, notice and status overlays.

private let scrim = Color(hex: 0x15181C).opacity(Double(0xCC) / 255)
private let onScrim = Color(hex: 0xE9E6DF)
private let ink = Color(hex: 0x15181C)

/// What the remote screen needs from a session. Input goes only through
/// `RemoteInput` (ruling R3); the rest is state, the video layer and the
/// session controls. `RemoteSession` conforms; UI tests use a still demo.
protocol RemoteScreenSession: RemoteInput, SessionControl, ObservableObject
where ObjectWillChangePublisher == ObservableObjectPublisher {
    var state: SessionState { get }
    var cursorSink: CursorSink? { get set }
    /// A picture shown in place of the video (UI tests only).
    var stillFrame: CGImage? { get }
    func attachLayer(_ l: AVSampleBufferDisplayLayer)
    func detachLayer()
    func selectDisplay(id: Int64)
    func applyQuality()
    func retry()
}

extension RemoteSession: RemoteScreenSession {
    var stillFrame: CGImage? { nil }
}

struct RemoteScreen: View {
    @ObservedObject var model: AppModel

    var body: some View {
        if let s = model.remote {
            RemoteContent(model: model, session: s)
        } else if let demo = model.session as? DemoRemoteSession {
            RemoteContent(model: model, session: demo)
        } else {
            Color.black.ignoresSafeArea().onAppear { model.back() }
        }
    }
}

/// Holds the live UIKit view for the controls that call into it.
private final class RemoteViewRef: ObservableObject {
    weak var view: RemoteView?
}

/// Height of the system keyboard over the screen (Compose `WindowInsets.ime`).
private final class KeyboardObserver: ObservableObject {
    @Published private(set) var height: CGFloat = 0
    private var tokens: [NSObjectProtocol] = []

    init() {
        let c = NotificationCenter.default
        tokens.append(c.addObserver(
            forName: UIResponder.keyboardWillChangeFrameNotification, object: nil, queue: .main
        ) { [weak self] n in self?.update(n) })
        tokens.append(c.addObserver(
            forName: UIResponder.keyboardWillHideNotification, object: nil, queue: .main
        ) { [weak self] _ in self?.height = 0 })
    }

    deinit {
        for t in tokens { NotificationCenter.default.removeObserver(t) }
    }

    private func update(_ n: Notification) {
        guard let end = (n.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue else { return }
        let screen = (n.object as? UIScreen) ?? UIScreen.main
        height = max(0, screen.bounds.height - end.minY)
    }
}

private struct CompactHeightKey: PreferenceKey {
    static var defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

private struct BarHeightKey: PreferenceKey {
    static var defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

private struct PanelTimer: Equatable {
    let open: Bool
    let poke: Int
}

private struct RemoteContent<S: RemoteScreenSession>: View {
    let model: AppModel
    @ObservedObject var session: S
    @StateObject private var ref = RemoteViewRef()
    @StateObject private var keyboard = KeyboardObserver()
    @State private var panel = false
    @State private var poke = 0
    @State private var zoom: Float = 1
    @State private var actualZoom: Float = 1
    @State private var pointerMode: PointerMode
    @State private var showStats: Bool
    @State private var quality: Quality
    // LANLink's compact keyboard (default) or the phone's own keyboard.
    @State private var compactOpen = false
    @State private var compactHeight: CGFloat = 0
    @State private var barHeight: CGFloat = 0

    init(model: AppModel, session: S) {
        self.model = model
        _session = ObservedObject(wrappedValue: session)
        _pointerMode = State(initialValue: model.settings.pointerMode)
        _showStats = State(initialValue: model.settings.showStats)
        _quality = State(initialValue: model.settings.quality)
    }

    private var settings: Settings { model.settings }
    private var imeVisible: Bool { keyboard.height > 0 }

    /// The picture keeps its size under the keyboard; the view slides so the
    /// place being typed into stays visible (see RemoteView.setKeyboardInset).
    private var covered: CGFloat {
        if compactOpen { return compactHeight }
        if imeVisible { return keyboard.height + barHeight }
        return 0
    }

    var body: some View {
        let st = session.state
        GeometryReader { g in
            let safeBottom = g.safeAreaInsets.bottom
            ZStack {
                RemoteViewHost(
                    session: session, settings: settings, ref: ref, covered: covered,
                    onZoom: { z, az in
                        DispatchQueue.main.async {
                            if zoom != z { zoom = z }
                            if actualZoom != az { actualZoom = az }
                        }
                    },
                    onThreeFingerTap: { toggleKeyboard() })
                    .ignoresSafeArea()

                ControlRail(
                    left: settings.controlsLeft,
                    keyboardOpen: imeVisible || compactOpen,
                    bottomInset: max(0, (compactOpen ? compactHeight : keyboard.height) - safeBottom),
                    pointerMode: pointerMode,
                    zoom: zoom,
                    moreOpen: panel,
                    onKeyboard: {
                        panel = false
                        toggleKeyboard()
                    },
                    onMode: switchMode,
                    onZoom: {
                        if zoom < 1.05 {
                            ref.view?.zoomTo(2)
                        } else if (1.9...2.1).contains(zoom) {
                            ref.view?.zoomActualPixels()
                        } else {
                            ref.view?.zoomFit()
                        }
                    },
                    onMore: {
                        panel.toggle()
                        poke += 1
                    }
                ) { controlPanel(st) }

                if showStats {
                    StatsOverlay(d: st.diagnostics)
                        .padding(.top, 56)
                        .padding(.leading, 8)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                        .allowsHitTesting(false)
                }

                if let n = st.notice {
                    Text(n)
                        .textStyle(Theme.Typography.bodyMedium)
                        .foregroundColor(onScrim)
                        .padding(12)
                        .background(RoundedRectangle(cornerRadius: Theme.Radius.medium).fill(scrim))
                        .padding(.horizontal, 16)
                        .padding(.bottom, 96)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
                        .allowsHitTesting(false)
                }

                if compactOpen {
                    VStack(spacing: 0) {
                        Spacer(minLength: 0)
                        CompactKeyboardView(
                            keyHeight: CGFloat(settings.keyHeight),
                            sticky: st.modifiers,
                            bottomInset: safeBottom,
                            onText: { session.text($0) },
                            onKey: { session.tapKey($0) },
                            onSticky: { session.toggleSticky($0) },
                            onHide: { compactOpen = false })
                            .background(GeometryReader { k in
                                Color.clear.preference(key: CompactHeightKey.self, value: k.size.height)
                            })
                    }
                    .ignoresSafeArea(edges: .bottom)
                    .onPreferenceChange(CompactHeightKey.self) { compactHeight = $0 }
                }

                if imeVisible && !compactOpen {
                    VStack(spacing: 0) {
                        Spacer(minLength: 0)
                        ShortcutBar(
                            active: st.modifiers,
                            onToggle: { session.toggleSticky($0) },
                            onKey: { session.tapKey($0) },
                            onCombo: { vks in sendCombo(vks) })
                            .background(GeometryReader { b in
                                Color.clear.preference(key: BarHeightKey.self, value: b.size.height)
                            })
                        Color.clear.frame(height: keyboard.height)
                    }
                    .ignoresSafeArea(edges: .bottom)
                    .onPreferenceChange(BarHeightKey.self) { barHeight = $0 }
                }

                statusOverlay(st)
            }
        }
        .ignoresSafeArea(.keyboard)
        .background(Color.black.ignoresSafeArea())
        .statusBarHidden(true)
        .persistentSystemOverlays(.hidden)
        .toolbar(.hidden, for: .navigationBar)
        .onAppear {
            UIApplication.shared.isIdleTimerDisabled = settings.keepScreenOn
            // The PC is landscape: turning the phone sideways roughly
            // doubles the size of everything on it.
            if settings.landscape { OrientationLock.set(.landscape) }
        }
        .onDisappear {
            OrientationLock.set(.allButUpsideDown)
            UIApplication.shared.isIdleTimerDisabled = false
        }
        // The control panel closes itself after a while without use.
        .task(id: PanelTimer(open: panel, poke: poke)) {
            guard panel else { return }
            try? await Task.sleep(nanoseconds: 8_000_000_000)
            if !Task.isCancelled { panel = false }
        }
        .onChange(of: compactOpen) { open in
            if !open { compactHeight = 0 }
        }
        .onChange(of: imeVisible) { visible in
            if !visible { barHeight = 0 }
        }
    }

    private func sendCombo(_ vks: [Int]) {
        switch vks.count {
        case 2: session.combo(vks: vks[0], vks[1])
        default: for vk in vks { session.tapKey(vk) }
        }
    }

    private func toggleKeyboard() {
        if settings.compactKeyboard {
            if imeVisible || ref.view?.keyboardVisible == true { ref.view?.hideKeyboard() }
            compactOpen.toggle()
        } else {
            compactOpen = false
            ref.view?.toggleKeyboard()
        }
    }

    private func switchMode() {
        pointerMode = pointerMode == .direct ? .trackpad : .direct
        settings.pointerMode = pointerMode
        ref.view?.pointerMode = pointerMode
    }

    private func close() {
        // Leaving the remote screen ends the session (AppModel.back).
        model.back()
    }

    private func controlPanel(_ st: SessionState) -> some View {
        ControlPanel(
            hostName: st.hostName,
            pointerMode: pointerMode,
            zoom: zoom,
            actualZoom: actualZoom,
            displays: st.displays,
            currentDisplay: st.displayId,
            quality: quality,
            onPoke: { poke += 1 },
            onMode: switchMode,
            onZoomFit: { ref.view?.zoomFit() },
            onZoom2: { ref.view?.zoomTo(2) },
            onZoomActual: { ref.view?.zoomActualPixels() },
            onDisplay: { session.selectDisplay(id: $0) },
            onQuality: { q in
                quality = q
                settings.quality = q
                session.applyQuality()
            },
            onStats: {
                showStats.toggle()
                settings.showStats = showStats
            },
            onDiagnostics: { model.go(.diagnostics) },
            onDisconnect: close)
    }

    @ViewBuilder
    private func statusOverlay(_ st: SessionState) -> some View {
        switch st.phase {
        case .connecting, .starting, .reconnecting:
            StatusOverlay(
                title: st.phase == .reconnecting ? "Reconnecting to \(st.hostName)…"
                    : st.phase == .starting ? "Starting the stream…" : "Connecting to \(st.hostName)…",
                message: st.message, progress: true, primary: ("Cancel", close), secondary: nil)
        case .failed:
            StatusOverlay(
                title: "Not connected", message: st.message, progress: false,
                primary: ("Try again", { session.retry() }), secondary: ("Close", close))
        case .ended:
            StatusOverlay(
                title: "Session ended", message: st.message, progress: false,
                primary: ("Reconnect", { session.retry() }), secondary: ("Close", close))
        case .streaming:
            EmptyView()
        }
    }
}

/// Hosts `RemoteView` and wires it to the session: the cursor sink, the
/// display layer (`attachLayer`, Kotlin's `attachSurface`) and the privacy
/// guard's background and foreground rules.
private struct RemoteViewHost<S: RemoteScreenSession>: UIViewRepresentable {
    let session: S
    let settings: Settings
    let ref: RemoteViewRef
    let covered: CGFloat
    let onZoom: (Float, Float) -> Void
    let onThreeFingerTap: () -> Void

    final class Coordinator {
        let session: S
        var lastCovered: CGFloat = -1

        init(session: S) { self.session = session }
    }

    func makeCoordinator() -> Coordinator { Coordinator(session: session) }

    func makeUIView(context: Context) -> RemoteView {
        let v = RemoteView(input: session)
        v.pointerMode = settings.pointerMode
        v.setSensitivity(settings.sensitivity, settings.acceleration)
        v.initialZoom = settings.pointerMode == .trackpad ? settings.startZoom : 1
        v.fill = settings.fillScreen
        v.privacy.allowScreenshots = settings.allowScreenshots
        // iOS suspends a background app: end the connection (held keys are
        // released first) and come back on return.
        v.privacy.onBackground = { [weak s = session] in s?.stop() }
        v.privacy.onForeground = { [weak s = session] in s?.retry() }
        v.onZoomChanged = onZoom
        v.onThreeFingerTap = onThreeFingerTap
        if let still = session.stillFrame { v.showStill(still) }
        session.cursorSink = v
        session.attachLayer(v.displayLayer)
        ref.view = v
        return v
    }

    func updateUIView(_ v: RemoteView, context: Context) {
        v.onZoomChanged = onZoom
        v.onThreeFingerTap = onThreeFingerTap
        if context.coordinator.lastCovered != covered {
            context.coordinator.lastCovered = covered
            v.setKeyboardInset(covered)
        }
    }

    static func dismantleUIView(_ v: RemoteView, coordinator: Coordinator) {
        let s = coordinator.session
        if s.cursorSink === v { s.cursorSink = nil }
        s.detachLayer()
        v.shutdown()
    }
}

private struct RailTimer: Equatable {
    let shown: Bool
    let poke: Int
    let more: Bool
    let keyboard: Bool
}

/// Controls docked to one edge of the screen (right by default: under the
/// right thumb, and on the far side from the camera cutout in the usual
/// landscape orientation). After a few seconds without use the rail slides
/// away to a thin tab; tapping the tab brings it back. It stays out while
/// the keyboard or the "more" panel is open, so both can always be closed.
private struct ControlRail<More: View>: View {
    let left: Bool
    let keyboardOpen: Bool
    let bottomInset: CGFloat
    let pointerMode: PointerMode
    let zoom: Float
    let moreOpen: Bool
    let onKeyboard: () -> Void
    let onMode: () -> Void
    let onZoom: () -> Void
    let onMore: () -> Void
    @ViewBuilder let more: () -> More
    @State var shown = true
    @State var poke = 0

    var body: some View {
        let out = shown || moreOpen || keyboardOpen
        let edge: Alignment = left ? .leading : .trailing
        ZStack(alignment: edge) {
            if moreOpen {
                // Tap anywhere outside the panel to close it.
                Color.clear
                    .contentShape(Rectangle())
                    .onTapGesture { onMore() }
                more().padding(.horizontal, 76)
            }
            if out {
                rail.transition(.move(edge: left ? .leading : .trailing).combined(with: .opacity))
            } else {
                Button {
                    shown = true
                    poke += 1
                } label: {
                    ZStack {
                        // Rounded on the side facing the screen only.
                        RoundedRectangle(cornerRadius: 12, style: .continuous).fill(scrim)
                            .frame(width: 32, height: 76)
                            .offset(x: left ? -6 : 6)
                            .frame(width: 20, height: 76)
                            .clipped()
                        RoundedRectangle(cornerRadius: 2).fill(onScrim.opacity(0.6)).frame(width: 3, height: 30)
                    }
                    .frame(width: 20, height: 76)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Show controls")
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: edge)
        .padding(.bottom, bottomInset)
        .animation(.easeInOut(duration: 0.25), value: out)
        .task(id: RailTimer(shown: shown, poke: poke, more: moreOpen, keyboard: keyboardOpen)) {
            guard shown && !moreOpen && !keyboardOpen else { return }
            try? await Task.sleep(nanoseconds: 4_000_000_000)
            if !Task.isCancelled { shown = false }
        }
    }

    private var rail: some View {
        let trackpad = pointerMode == .trackpad
        return VStack(spacing: 2) {
            RailButton(
                glyph: .keyboard, label: keyboardOpen ? "Hide keyboard" : "Show keyboard", active: keyboardOpen
            ) {
                poke += 1
                onKeyboard()
            }
            RailButton(
                glyph: trackpad ? .mouse : .touch,
                label: trackpad ? "Trackpad mode (tap for touch)" : "Touch mode (tap for trackpad)"
            ) {
                poke += 1
                onMode()
            }
            Button {
                poke += 1
                onZoom()
            } label: {
                Text(zoom < 1.05 ? "Fit" : String(format: "%.1f×", zoom))
                    .textStyle(Theme.Typography.labelLarge)
                    .lineLimit(1)
                    .foregroundColor(onScrim)
                    .frame(width: 48, height: 48)
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .padding(.horizontal, 4)
            .accessibilityLabel("Zoom: Fit, 2×, 1:1")
            RailButton(glyph: .menu, label: "More controls", active: moreOpen) {
                poke += 1
                onMore()
            }
        }
        .padding(.vertical, 6)
        .background(RoundedRectangle(cornerRadius: 26, style: .continuous).fill(scrim).shadow(radius: 6))
        .padding(.horizontal, 8)
    }
}

private struct RailButton: View {
    let glyph: Glyph
    let label: String
    var active = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            GlyphIcon(glyph, size: 22)
                .foregroundColor(active ? ink : onScrim)
                .frame(width: 48, height: 48)
                .background(Circle().fill(active ? Theme.sandstone : Color.clear))
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 4)
        .accessibilityLabel(label)
    }
}

private struct ControlPanel: View {
    let hostName: String
    let pointerMode: PointerMode
    let zoom: Float
    let actualZoom: Float
    let displays: [DisplayItem]
    let currentDisplay: Int64
    let quality: Quality
    let onPoke: () -> Void
    let onMode: () -> Void
    let onZoomFit: () -> Void
    let onZoom2: () -> Void
    let onZoomActual: () -> Void
    let onDisplay: (Int64) -> Void
    let onQuality: (Quality) -> Void
    let onStats: () -> Void
    let onDiagnostics: () -> Void
    let onDisconnect: () -> Void

    var body: some View {
        let isFit = zoom < 1.05
        let is2x = (1.95...2.05).contains(zoom)
        let isActual = !isFit && !is2x && abs(zoom - actualZoom) < 0.05
        VStack(alignment: .leading, spacing: 6) {
            Text(hostName)
                .textStyle(Theme.Typography.labelLarge)
                .foregroundColor(onScrim)
                .lineLimit(1)
                .padding(.leading, 6)
                .padding(.top, 2)
            HStack(spacing: 6) {
                PanelChip(glyph: .mouse, label: "Trackpad", selected: pointerMode == .trackpad) {
                    onPoke()
                    if pointerMode != .trackpad { onMode() }
                }
                PanelChip(glyph: .touch, label: "Touch", selected: pointerMode == .direct) {
                    onPoke()
                    if pointerMode != .direct { onMode() }
                }
            }
            Text("Zoom \(String(format: "%.1f", zoom))×")
                .textStyle(Theme.Typography.labelMedium)
                .foregroundColor(onScrim.opacity(0.7))
                .padding(.leading, 6)
            HStack(spacing: 6) {
                PanelChip(glyph: nil, label: "Fit", selected: isFit) { onPoke(); onZoomFit() }
                PanelChip(glyph: nil, label: "2×", selected: is2x) { onPoke(); onZoom2() }
                PanelChip(glyph: nil, label: "1:1", selected: isActual) { onPoke(); onZoomActual() }
            }
            HStack(spacing: 2) {
                if displays.count > 1 {
                    Menu {
                        ForEach(displays) { d in
                            Button((d.id == currentDisplay ? "✓ " : "") + d.name) { onDisplay(d.id) }
                        }
                    } label: {
                        ToolIcon(glyph: .monitor, tint: onScrim)
                    }
                    .simultaneousGesture(TapGesture().onEnded { onPoke() })
                    .accessibilityLabel("Switch display")
                }
                Menu {
                    ForEach([Quality.smooth, .balanced, .sharp], id: \.self) { q in
                        Button((q == quality ? "✓ " : "") + q.label) { onQuality(q) }
                    }
                    Button("Diagnostics…") { onDiagnostics() }
                } label: {
                    ToolIcon(glyph: .quality, tint: onScrim)
                }
                .simultaneousGesture(TapGesture().onEnded { onPoke() })
                .accessibilityLabel("Stream quality")
                Button {
                    onPoke()
                    onStats()
                } label: {
                    ToolIcon(glyph: .gauge, tint: onScrim)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Performance overlay")
                Button(action: onDisconnect) {
                    ToolIcon(glyph: .power, tint: Color(hex: 0xE5A39B))
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Disconnect")
            }
        }
        .padding(10)
        .frame(maxWidth: 300, alignment: .leading)
        .fixedSize(horizontal: true, vertical: false)
        .background(RoundedRectangle(cornerRadius: 20, style: .continuous).fill(scrim).shadow(radius: 8))
    }
}

private struct ToolIcon: View {
    let glyph: Glyph
    let tint: Color

    var body: some View {
        GlyphIcon(glyph, size: 22)
            .foregroundColor(tint)
            .frame(width: 48, height: 48)
            .contentShape(Rectangle())
    }
}

private struct PanelChip: View {
    let glyph: Glyph?
    let label: String
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                if let glyph { GlyphIcon(glyph, size: 18) }
                Text(label).textStyle(Theme.Typography.labelLarge).padding(.vertical, 12)
            }
            .padding(.horizontal, 14)
            .frame(minHeight: 44)
            .foregroundColor(selected ? ink : onScrim)
            .background(
                RoundedRectangle(cornerRadius: Theme.Radius.medium, style: .continuous)
                    .fill(selected ? Theme.sandstone : Color.white.opacity(Double(0x26) / 255)))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

private struct ShortcutBar: View {
    let active: Set<Int>
    let onToggle: (Int) -> Void
    let onKey: (Int) -> Void
    let onCombo: ([Int]) -> Void
    @State var fn = false

    // Same keys and order as the Android bar.
    private static let keys = [("Esc", KeyMap.VK_ESCAPE), ("Tab", KeyMap.VK_TAB)]
    private static let sticky = [
        ("Ctrl", KeyMap.VK_LCONTROL), ("Alt", KeyMap.VK_LMENU), ("Shift", KeyMap.VK_LSHIFT), ("Win", KeyMap.VK_LWIN),
    ]
    private static let arrows = [
        ("←", KeyMap.VK_LEFT), ("↑", KeyMap.VK_UP), ("↓", KeyMap.VK_DOWN), ("→", KeyMap.VK_RIGHT),
        ("Del", KeyMap.VK_DELETE),
    ]
    private static let combos = [
        ("Alt+Tab", [KeyMap.VK_LMENU, KeyMap.VK_TAB]), ("Win+D", [KeyMap.VK_LWIN, 0x44]),
        ("Ctrl+C", [KeyMap.VK_LCONTROL, 0x43]), ("Ctrl+V", [KeyMap.VK_LCONTROL, 0x56]),
        ("Ctrl+Z", [KeyMap.VK_LCONTROL, 0x5A]),
    ]
    private static let paging = [
        ("Home", KeyMap.VK_HOME), ("End", KeyMap.VK_END), ("PgUp", KeyMap.VK_PRIOR), ("PgDn", KeyMap.VK_NEXT),
    ]

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 4) {
                ForEach(Self.keys.indices, id: \.self) { i in KeyChip(label: Self.keys[i].0) { onKey(Self.keys[i].1) } }
                ForEach(Self.sticky.indices, id: \.self) { i in
                    let k = Self.sticky[i]
                    KeyChip(label: k.0, active: active.contains(k.1)) { onToggle(k.1) }
                }
                ForEach(Self.arrows.indices, id: \.self) { i in KeyChip(label: Self.arrows[i].0) { onKey(Self.arrows[i].1) } }
                KeyChip(label: "Fn", active: fn) { fn.toggle() }
                if fn {
                    ForEach(0..<12, id: \.self) { i in KeyChip(label: "F\(i + 1)") { onKey(KeyMap.VK_F1 + i) } }
                }
                ForEach(Self.combos.indices, id: \.self) { i in KeyChip(label: Self.combos[i].0) { onCombo(Self.combos[i].1) } }
                ForEach(Self.paging.indices, id: \.self) { i in KeyChip(label: Self.paging[i].0) { onKey(Self.paging[i].1) } }
            }
            .padding(.horizontal, 6)
            .padding(.vertical, 4)
        }
        .frame(maxWidth: .infinity)
        .background(scrim)
    }
}

private struct KeyChip: View {
    let label: String
    var active = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(label)
                .textStyle(Theme.Typography.labelLarge)
                .padding(.vertical, 10)
                .padding(.horizontal, 12)
                .frame(minHeight: 40)
                .foregroundColor(active ? ink : onScrim)
                .background(
                    RoundedRectangle(cornerRadius: Theme.Radius.small, style: .continuous)
                        .fill(active ? Theme.sandstone : Color.white.opacity(0.2)))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

private struct StatsOverlay: View {
    let d: Diagnostics

    private func f(_ v: Float?, _ unit: String, _ digits: Int = 0) -> String {
        v.map { String(format: "%.\(digits)f", $0) + unit } ?? "–"
    }

    var body: some View {
        let rate = d.bitrateKbps.map { String("\(Double($0) / 1000.0)".prefix(4)) + " Mb/s" } ?? "–"
        let glass = d.latencyEstimateMs.map {
            String(format: "%.0f±%.0f ms", $0, d.latencyUncertaintyMs ?? 0)
        } ?? "–"
        VStack(alignment: .leading, spacing: 0) {
            Text("rtt \(f(d.rttMs, " ms", 1))   fps \(f(d.renderedFps, ""))")
            Text("\(d.resolution ?? "–")  \(rate)")
            Text("enc \(f(d.encodeP50Ms, " ms", 1))  dec \(f(d.decodeP50Ms, " ms", 1))")
            Text("glass≈ " + glass)
        }
        .font(.system(size: 11, weight: .medium, design: .monospaced))
        .foregroundColor(onScrim)
        .padding(8)
        .background(RoundedRectangle(cornerRadius: Theme.Radius.small).fill(Color.black.opacity(Double(0xAA) / 255)))
    }
}

private struct StatusOverlay: View {
    let title: String
    let message: String?
    let progress: Bool
    let primary: (String, () -> Void)
    let secondary: (String, () -> Void)?

    var body: some View {
        ZStack {
            Color(hex: 0x15181C).opacity(Double(0xE6) / 255).ignoresSafeArea()
            VStack(alignment: .leading, spacing: 14) {
                if progress {
                    ProgressView().tint(onScrim).frame(width: 28, height: 28)
                }
                Text(title).textStyle(Theme.Typography.titleLarge).foregroundColor(onScrim)
                if let message {
                    Text(message)
                        .textStyle(Theme.Typography.bodyMedium)
                        .foregroundColor(onScrim.opacity(0.8))
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer().frame(height: 4)
                PrimaryButton(primary.0, action: primary.1)
                if let secondary { SecondaryButton(secondary.0, action: secondary.1) }
            }
            .padding(28)
            .frame(maxWidth: 420)
        }
        // LanLinkTheme(dark = true): the buttons use the Night scheme.
        .environment(\.colorScheme, .dark)
    }
}
