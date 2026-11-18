import SwiftUI
import UIKit

// Twin of android/.../ui/screens/CompactKeyboard.kt: LANLink's compact
// keyboard (see `CompactLayout`): about phone width, centred, four short
// rows, so most of the PC stays visible above and beside it. Keys act on
// press (no waiting for lift) and Backspace/arrows repeat while held.

private let keyColor = Color(hex: 0x2A2F36)
private let keyPressed = Color(hex: 0x4A525C)
private let keyText = Color(hex: 0xE9E6DF)
private let keyInk = Color(hex: 0x15181C)
private let keyboardBackground = Color(hex: 0x15181C).opacity(Double(0xF2) / 255)

/// Delay before a held key (Backspace, arrows) starts repeating, then the repeat interval.
let REPEAT_DELAY_MS: Int64 = 400
let REPEAT_EVERY_MS: Int64 = 55

/// Schedules key repeats; tests drive it by hand.
protocol RepeatClock: AnyObject {
    /// Runs `action` after `ms` milliseconds; the returned closure cancels it.
    func after(ms: Int64, _ action: @escaping () -> Void) -> () -> Void
}

/// The main queue.
final class MainRepeatClock: RepeatClock {
    func after(ms: Int64, _ action: @escaping () -> Void) -> () -> Void {
        let work = DispatchWorkItem(block: action)
        DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(Int(ms)), execute: work)
        return { work.cancel() }
    }
}

/// A key being held; `release()` ends its repeat.
final class KeyHold {
    private var cancel: (() -> Void)?
    private var released = false

    fileprivate init() {}

    fileprivate func schedule(_ c: @escaping () -> Void) {
        if released { c() } else { cancel = c }
    }

    fileprivate var isReleased: Bool { released }

    func release() {
        released = true
        cancel?()
        cancel = nil
    }
}

/// The keyboard's state and key actions: the letter/symbol layer, Shift,
/// and the press-then-repeat of held keys.
final class CompactKeyboardController: ObservableObject {
    @Published private(set) var symbols = false
    /// Changes whenever Shift does, so the key labels redraw.
    @Published private(set) var shiftVersion = 0
    let shift = ShiftState()
    private let clock: RepeatClock
    private let onText: (String) -> Void
    private let onKey: (Int) -> Void
    private let onSticky: (Int) -> Void
    private let onHide: () -> Void

    init(
        clock: RepeatClock = MainRepeatClock(),
        onText: @escaping (String) -> Void,
        onKey: @escaping (Int) -> Void,
        onSticky: @escaping (Int) -> Void,
        onHide: @escaping () -> Void
    ) {
        self.clock = clock
        self.onText = onText
        self.onKey = onKey
        self.onSticky = onSticky
        self.onHide = onHide
    }

    var rows: [[KeySpec]] { CompactLayout.rows(symbols: symbols, shifted: shift.shifted) }

    /// A finger went down on `k`: act now and, for repeating keys, again
    /// after REPEAT_DELAY_MS and every REPEAT_EVERY_MS until released.
    func press(_ k: KeySpec) -> KeyHold {
        let hold = KeyHold()
        act(k)
        if k.repeats { repeatLater(k, after: REPEAT_DELAY_MS, hold) }
        return hold
    }

    private func repeatLater(_ k: KeySpec, after ms: Int64, _ hold: KeyHold) {
        hold.schedule(clock.after(ms: ms) { [weak self, weak hold] in
            guard let self, let hold, !hold.isReleased else { return }
            self.act(k)
            self.repeatLater(k, after: REPEAT_EVERY_MS, hold)
        })
    }

    private func act(_ k: KeySpec) {
        switch k.action {
        case let .text(t):
            onText(t)
            if !symbols {
                shift.typed()
                shiftVersion += 1
            }
        case let .key(vk):
            onKey(vk)
        case let .sticky(vk):
            onSticky(vk)
        case .shift:
            shift.tap(t: Int64(Date().timeIntervalSince1970 * 1000))
            shiftVersion += 1
        case .layer:
            symbols.toggle()
        case .hide:
            onHide()
        }
    }
}

struct CompactKeyboardView: View {
    let keyHeight: CGFloat
    let sticky: Set<Int>
    /// Bottom safe area (home indicator), padded inside the keyboard.
    let bottomInset: CGFloat
    @StateObject private var controller: CompactKeyboardController

    init(
        keyHeight: CGFloat,
        sticky: Set<Int>,
        bottomInset: CGFloat,
        onText: @escaping (String) -> Void,
        onKey: @escaping (Int) -> Void,
        onSticky: @escaping (Int) -> Void,
        onHide: @escaping () -> Void
    ) {
        self.keyHeight = keyHeight
        self.sticky = sticky
        self.bottomInset = bottomInset
        _controller = StateObject(wrappedValue: CompactKeyboardController(
            onText: onText, onKey: onKey, onSticky: onSticky, onHide: onHide))
    }

