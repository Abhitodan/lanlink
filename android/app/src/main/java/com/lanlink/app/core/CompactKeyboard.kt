package com.lanlink.app.core

/**
 * LANLink's own compact keyboard for controlling a PC: phone-width, short
 * keys, no suggestion strip, with the keys a desktop needs (Esc, Tab, Ctrl,
 * Alt, Win, arrows) built in. The system keyboard cannot be resized by an
 * app, and in landscape it covers more than half the screen.
 *
 * Pure layout and state; the composable in ui.screens draws it.
 */
sealed interface KeyAction {
    /** Type this text (Unicode; a sticky Ctrl/Alt/Win turns it into a shortcut). */
    data class Text(val text: String) : KeyAction
    /** Press this Windows virtual key. */
    data class Key(val vk: Int) : KeyAction
    /** Toggle a sticky modifier (applies to the next key). */
    data class Sticky(val vk: Int) : KeyAction
    data object Shift : KeyAction
    data object Layer : KeyAction
    data object Hide : KeyAction
}

/** One key: what it shows, what it does, and its width relative to a letter. */
data class KeySpec(val label: String, val action: KeyAction, val weight: Float = 1f, val repeats: Boolean = false)

object CompactLayout {
    private fun letters(row: String, shifted: Boolean) = row.map {
        val c = if (shifted) it.uppercaseChar().toString() else it.toString()
        KeySpec(c, KeyAction.Text(c))
    }
    private fun chars(row: String) = row.map { KeySpec(it.toString(), KeyAction.Text(it.toString())) }

    private val backspace = KeySpec("⌫", KeyAction.Key(KeyMap.VK_BACK), 1.5f, repeats = true)
    private val enter = KeySpec("↵", KeyAction.Key(KeyMap.VK_RETURN), 1.5f)

    /** Rows for the letter layer or the symbols layer. */
    fun rows(symbols: Boolean, shifted: Boolean): List<List<KeySpec>> =
        if (!symbols) listOf(
            letters("qwertyuiop", shifted) + backspace,
            listOf(KeySpec("Tab", KeyAction.Key(KeyMap.VK_TAB), 1.2f)) + letters("asdfghjkl", shifted) + enter,
            listOf(KeySpec("⇧", KeyAction.Shift, 1.5f)) + letters("zxcvbnm", shifted) +
                chars(if (shifted) "!?" else ",.") + KeySpec("⇧", KeyAction.Shift, 1.2f),
            bottomRow(symbols = false),
        ) else listOf(
            chars("1234567890") + backspace,
            listOf(KeySpec("Esc", KeyAction.Key(KeyMap.VK_ESCAPE), 1.2f)) + chars("@#\$%&*-+()") + enter,
            listOf(KeySpec("Win", KeyAction.Sticky(KeyMap.VK_LWIN), 1.5f)) + chars("_=/\\:;'\"!?") +
                KeySpec("Del", KeyAction.Key(KeyMap.VK_DELETE), 1.2f, repeats = true),
            bottomRow(symbols = true),
        )

    private fun arrow(label: String, vk: Int) = KeySpec(label, KeyAction.Key(vk), repeats = true)

    /** Layer switch, Ctrl, Alt, space, arrows (all four on the symbols layer), hide. */
    private fun bottomRow(symbols: Boolean): List<KeySpec> {
        val arrows = if (symbols) listOf(
            arrow("←", KeyMap.VK_LEFT), arrow("↑", KeyMap.VK_UP), arrow("↓", KeyMap.VK_DOWN), arrow("→", KeyMap.VK_RIGHT),
        ) else listOf(arrow("←", KeyMap.VK_LEFT), arrow("→", KeyMap.VK_RIGHT))
        return listOf(
            KeySpec(if (symbols) "abc" else "?123", KeyAction.Layer, 1.5f),
            KeySpec("Ctrl", KeyAction.Sticky(KeyMap.VK_LCONTROL), 1.3f),
            KeySpec("Alt", KeyAction.Sticky(KeyMap.VK_LMENU), 1.2f),
            KeySpec("", KeyAction.Text(" "), if (symbols) 3f else 5f),
        ) + arrows + KeySpec("⌄", KeyAction.Hide)
    }
}

/**
 * Shift: one tap = the next letter only; a second tap within
 * [DOUBLE_TAP_MS] = caps lock until tapped again.
 */
class ShiftState {
    companion object { const val DOUBLE_TAP_MS = 350L }

    var shifted = false
        private set
    var locked = false
        private set
    private var lastTap = Long.MIN_VALUE / 2

    fun tap(t: Long) {
        when {
            locked -> { locked = false; shifted = false }
            shifted && t - lastTap <= DOUBLE_TAP_MS -> locked = true
            shifted -> shifted = false
            else -> shifted = true
        }
        lastTap = t
    }

    /** A character was typed: a one-shot shift ends, caps lock stays. */
    fun typed() {
        if (!locked) shifted = false
    }
}
