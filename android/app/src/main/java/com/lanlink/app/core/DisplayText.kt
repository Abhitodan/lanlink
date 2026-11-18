package com.lanlink.app.core

/**
 * A name another device chose (mDNS, pairing link) made safe to show:
 * control and invisible formatting characters (bidi overrides and
 * isolates, zero-width marks) are removed, whitespace collapses, and the
 * length is capped. Mirrors `lanlink_protocol::wire::display_name`.
 */
object DisplayText {
    private fun hidden(c: Char): Boolean =
        Character.isISOControl(c) || c == '\u00AD' || c == '\u061C' || c == '\u180E' ||
            c in '\u200B'..'\u200F' || c in '\u2028'..'\u202E' || c in '\u2060'..'\u2064' ||
            c in '\u2066'..'\u206F' || c == '\uFEFF' || c in '\uFFF9'..'\uFFFB'

    fun name(s: String, maxChars: Int = 48): String {
        val out = StringBuilder()
        var space = false
        var i = 0
        while (i < s.length) {
            val cp = s.codePointAt(i)
            val n = Character.charCount(cp)
            val c = s[i]
            i += n
            if (Character.isWhitespace(cp)) { space = out.isNotEmpty(); continue }
            if (n == 1 && hidden(c)) continue
            if (n == 1 && (c in '\u0600'..'\u0605' || c == '\u06DD' || c == '\u070F' || c == '\u08E2')) continue
            // Tag characters and other invisible supplementary formats.
            if (cp in 0xE0000..0xE007F || cp == 0x110BD || cp in 0x1D173..0x1D17A) continue
            if (out.codePointCount(0, out.length) >= maxChars) break
            if (space) { out.append(' '); space = false }
            out.appendCodePoint(cp)
        }
        return out.toString()
    }
}
