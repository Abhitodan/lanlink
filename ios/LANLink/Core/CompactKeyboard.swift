import Foundation

/// LANLink's own compact keyboard for controlling a PC: phone-width, short
/// keys, no suggestion strip, with the keys a desktop needs (Esc, Tab, Ctrl,
/// Alt, Win, arrows) built in. The system keyboard cannot be resized by an
/// app, and in landscape it covers more than half the screen.
///
/// Pure layout and state; the SwiftUI view in UI draws it.
enum KeyAction: Equatable {
    /// Type this text (Unicode; a sticky Ctrl/Alt/Win turns it into a shortcut).
    case text(String)
    /// Press this Windows virtual key.
    case key(Int)
    /// Toggle a sticky modifier (applies to the next key).
    case sticky(Int)
    case shift
    case layer
    case hide
}

/// One key: what it shows, what it does, and its width relative to a letter.
struct KeySpec: Equatable {
    let label: String
    let action: KeyAction
    let weight: Float
    let repeats: Bool

    init(label: String, action: KeyAction, weight: Float = 1, repeats: Bool = false) {
        self.label = label
        self.action = action
        self.weight = weight
        self.repeats = repeats
    }
}

enum CompactLayout {
    private static func letters(_ row: String, _ shifted: Bool) -> [KeySpec] {
        row.map { ch in
            let c = shifted ? String(ch).uppercased() : String(ch)
            return KeySpec(label: c, action: .text(c))
        }
    }

    private static func chars(_ row: String) -> [KeySpec] {
        row.map { KeySpec(label: String($0), action: .text(String($0))) }
    }

    private static let backspace = KeySpec(label: "\u{232B}", action: .key(KeyMap.VK_BACK), weight: 1.5, repeats: true)
    private static let enter = KeySpec(label: "\u{21B5}", action: .key(KeyMap.VK_RETURN), weight: 1.5)

    /// Rows for the letter layer or the symbols layer.
    static func rows(symbols: Bool, shifted: Bool) -> [[KeySpec]] {
        if !symbols {
            var r0 = letters("qwertyuiop", shifted)
            r0.append(backspace)

            var r1 = [KeySpec(label: "Tab", action: .key(KeyMap.VK_TAB), weight: 1.2)]
            r1 += letters("asdfghjkl", shifted)
            r1.append(enter)

            var r2 = [KeySpec(label: "\u{21E7}", action: .shift, weight: 1.5)]
            r2 += letters("zxcvbnm", shifted)
            r2 += chars(shifted ? "!?" : ",.")
            r2.append(KeySpec(label: "\u{21E7}", action: .shift, weight: 1.2))

            return [r0, r1, r2, bottomRow(symbols: false)]
        } else {
            var r0 = chars("1234567890")
            r0.append(backspace)

            var r1 = [KeySpec(label: "Esc", action: .key(KeyMap.VK_ESCAPE), weight: 1.2)]
            r1 += chars("@#$%&*-+()")
            r1.append(enter)

            var r2 = [KeySpec(label: "Win", action: .sticky(KeyMap.VK_LWIN), weight: 1.5)]
            r2 += chars("_=/\\:;'\"!?")
            r2.append(KeySpec(label: "Del", action: .key(KeyMap.VK_DELETE), weight: 1.2, repeats: true))

            return [r0, r1, r2, bottomRow(symbols: true)]
        }
    }

    private static func arrow(_ label: String, _ vk: Int) -> KeySpec {
        KeySpec(label: label, action: .key(vk), repeats: true)
    }

    /// Layer switch, Ctrl, Alt, space, arrows (all four on the symbols layer), hide.
    private static func bottomRow(symbols: Bool) -> [KeySpec] {
        let arrows: [KeySpec] = symbols
            ? [
                arrow("\u{2190}", KeyMap.VK_LEFT), arrow("\u{2191}", KeyMap.VK_UP),
                arrow("\u{2193}", KeyMap.VK_DOWN), arrow("\u{2192}", KeyMap.VK_RIGHT),
            ]
            : [arrow("\u{2190}", KeyMap.VK_LEFT), arrow("\u{2192}", KeyMap.VK_RIGHT)]
        var row: [KeySpec] = [
            KeySpec(label: symbols ? "abc" : "?123", action: .layer, weight: 1.5),
            KeySpec(label: "Ctrl", action: .sticky(KeyMap.VK_LCONTROL), weight: 1.3),
            KeySpec(label: "Alt", action: .sticky(KeyMap.VK_LMENU), weight: 1.2),
            KeySpec(label: "", action: .text(" "), weight: symbols ? 3 : 5),
        ]
        row += arrows
        row.append(KeySpec(label: "\u{2304}", action: .hide))
        return row
    }
}

/// Shift: one tap = the next letter only; a second tap within
/// [DOUBLE_TAP_MS] = caps lock until tapped again.
final class ShiftState {
    static let DOUBLE_TAP_MS: Int64 = 350

    private(set) var shifted = false
    private(set) var locked = false
    private var lastTap: Int64 = Int64.min / 2

    func tap(t: Int64) {
        if locked {
            locked = false; shifted = false
        } else if shifted && t - lastTap <= ShiftState.DOUBLE_TAP_MS {
            locked = true
        } else if shifted {
            shifted = false
        } else {
            shifted = true
        }
        lastTap = t
    }

    /// A character was typed: a one-shot shift ends, caps lock stays.
    func typed() {
        if !locked { shifted = false }
    }
}
