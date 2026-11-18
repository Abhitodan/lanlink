import Foundation

/// A name another device chose (mDNS, pairing link) made safe to show:
/// control and invisible formatting characters (bidi overrides and
/// isolates, zero-width marks) are removed, whitespace collapses, and the
/// length is capped. Mirrors `lanlink_protocol::wire::display_name`.
enum DisplayText {
    private static func hidden(_ s: Unicode.Scalar) -> Bool {
        if s.properties.generalCategory == .control { return true }
        switch s.value {
        case 0x00AD,                 // soft hyphen
             0x0600...0x0605,        // Arabic number signs (format)
             0x06DD, 0x070F, 0x08E2,
             0x110BD,
             0x1D173...0x1D17A,
             0xE0000...0xE007F,      // tag characters (invisible)
             0x061C, 0x180E,
             0x200B...0x200F,        // zero-width, LRM/RLM
             0x2028...0x202E,        // separators, bidi embeddings/overrides
             0x2060...0x2064,
             0x2066...0x206F,        // bidi isolates, deprecated formats
             0xFEFF,
             0xFFF9...0xFFFB:
            return true
        default:
            return false
        }
    }

    static func name(s: String, maxChars: Int = 48) -> String {
        var out = String.UnicodeScalarView()
        var count = 0
        var space = false
        for u in s.unicodeScalars {
            // Line breaks and tabs are whitespace before they are control
            // characters: they separate words, so they become a space.
            if u.properties.isWhitespace { space = count > 0; continue }
            if hidden(u) { continue }
            if count >= maxChars { break }
            if space { out.append(" "); count += 1; space = false }
            out.append(u)
            count += 1
        }
        return String(out)
    }
}
