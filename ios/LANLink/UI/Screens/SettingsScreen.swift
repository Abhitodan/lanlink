import SwiftUI

// Twin of SettingsScreen and NumberField in
// android/.../ui/screens/SettingsScreens.kt, plus the Material pieces it
// uses (FilterChip, OutlinedTextField) rebuilt to look the same.

struct SettingsScreen: View {
    @ObservedObject var model: AppModel
    @State private var quality: Quality
    @State private var mode: PointerMode
    @State private var sens: Float
    @State private var accel: Float
    @State private var fill: Bool
    @State private var landscape: Bool
    @State private var controlsLeft: Bool
    @State private var allowShots: Bool
    @State private var compactKeyboard: Bool
    @State private var keyHeight: Float
    @State private var startZoom: Float
    @State private var awake: Bool
    @State private var stats: Bool
    @State private var viewOnly: Bool
    @State private var fps: String
    @State private var kbps: String
    @State private var maxW: String
    @State private var cap: String
    @State private var name: String

    init(model: AppModel) {
        self.model = model
        let s = model.settings
        _quality = State(initialValue: s.quality)
        _mode = State(initialValue: s.pointerMode)
        _sens = State(initialValue: s.sensitivity)
        _accel = State(initialValue: s.acceleration)
        _fill = State(initialValue: s.fillScreen)
        _landscape = State(initialValue: s.landscape)
        _controlsLeft = State(initialValue: s.controlsLeft)
        _allowShots = State(initialValue: s.allowScreenshots)
        _compactKeyboard = State(initialValue: s.compactKeyboard)
        _keyHeight = State(initialValue: s.keyHeight)
        _startZoom = State(initialValue: s.startZoom)
        _awake = State(initialValue: s.keepScreenOn)
        _stats = State(initialValue: s.showStats)
        _viewOnly = State(initialValue: s.viewOnly)
        _fps = State(initialValue: s.customFps == 0 ? "" : String(s.customFps))
        _kbps = State(initialValue: s.customKbps == 0 ? "" : String(s.customKbps))
        _maxW = State(initialValue: s.customMaxWidth == 0 ? "" : String(s.customMaxWidth))
        _cap = State(initialValue: s.maxKbps == 0 ? "" : String(s.maxKbps))
        _name = State(initialValue: s.deviceName)
    }

    private var s: Settings { model.settings }

    /// Applies the stream settings to a running session.
    private func applyLive() { model.remote?.applyQuality() }

    var body: some View {
        Page(title: "Settings", onBack: { model.back() }) {
            SectionLabel("Stream quality")
            HStack(spacing: 8) {
                ForEach([Quality.smooth, .balanced, .sharp, .custom], id: \.self) { q in
                    FilterChip(q.label, selected: quality == q) {
                        quality = q
                        s.quality = q
                        applyLive()
                    }
                }
            }
            Note(qualityNote)
            if quality == .custom {
                NumberField("Frame rate (fps)", text: $fps) { s.customFps = Int($0) ?? 0 }
                NumberField("Bitrate (kbps)", text: $kbps) { s.customKbps = Int($0) ?? 0 }
                NumberField("Maximum width (px)", text: $maxW) { s.customMaxWidth = Int($0) ?? 0 }
                PrimaryButton("Apply to current session") { applyLive() }
            }
            NumberField("Bandwidth limit (kbps, optional)", text: $cap) {
                s.maxKbps = Int($0) ?? 0
                applyLive()
            }

            SectionLabel("Touch and pointer")
            HStack(spacing: 8) {
                FilterChip("Direct touch", icon: .touch, selected: mode == .direct) {
                    mode = .direct
                    s.pointerMode = mode
                }
                FilterChip("Trackpad", icon: .mouse, selected: mode == .trackpad) {
                    mode = .trackpad
                    s.pointerMode = mode
                }
            }
            Note(
                mode == .direct
                    ? "Tap to click. Long-press to drag. Two fingers scroll; pinch to zoom; two-finger tap for right click."
                    : "Slide to move the pointer. Tap to click; double-tap and hold to drag. Two fingers scroll; two-finger tap for right click; three-finger tap for the keyboard."
            )
            Text("Trackpad sensitivity: \(String(format: "%.1f", sens))×").textStyle(Theme.Typography.bodyMedium)
            slider($sens, 0.3...3) { s.sensitivity = $0 }
            Text("Pointer acceleration: \(String(format: "%.1f", accel))").textStyle(Theme.Typography.bodyMedium)
            slider($accel, 0...2) { s.acceleration = $0 }
            Text("Trackpad starts zoomed in: \(String(format: "%.1f", startZoom))×").textStyle(Theme.Typography.bodyMedium)
            slider($startZoom, 1...4) { s.startZoom = $0 }
            Note("The view follows the pointer when it nears the edge, so you never have to pan by hand.")

            SectionLabel("Display")
            ToggleRow("Turn sideways while controlling", isOn: bind($landscape) { s.landscape = $0 })
            ToggleRow("Controls on the left edge", isOn: bind($controlsLeft) { s.controlsLeft = $0 })

            SectionLabel("Keyboard")
            ToggleRow("Compact LANLink keyboard", isOn: bind($compactKeyboard) { s.compactKeyboard = $0 })
            if compactKeyboard {
                Text("Key size: \(Int(keyHeight)) dp").textStyle(Theme.Typography.bodyMedium)
                slider($keyHeight, 28...48) { s.keyHeight = $0 }
                Note("Phone-width with Esc, Tab, Ctrl, Alt and arrows built in. Turn off to use your phone's own keyboard (swipe typing, other languages).")
            }
            ToggleRow("Fill the screen (crop edges)", isOn: bind($fill) { s.fillScreen = $0 })
            ToggleRow("Keep phone screen on during a session", isOn: bind($awake) { s.keepScreenOn = $0 })
            ToggleRow("Show performance overlay", isOn: bind($stats) { s.showStats = $0 })
            ToggleRow("View only (do not control the PC)", isOn: bind($viewOnly) { s.viewOnly = $0 })

            SectionLabel("Privacy")
            ToggleRow(
                "Allow the PC screen in recordings and the app switcher",
                isOn: bind($allowShots) { s.allowScreenshots = $0 })
            Note("Off: the PC screen is covered in the app switcher and blanked while the screen is recorded or mirrored. iOS cannot block screenshots.")

            SectionLabel("This phone")
            OutlinedField(
                "Name shown on the PC",
                text: Binding(get: { name }, set: { v in
                    name = String(v.prefix(48))
                    s.deviceName = name
                }))

            SectionLabel("More")
            SecondaryButton("Security and paired PCs", icon: .shield) { model.go(.security) }
            SecondaryButton("Performance diagnostics", icon: .gauge) { model.go(.diagnostics) }
            Note("Audio and clipboard sharing are not available in this version.")
        }
    }