    var body: some View {
        // On small landscape screens or with large text, cap the row height
        // so the 4-row keyboard never covers more than ~42% of the screen.
        let screenH = UIScreen.main.bounds.height
        let h = min(keyHeight, max((screenH * 0.42 - 25) / 4, 26))
        let rows = controller.rows
        VStack(spacing: 5) {
            ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                KeyRow(row: row, height: h, sticky: sticky, controller: controller)
            }
        }
        .padding(5)
        .padding(.bottom, bottomInset)
        .frame(maxWidth: 640)
        .background(
            UnevenTop(radius: 18)
                .fill(keyboardBackground)
                .shadow(color: .black.opacity(0.35), radius: 10, y: -2))
        // Touches between the keys stay on the keyboard, off the PC.
        .contentShape(Rectangle())
        .onTapGesture {}
        .frame(maxWidth: .infinity)
    }
}

/// One row: keys share the width by weight, 5 pt apart.
private struct KeyRow: View {
    let row: [KeySpec]
    let height: CGFloat
    let sticky: Set<Int>
    @ObservedObject var controller: CompactKeyboardController

    var body: some View {
        GeometryReader { g in
            let total = row.reduce(Float(0)) { $0 + $1.weight }
            let unit = (g.size.width - 5 * CGFloat(row.count - 1)) / CGFloat(total)
            HStack(spacing: 5) {
                ForEach(Array(row.enumerated()), id: \.offset) { _, k in
                    KeyCap(
                        spec: k,
                        active: isActive(k),
                        locked: k.action == .shift && controller.shift.locked,
                        controller: controller)
                        .frame(width: max(0, unit * CGFloat(k.weight)), height: height)
                }
            }
        }
        .frame(height: height)
    }

    private func isActive(_ k: KeySpec) -> Bool {
        switch k.action {
        case let .sticky(vk): return sticky.contains(vk)
        case .shift: return controller.shift.shifted
        default: return false
        }
    }
}

private struct KeyCap: View {
    let spec: KeySpec
    let active: Bool
    let locked: Bool
    let controller: CompactKeyboardController
    @State var hold: KeyHold?

    private static let tap = UIImpactFeedbackGenerator(style: .light)

    var body: some View {
        let down = hold != nil
        let bg = down ? keyPressed : (active ? Theme.sandstone : keyColor)
        let fg = active && !down ? keyInk : keyText
        let label = spec.action == .text(" ") ? "space" : (locked ? "\u{21EA}" : spec.label)
        RoundedRectangle(cornerRadius: 8, style: .continuous)
            .fill(bg)
            .overlay(
                Text(label)
                    .textStyle(spec.label.count > 1 ? Theme.Typography.labelMedium : Theme.Typography.titleMedium)
                    .foregroundColor(label == "space" ? fg.opacity(0.5) : fg)
                    .lineLimit(1)
                    .minimumScaleFactor(0.7))
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0, coordinateSpace: .local)
                    .onChanged { _ in
                        guard hold == nil else { return }
                        Self.tap.impactOccurred()
                        hold = controller.press(spec)
                    }
                    .onEnded { _ in
                        hold?.release()
                        hold = nil
                    })
            .onDisappear {
                hold?.release()
                hold = nil
            }
            .accessibilityElement()
            .accessibilityLabel(label == "space" ? "Space" : label)
            .accessibilityAddTraits([.isButton, .isKeyboardKey])
            .accessibilityAction { controller.press(spec).release() }
    }
}

/// Rounded top corners only (Compose `RoundedCornerShape(topStart, topEnd)`).
private struct UnevenTop: Shape {
    let radius: CGFloat

    func path(in rect: CGRect) -> Path {
        let r = min(radius, rect.width / 2, rect.height)
        var p = Path()
        p.move(to: CGPoint(x: rect.minX, y: rect.maxY))
        p.addLine(to: CGPoint(x: rect.minX, y: rect.minY + r))
        p.addArc(center: CGPoint(x: rect.minX + r, y: rect.minY + r), radius: r,
                 startAngle: .degrees(180), endAngle: .degrees(270), clockwise: false)
        p.addLine(to: CGPoint(x: rect.maxX - r, y: rect.minY))
        p.addArc(center: CGPoint(x: rect.maxX - r, y: rect.minY + r), radius: r,
                 startAngle: .degrees(270), endAngle: .degrees(0), clockwise: false)
        p.addLine(to: CGPoint(x: rect.maxX, y: rect.maxY))
        p.closeSubpath()
        return p
    }
}
