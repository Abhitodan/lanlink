import SwiftUI
import UIKit

// Calm, content-first palette: ivory surfaces, graphite text, a muted blue
// for actions and a warm sandstone accent used sparingly.
// Port of android/.../ui/theme/Theme.kt; the hex values are identical.
enum Palette {
    static let ivory = UIColor(hex: 0xFAF8F3)
    static let ivoryDeep = UIColor(hex: 0xF1EDE4)
    static let line = UIColor(hex: 0xE2DDD2)
    static let graphite = UIColor(hex: 0x1F2328)
    static let graphiteSoft = UIColor(hex: 0x5B616B)
    static let blue = UIColor(hex: 0x2F4A6D)
    static let blueSoft = UIColor(hex: 0xDCE4EE)
    static let sandstone = UIColor(hex: 0xD9B98C)
    static let sandstoneSoft = UIColor(hex: 0xF3E7D5)
    static let danger = UIColor(hex: 0xA23B33)
    static let ok = UIColor(hex: 0x3E7A55)

    static let night = UIColor(hex: 0x15181C)
    static let nightRaised = UIColor(hex: 0x1E2227)
    static let nightLine = UIColor(hex: 0x2D3239)
    static let mist = UIColor(hex: 0xE9E6DF)
    static let mistSoft = UIColor(hex: 0xA9AEB6)
    static let blueNight = UIColor(hex: 0x9DB6D6)
}

/// One colour scheme (the Material 3 roles `Theme.kt` sets).
struct ColorTokens {
    let primary: UIColor
    let onPrimary: UIColor
    let primaryContainer: UIColor
    let onPrimaryContainer: UIColor
    let secondary: UIColor
    let onSecondary: UIColor
    let secondaryContainer: UIColor
    let onSecondaryContainer: UIColor
    let background: UIColor
    let onBackground: UIColor
    let surface: UIColor
    let onSurface: UIColor
    let surfaceVariant: UIColor
    let onSurfaceVariant: UIColor
    let outline: UIColor
    let outlineVariant: UIColor
    let error: UIColor
}

/// Colour roles that follow the system light/dark setting (like
/// `LanLinkTheme(dark = isSystemInDarkTheme())`), plus type and shapes.
enum Theme {
    static let light = ColorTokens(
        primary: Palette.blue,
        onPrimary: Palette.ivory,
        primaryContainer: Palette.blueSoft,
        onPrimaryContainer: Palette.blue,
        secondary: Palette.sandstone,
        onSecondary: Palette.graphite,
        secondaryContainer: Palette.sandstoneSoft,
        onSecondaryContainer: Palette.graphite,
        background: Palette.ivory,
        onBackground: Palette.graphite,
        surface: Palette.ivory,
        onSurface: Palette.graphite,
        surfaceVariant: Palette.ivoryDeep,
        onSurfaceVariant: Palette.graphiteSoft,
        outline: Palette.line,
        outlineVariant: Palette.line,
        error: Palette.danger
    )

    static let dark = ColorTokens(
        primary: Palette.blueNight,
        onPrimary: Palette.night,
        primaryContainer: UIColor(hex: 0x2A3B52),
        onPrimaryContainer: Palette.mist,
        secondary: Palette.sandstone,
        onSecondary: Palette.night,
        secondaryContainer: UIColor(hex: 0x3A3226),
        onSecondaryContainer: Palette.mist,
        background: Palette.night,
        onBackground: Palette.mist,
        surface: Palette.night,
        onSurface: Palette.mist,
        surfaceVariant: Palette.nightRaised,
        onSurfaceVariant: Palette.mistSoft,
        outline: Palette.nightLine,
        outlineVariant: Palette.nightLine,
        error: UIColor(hex: 0xE08A80)
    )

