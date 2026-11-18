import Foundation

/// iOS `UIKeyboardHIDUsage` values (USB HID usage page 0x07) -> Windows
/// virtual-key codes, for physical (Bluetooth/USB) keyboards and the
/// shortcut toolbar. The Android client maps `KeyEvent` codes; behaviour is
/// the same, only the input codes differ.
///
/// Physical keys are sent as *key presses*, so the PC's keyboard layout
/// decides which character they produce (like a keyboard plugged into the
/// PC). Text typed on the on-screen keyboard is sent as Unicode text instead,
/// so it arrives exactly as typed regardless of the PC's layout.
enum KeyMap {
    static let VK_BACK = 0x08
    static let VK_TAB = 0x09
    static let VK_RETURN = 0x0D
    static let VK_ESCAPE = 0x1B
    static let VK_SPACE = 0x20
    static let VK_PRIOR = 0x21
    static let VK_NEXT = 0x22
    static let VK_END = 0x23
    static let VK_HOME = 0x24
    static let VK_LEFT = 0x25
    static let VK_UP = 0x26
    static let VK_RIGHT = 0x27
    static let VK_DOWN = 0x28
    static let VK_SNAPSHOT = 0x2C
    static let VK_INSERT = 0x2D
    static let VK_DELETE = 0x2E
    static let VK_LWIN = 0x5B
    static let VK_APPS = 0x5D
    static let VK_F1 = 0x70
    static let VK_LSHIFT = 0xA0
    static let VK_RSHIFT = 0xA1
    static let VK_LCONTROL = 0xA2
    static let VK_RCONTROL = 0xA3
    static let VK_LMENU = 0xA4
    static let VK_RMENU = 0xA5

    // UIKeyboardHIDUsage raw values, duplicated so this file is testable
    // without UIKit.
    private static let table: [Int: Int] = {
        var m: [Int: Int] = [:]
        for i in 0...25 { m[0x04 + i] = 0x41 + i } // keyboardA..Z
        for i in 0...8 { m[0x1E + i] = 0x31 + i } // keyboard1..9
        m[0x27] = 0x30 // keyboard0
        for i in 0...11 { m[0x3A + i] = KeyMap.VK_F1 + i } // keyboardF1..F12
        for i in 0...8 { m[0x59 + i] = 0x61 + i } // keypad1..9
        m[0x62] = 0x60 // keypad0
        m[0x28] = KeyMap.VK_RETURN // keyboardReturnOrEnter
        m[0x58] = KeyMap.VK_RETURN // keypadEnter
        m[0x2A] = KeyMap.VK_BACK // keyboardDeleteOrBackspace
        m[0x4C] = KeyMap.VK_DELETE // keyboardDeleteForward
        m[0x2B] = KeyMap.VK_TAB
        m[0x2C] = KeyMap.VK_SPACE
        m[0x29] = KeyMap.VK_ESCAPE
        m[0x52] = KeyMap.VK_UP; m[0x51] = KeyMap.VK_DOWN; m[0x50] = KeyMap.VK_LEFT; m[0x4F] = KeyMap.VK_RIGHT
        m[0x4A] = KeyMap.VK_HOME; m[0x4D] = KeyMap.VK_END
        m[0x4B] = KeyMap.VK_PRIOR; m[0x4E] = KeyMap.VK_NEXT
        m[0x49] = KeyMap.VK_INSERT
        m[0x46] = KeyMap.VK_SNAPSHOT // keyboardPrintScreen
        m[0x48] = 0x13 // keyboardPause -> Pause
        m[0x39] = 0x14 // keyboardCapsLock
        m[0x47] = 0x91 // keyboardScrollLock
        m[0x53] = 0x90 // keypadNumLock
        m[0xE1] = KeyMap.VK_LSHIFT; m[0xE5] = KeyMap.VK_RSHIFT
        m[0xE0] = KeyMap.VK_LCONTROL; m[0xE4] = KeyMap.VK_RCONTROL
        m[0xE2] = KeyMap.VK_LMENU; m[0xE6] = KeyMap.VK_RMENU
        m[0xE3] = KeyMap.VK_LWIN; m[0xE7] = 0x5C // keyboardLeftGUI / keyboardRightGUI
        m[0x65] = KeyMap.VK_APPS // keyboardApplication
        m[0x36] = 0xBC // comma
        m[0x37] = 0xBE // period
        m[0x2D] = 0xBD // hyphen
        m[0x2E] = 0xBB // equalSign
        m[0x2F] = 0xDB // openBracket
        m[0x30] = 0xDD // closeBracket
        m[0x31] = 0xDC // backslash
        m[0x33] = 0xBA // semicolon
        m[0x34] = 0xDE // quote
        m[0x38] = 0xBF // slash
        m[0x35] = 0xC0 // graveAccentAndTilde
        m[0x54] = 0x6F // keypadSlash
        m[0x55] = 0x6A // keypadAsterisk
        m[0x56] = 0x6D // keypadHyphen
        m[0x57] = 0x6B // keypadPlus
        m[0x63] = 0x6E // keypadPeriod
        return m
    }()

    static func vkFor(keyCode: Int) -> Int? { table[keyCode] }

    static func isModifier(vk: Int) -> Bool {
        (VK_LSHIFT...VK_RMENU).contains(vk) || vk == VK_LWIN || vk == 0x5C
    }

    /// Virtual key that types [c] on a US layout, for use together with a
    /// held modifier (Ctrl+C must be sent as the C *key*, not the character).
    /// Returns nil for characters that need a different strategy.
    static func vkForShortcutChar(c: Character) -> Int? {
        let scalars = c.unicodeScalars
        guard scalars.count == 1, let u = scalars.first else { return nil }
        switch u.value {
        case 0x61...0x7A: return 0x41 + Int(u.value - 0x61) // a-z
        case 0x41...0x5A: return 0x41 + Int(u.value - 0x41) // A-Z
        case 0x30...0x39: return 0x30 + Int(u.value - 0x30) // 0-9
        case 0x20: return VK_SPACE
        case 0x0A: return VK_RETURN
        case 0x09: return VK_TAB
        case 0x2C: return 0xBC // ,
        case 0x2E: return 0xBE // .
        case 0x2D: return 0xBD // -
        case 0x3D: return 0xBB // =
        case 0x2F: return 0xBF // /
        case 0x3B: return 0xBA // ;
        case 0x27: return 0xDE // '
        case 0x5B: return 0xDB // [
        case 0x5D: return 0xDD // ]
        case 0x5C: return 0xDC // \
        case 0x60: return 0xC0 // `
        default: return nil
        }
    }
}
