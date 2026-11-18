import SwiftUI

@main
struct LANLinkApp: App {
    var body: some Scene {
        WindowGroup {
            RootView()
        }
    }
}

/// Placeholder first page until the screens land (later tasks replace it).
struct RootView: View {
    var body: some View {
        Page(title: "LANLink", onBack: nil) {
            SectionLabel("Your PCs")
            Card {
                HStack(spacing: 12) {
                    GlyphIcon(.laptop)
                    Text("LANLink").textStyle(Theme.Typography.titleMedium)
                    Spacer()
                    StatusDot(Theme.ok)
                }
            }
            Note("LANLink connects to your PC over your local network.")
        }
    }
}
