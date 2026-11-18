import SwiftUI

// Port of android/.../ui/screens/Components.kt (plus `Toggle` from
// SettingsScreens.kt as `ToggleRow`). Same spacing, radii and type scale.

/// Standard page: back arrow, title, scrollable content with safe padding.
struct Page<Actions: View, Content: View>: View {
    let title: String
    let onBack: (() -> Void)?
    let actions: Actions
    let content: Content

    init(
        title: String,
        onBack: (() -> Void)?,
        @ViewBuilder actions: () -> Actions,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.onBack = onBack
        self.actions = actions()
        self.content = content()
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 0) {
                if let onBack {
                    Button(action: onBack) {
                        Image(systemName: "arrow.left")
                            .font(.system(size: 20, weight: .regular))
                            .frame(width: 48, height: 48)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Back")
                } else {
                    Spacer().frame(width: 16)
                }
                Text(title)
                    .textStyle(Theme.Typography.titleLarge)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityAddTraits(.isHeader)
                actions
            }
            .frame(minHeight: 56)
            .padding(.horizontal, 4)

            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    content
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 8)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .foregroundColor(Theme.onBackground)
        .background(Theme.background.ignoresSafeArea())
        .toolbar(.hidden, for: .navigationBar)
    }
}

extension Page where Actions == EmptyView {
    init(title: String, onBack: (() -> Void)?, @ViewBuilder content: () -> Content) {
        self.init(title: title, onBack: onBack, actions: { EmptyView() }, content: content)
    }
}

struct Card<Content: View>: View {
    let onClick: (() -> Void)?
    let content: Content

    init(onClick: (() -> Void)? = nil, @ViewBuilder content: () -> Content) {
        self.onClick = onClick
        self.content = content()
    }

    var body: some View {
        if let onClick {
            Button(action: onClick) { surface }
                .buttonStyle(CardButtonStyle())
        } else {
            surface
        }
    }

    private var surface: some View {
        let shape = RoundedRectangle(cornerRadius: Theme.Radius.large, style: .continuous)
        return VStack(alignment: .leading, spacing: 6) {
            content
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        // Material's Surface(surfaceVariant) gives its content onSurfaceVariant.
        .foregroundColor(Theme.onSurfaceVariant)
        .background(Theme.surfaceVariant)
        .clipShape(shape)
        .overlay(shape.strokeBorder(Theme.outline, lineWidth: 1))
        .contentShape(shape)
    }
}

/// Pressed feedback for a clickable card (stands in for the Material ripple).
private struct CardButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .opacity(configuration.isPressed ? 0.85 : 1)
    }
}

struct SectionLabel: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(text.uppercased())
            .textStyle(Theme.Typography.labelMedium)
            .foregroundColor(Theme.onSurfaceVariant)
            .padding(.top, 8)
            .accessibilityAddTraits(.isHeader)
    }
}

struct PrimaryButton: View {
    let text: String
    var icon: Glyph? = nil
    var enabled: Bool = true
    let action: () -> Void

    init(_ text: String, icon: Glyph? = nil, enabled: Bool = true, action: @escaping () -> Void) {
        self.text = text
        self.icon = icon
        self.enabled = enabled
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            HStack(spacing: 0) {
                if let icon {
                    GlyphIcon(icon, size: 20)
                    Spacer().frame(width: 10)
                }
                Text(text).textStyle(Theme.Typography.labelLarge)
            }
        }
        .buttonStyle(FilledButtonStyle())
        .disabled(!enabled)
    }
}

/// Material 3 filled button: primary container, onPrimary content; disabled
/// uses onSurface at 12% / 38%.
private struct FilledButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        let shape = RoundedRectangle(cornerRadius: Theme.Radius.medium, style: .continuous)
        return configuration.label
            .foregroundColor(isEnabled ? Theme.onPrimary : Theme.onSurface.opacity(0.38))
            .padding(.horizontal, 20)
            .frame(maxWidth: .infinity, minHeight: 52)
            .background(isEnabled ? Theme.primary : Theme.onSurface.opacity(0.12))
            .clipShape(shape)
            .contentShape(shape)
            .opacity(configuration.isPressed ? 0.88 : 1)
    }
}

struct SecondaryButton: View {
    let text: String
    var icon: Glyph? = nil
    var danger: Bool = false
    let action: () -> Void

    init(_ text: String, icon: Glyph? = nil, danger: Bool = false, action: @escaping () -> Void) {
        self.text = text
        self.icon = icon
        self.danger = danger
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            HStack(spacing: 0) {
                if let icon {
                    GlyphIcon(icon, size: 20)
                    Spacer().frame(width: 10)
                }
                Text(text).textStyle(Theme.Typography.labelLarge)
            }
        }
        .buttonStyle(OutlinedButtonStyle(danger: danger))
    }
}

/// Material 3 outlined button: 1 pt outline (error when dangerous), no fill.
private struct OutlinedButtonStyle: ButtonStyle {
    let danger: Bool
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        let shape = RoundedRectangle(cornerRadius: Theme.Radius.medium, style: .continuous)
        let content = danger ? Theme.error : Theme.onSurface
        return configuration.label
            .foregroundColor(isEnabled ? content : Theme.onSurface.opacity(0.38))
            .padding(.horizontal, 24)
            .frame(maxWidth: .infinity, minHeight: 52)
            .background(configuration.isPressed ? content.opacity(0.10) : Color.clear)
            .clipShape(shape)
            .overlay(shape.strokeBorder(danger ? Theme.error : Theme.outline, lineWidth: 1))
            .contentShape(shape)
    }
}

struct StatusDot: View {
    let color: Color

    init(_ color: Color) {
        self.color = color
    }

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 8, height: 8)
    }
}

/// A label/value row; "Not measured" when `value` is nil.
struct MetricRow: View {
    let label: String
    let value: String?
    var hint: String? = nil

    init(_ label: String, value: String?, hint: String? = nil) {
        self.label = label
        self.value = value
        self.hint = hint
    }

    var body: some View {
        HStack(alignment: .center, spacing: 0) {
            VStack(alignment: .leading, spacing: 0) {
                Text(label).textStyle(Theme.Typography.bodyMedium)
                if let hint {
                    Text(hint)
                        .textStyle(Theme.Typography.labelMedium)
                        .foregroundColor(Theme.onSurfaceVariant)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Text(value ?? "Not measured")
                .textStyle(Theme.Typography.bodyMedium)
                .foregroundColor(value == nil ? Theme.onSurfaceVariant : Theme.onSurface)
        }
        .frame(minHeight: 36)
    }
}

struct Note: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(text)
            .textStyle(Theme.Typography.bodyMedium)
            .foregroundColor(Theme.onSurfaceVariant)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// A labelled switch row (Android `Toggle` in SettingsScreens.kt); the switch
/// is the native iOS one, tinted with the primary colour.
struct ToggleRow: View {
    let label: String
    @Binding var isOn: Bool

    init(_ label: String, isOn: Binding<Bool>) {
        self.label = label
        _isOn = isOn
    }

    var body: some View {
        Toggle(isOn: $isOn) {
            Text(label).textStyle(Theme.Typography.bodyLarge)
        }
        .tint(Theme.primary)
        .frame(minHeight: 48)
    }
}

/// Fixed vertical gap (`VSpace` in Components.kt).
struct VSpace: View {
    let height: CGFloat

    init(_ height: CGFloat) {
        self.height = height
    }

    var body: some View {
        Spacer().frame(height: height)
    }
}
