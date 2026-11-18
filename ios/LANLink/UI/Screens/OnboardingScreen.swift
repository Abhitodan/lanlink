import SwiftUI

// Twin of OnboardingScreen in android/.../ui/screens/HomeScreens.kt.

struct OnboardingScreen: View {
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Spacer(minLength: 0)
            GlyphIcon(.laptop, size: 56)
                .foregroundColor(Theme.primary)
            Text("Your PC, in your hand.")
                .textStyle(Theme.Typography.headlineMedium)
            Text("LANLink shows your Windows screen on this phone and lets you use it with touch and the keyboard, over your own Wi-Fi.")
                .textStyle(Theme.Typography.bodyLarge)
                .fixedSize(horizontal: false, vertical: true)
            Card {
                HStack(spacing: 0) {
                    GlyphIcon(.shield, size: 22)
                        .foregroundColor(Theme.primary)
                    Spacer().frame(width: 12)
                    Text("Private by design").textStyle(Theme.Typography.titleMedium)
                }
                Note(
                    "Everything stays on your local network and is encrypted. Your PC must approve this phone once, "
                        + "and can remove it at any time. No account, no cloud."
                )
            }
            Spacer(minLength: 0)
            PrimaryButton("Scan the PC's pairing code", icon: .qrScan) {
                model.finishOnboarding(next: .pair())
            }
            SecondaryButton("Find my PC on this network", icon: .laptop) {
                model.finishOnboarding(next: .home)
            }
            Note("On the PC, run LANLink and choose \"Pair a phone\" to show the code.")
        }
        .padding(28)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
        .foregroundColor(Theme.onBackground)
        .background(Theme.background.ignoresSafeArea())
        .toolbar(.hidden, for: .navigationBar)
    }
}