    private var qualityNote: String {
        switch quality {
        case .smooth: return "Prioritises responsiveness and steady motion; lower resolution."
        case .balanced: return "Clear text and smooth motion at the phone's screen resolution."
        case .sharp: return "Full PC resolution for fine text, especially when zooming in. Uses more bandwidth."
        case .custom: return "Set your own limits. Empty fields are decided automatically."
        }
    }

    private func bind(_ state: Binding<Bool>, _ save: @escaping (Bool) -> Void) -> Binding<Bool> {
        Binding(get: { state.wrappedValue }, set: { state.wrappedValue = $0; save($0) })
    }

    private func slider(
        _ value: Binding<Float>, _ range: ClosedRange<Float>, _ save: @escaping (Float) -> Void
    ) -> some View {
        Slider(value: Binding(get: { value.wrappedValue }, set: { value.wrappedValue = $0; save($0) }), in: range)
            .tint(Theme.primary)
    }
}

/// A digits-only field (at most 6) that saves every change.
struct NumberField: View {
    let label: String
    @Binding var text: String
    let onChange: (String) -> Void

    init(_ label: String, text: Binding<String>, onChange: @escaping (String) -> Void) {
        self.label = label
        _text = text
        self.onChange = onChange
    }

    var body: some View {
        OutlinedField(
            label,
            text: Binding(get: { text }, set: { v in
                text = String(v.filter { $0.isASCII && $0.isNumber }.prefix(6))
                onChange(text)
            }),
            keyboard: .numberPad)
    }
}

/// Material 3 filter chip: 32 pt, small radius, outlined when off and
/// filled with the secondary container when on.
struct FilterChip: View {
    let label: String
    var icon: Glyph? = nil
    let selected: Bool
    let action: () -> Void

    init(_ label: String, icon: Glyph? = nil, selected: Bool, action: @escaping () -> Void) {
        self.label = label
        self.icon = icon
        self.selected = selected
        self.action = action
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Theme.Radius.small, style: .continuous)
        Button(action: action) {
            HStack(spacing: 8) {
                if let icon { GlyphIcon(icon, size: 18) }
                Text(label).textStyle(Theme.Typography.labelLarge).lineLimit(1)
            }
            .padding(.leading, icon == nil ? 16 : 8)
            .padding(.trailing, 16)
            .frame(minHeight: 32)
            .foregroundColor(selected ? Theme.onSecondaryContainer : Theme.onSurfaceVariant)
            .background(shape.fill(selected ? Theme.secondaryContainer : Color.clear))
            .overlay(shape.strokeBorder(selected ? Color.clear : Theme.outline, lineWidth: 1))
            .contentShape(shape)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// Material 3 outlined text field: 56 pt, outline border (primary, 2 pt,
/// when focused) and a label that moves onto the border once there is text.
struct OutlinedField: View {
    let label: String
    @Binding var text: String
    var keyboard: UIKeyboardType = .default
    @FocusState private var focused: Bool

    init(_ label: String, text: Binding<String>, keyboard: UIKeyboardType = .default) {
        self.label = label
        _text = text
        self.keyboard = keyboard
    }

    var body: some View {
        let floating = focused || !text.isEmpty
        let shape = RoundedRectangle(cornerRadius: 4, style: .continuous)
        ZStack(alignment: .leading) {
            shape.strokeBorder(focused ? Theme.primary : Theme.outline, lineWidth: focused ? 2 : 1)
            TextField("", text: $text)
                .textStyle(Theme.Typography.bodyLarge)
                .foregroundColor(Theme.onSurface)
                .keyboardType(keyboard)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .focused($focused)
                .padding(.horizontal, 16)
                .accessibilityLabel(label)
            Text(label)
                .textStyle(floating ? Theme.Typography.labelMedium : Theme.Typography.bodyLarge)
                .foregroundColor(focused ? Theme.primary : Theme.onSurfaceVariant)
                .lineLimit(1)
                .padding(.horizontal, 4)
                .background(floating ? Theme.background : Color.clear)
                .padding(.leading, 12)
                .offset(y: floating ? -28 : 0)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        }
        .frame(height: 56)
        .animation(.easeOut(duration: 0.15), value: floating)
    }
}