    static let primary = adaptive(\.primary)
    static let onPrimary = adaptive(\.onPrimary)
    static let primaryContainer = adaptive(\.primaryContainer)
    static let onPrimaryContainer = adaptive(\.onPrimaryContainer)
    static let secondary = adaptive(\.secondary)
    static let onSecondary = adaptive(\.onSecondary)
    static let secondaryContainer = adaptive(\.secondaryContainer)
    static let onSecondaryContainer = adaptive(\.onSecondaryContainer)
    static let background = adaptive(\.background)
    static let onBackground = adaptive(\.onBackground)
    static let surface = adaptive(\.surface)
    static let onSurface = adaptive(\.onSurface)
    static let surfaceVariant = adaptive(\.surfaceVariant)
    static let onSurfaceVariant = adaptive(\.onSurfaceVariant)
    static let outline = adaptive(\.outline)
    static let outlineVariant = adaptive(\.outlineVariant)
    static let error = adaptive(\.error)

    /// Brand status colours used directly in both schemes (as on Android).
    static let ok = Color(Palette.ok)
    static let danger = Color(Palette.danger)
    static let sandstone = Color(Palette.sandstone)

    /// The scheme for a given appearance, for UIKit code.
    static func tokens(for scheme: ColorScheme) -> ColorTokens {
        scheme == .dark ? dark : light
    }

    private static func adaptive(_ role: KeyPath<ColorTokens, UIColor>) -> Color {
        let lightColor = light[keyPath: role]
        let darkColor = dark[keyPath: role]
        return Color(UIColor { traits in
            traits.userInterfaceStyle == .dark ? darkColor : lightColor
        })
    }

    // MARK: - Shapes (Theme.kt AppShapes)

    enum Radius {
        static let small: CGFloat = 8
        static let medium: CGFloat = 12
        static let large: CGFloat = 16
    }

    // MARK: - Typography (Theme.kt Type, system font)

    struct TextStyle {
        let size: CGFloat
        let weight: Font.Weight
        let lineHeight: CGFloat
        let tracking: CGFloat
        let relativeTo: Font.TextStyle
    }

    enum Typography {
        static let headlineMedium = TextStyle(size: 26, weight: .semibold, lineHeight: 32, tracking: -0.2, relativeTo: .title)
        static let titleLarge = TextStyle(size: 20, weight: .semibold, lineHeight: 26, tracking: 0, relativeTo: .title3)
        static let titleMedium = TextStyle(size: 16, weight: .medium, lineHeight: 22, tracking: 0, relativeTo: .headline)
        static let bodyLarge = TextStyle(size: 16, weight: .regular, lineHeight: 24, tracking: 0, relativeTo: .body)
        static let bodyMedium = TextStyle(size: 14, weight: .regular, lineHeight: 20, tracking: 0, relativeTo: .subheadline)
        static let labelLarge = TextStyle(size: 14, weight: .medium, lineHeight: 20, tracking: 0.1, relativeTo: .subheadline)
        static let labelMedium = TextStyle(size: 12, weight: .medium, lineHeight: 16, tracking: 0.3, relativeTo: .caption)
    }
}

/// Applies a `Theme.TextStyle`; sizes scale with Dynamic Type like `sp`.
struct TextStyleModifier: ViewModifier {
    let style: Theme.TextStyle
    @ScaledMetric private var size: CGFloat
    @ScaledMetric private var lineHeight: CGFloat

    init(_ style: Theme.TextStyle) {
        self.style = style
        _size = ScaledMetric(wrappedValue: style.size, relativeTo: style.relativeTo)
        _lineHeight = ScaledMetric(wrappedValue: style.lineHeight, relativeTo: style.relativeTo)
    }

    func body(content: Content) -> some View {
        let font = UIFont.systemFont(ofSize: size)
        return content
            .font(.system(size: size, weight: style.weight))
            .tracking(style.tracking)
            .lineSpacing(max(0, lineHeight - font.lineHeight))
    }
}

extension View {
    func textStyle(_ style: Theme.TextStyle) -> some View {
        modifier(TextStyleModifier(style))
    }
}

extension UIColor {
    /// `0xRRGGBB`, fully opaque.
    convenience init(hex: UInt32) {
        self.init(
            red: CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255,
            alpha: 1
        )
    }
}

extension Color {
    /// `0xRRGGBB`, fully opaque.
    init(hex: UInt32) {
        self.init(UIColor(hex: hex))
    }
}
