package com.lanlink.app.core

/**
 * Android `KeyEvent` codes → Windows virtual-key codes, for physical
 * (Bluetooth/USB) keyboards and the shortcut toolbar.
 *
 * Physical keys are sent as *key presses*, so the PC's keyboard layout
 * decides which character they produce (like a keyboard plugged into the
 * PC). Text typed on the on-screen keyboard is sent as Unicode text instead,
 * so it arrives exactly as typed regardless of the PC's layout.
 */
object KeyMap {
    const val VK_BACK = 0x08
    const val VK_TAB = 0x09
    const val VK_RETURN = 0x0D
    const val VK_ESCAPE = 0x1B
    const val VK_SPACE = 0x20
    const val VK_PRIOR = 0x21
    const val VK_NEXT = 0x22
    const val VK_END = 0x23
    const val VK_HOME = 0x24
    const val VK_LEFT = 0x25
    const val VK_UP = 0x26
    const val VK_RIGHT = 0x27
    const val VK_DOWN = 0x28
    const val VK_SNAPSHOT = 0x2C
    const val VK_INSERT = 0x2D
    const val VK_DELETE = 0x2E
    const val VK_LWIN = 0x5B
    const val VK_APPS = 0x5D
    const val VK_F1 = 0x70
    const val VK_LSHIFT = 0xA0
    const val VK_RSHIFT = 0xA1
    const val VK_LCONTROL = 0xA2
    const val VK_RCONTROL = 0xA3
    const val VK_LMENU = 0xA4
    const val VK_RMENU = 0xA5

    // android.view.KeyEvent constants, duplicated so this file is testable
    // on the JVM without Android.
    private val table: Map<Int, Int> = buildMap {
        for (i in 0..25) put(29 + i, 0x41 + i) // KEYCODE_A..Z
        for (i in 0..9) put(7 + i, 0x30 + i) // KEYCODE_0..9
        for (i in 0..11) put(131 + i, VK_F1 + i) // KEYCODE_F1..F12
        for (i in 0..9) put(144 + i, 0x60 + i) // KEYCODE_NUMPAD_0..9
        put(66, VK_RETURN) // ENTER
        put(160, VK_RETURN) // NUMPAD_ENTER
        put(67, VK_BACK) // DEL (backspace)
        put(112, VK_DELETE) // FORWARD_DEL
        put(61, VK_TAB)
        put(62, VK_SPACE)
        put(111, VK_ESCAPE)
        put(19, VK_UP); put(20, VK_DOWN); put(21, VK_LEFT); put(22, VK_RIGHT)
        put(122, VK_HOME); put(123, VK_END)
        put(92, VK_PRIOR); put(93, VK_NEXT)
        put(124, VK_INSERT)
        put(120, VK_SNAPSHOT) // SYSRQ
        put(121, 0x13) // BREAK → Pause
        put(115, 0x14) // CAPS_LOCK
        put(116, 0x91) // SCROLL_LOCK
        put(143, 0x90) // NUM_LOCK
        put(59, VK_LSHIFT); put(60, VK_RSHIFT)
        put(113, VK_LCONTROL); put(114, VK_RCONTROL)
        put(57, VK_LMENU); put(58, VK_RMENU)
        put(117, VK_LWIN); put(118, 0x5C) // META_LEFT / META_RIGHT
        put(82, VK_APPS) // MENU
        put(55, 0xBC) // COMMA
        put(56, 0xBE) // PERIOD
        put(69, 0xBD) // MINUS
        put(70, 0xBB) // EQUALS
        put(71, 0xDB) // LEFT_BRACKET
        put(72, 0xDD) // RIGHT_BRACKET
        put(73, 0xDC) // BACKSLASH
        put(74, 0xBA) // SEMICOLON
        put(75, 0xDE) // APOSTROPHE
        put(76, 0xBF) // SLASH
        put(68, 0xC0) // GRAVE
        put(154, 0x6F) // NUMPAD_DIVIDE
        put(155, 0x6A) // NUMPAD_MULTIPLY
        put(156, 0x6D) // NUMPAD_SUBTRACT
        put(157, 0x6B) // NUMPAD_ADD
        put(158, 0x6E) // NUMPAD_DOT
    }

    fun vkFor(androidKeyCode: Int): Int? = table[androidKeyCode]

    fun isModifier(vk: Int): Boolean = vk in VK_LSHIFT..VK_RMENU || vk == VK_LWIN || vk == 0x5C

    /**
     * Virtual key that types [c] on a US layout, for use together with a
     * held modifier (Ctrl+C must be sent as the C *key*, not the character).
     * Returns null for characters that need a different strategy.
     */
    fun vkForShortcutChar(c: Char): Int? = when (c) {
        in 'a'..'z' -> 0x41 + (c - 'a')
        in 'A'..'Z' -> 0x41 + (c - 'A')
        in '0'..'9' -> 0x30 + (c - '0')
        ' ' -> VK_SPACE
        '\n' -> VK_RETURN
        '\t' -> VK_TAB
        ',' -> 0xBC
        '.' -> 0xBE
        '-' -> 0xBD
        '=' -> 0xBB
        '/' -> 0xBF
        ';' -> 0xBA
        '\'' -> 0xDE
        '[' -> 0xDB
        ']' -> 0xDD
        '\\' -> 0xDC
        '`' -> 0xC0
        else -> null
    }
}
