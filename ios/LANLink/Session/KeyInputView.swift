import UIKit

/// Invisible view that owns the system keyboard (twin of
/// `KeyboardInputView` in RemoteView.kt).
///
/// Whatever is typed here goes to the PC, often passwords. Android asks the
/// keyboard not to learn it (`IME_FLAG_NO_PERSONALIZED_LEARNING`, fix M4);
/// iOS has no such flag, so autocorrection, spell checking, smart
/// punctuation, auto-capitalisation and predictive text are all turned off.
/// Without suggestions there is nothing to learn from and no correction is
/// ever replayed on the PC.
///
/// `UIKeyInput` has no composing text: the keyboard commits each character
/// (`insertText`) and each Backspace (`deleteBackward`) one at a time, so a
/// delete never carries a count to clamp (fix L2's 256 cap holds by
/// construction).
final class KeyInputView: UIView, UIKeyInput {
    private let input: RemoteInput
    /// Physical keyboard keys while this view holds the keyboard; returns
    /// whether the key was sent to the PC.
    var onHardwareKey: ((UIPress, Bool) -> Bool)?

    // UITextInputTraits: no learning, no rewriting (see above).
    var autocorrectionType: UITextAutocorrectionType = .no
    var spellCheckingType: UITextSpellCheckingType = .no
    var smartQuotesType: UITextSmartQuotesType = .no
    var smartDashesType: UITextSmartDashesType = .no
    var smartInsertDeleteType: UITextSmartInsertDeleteType = .no
    var autocapitalizationType: UITextAutocapitalizationType = .none
    var keyboardType: UIKeyboardType = .default
    var returnKeyType: UIReturnKeyType = .default
    var enablesReturnKeyAutomatically = false

    @available(iOS 17.0, *)
    var inlinePredictionType: UITextInlinePredictionType {
        get { .no }
        set {}
    }

    init(input: RemoteInput) {
        self.input = input
        super.init(frame: CGRect(x: 0, y: 0, width: 1, height: 1))
        backgroundColor = .clear
        isAccessibilityElement = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override var canBecomeFirstResponder: Bool { true }

    /// Always true, so the keyboard's Backspace is delivered even before
    /// anything was typed here (the text being edited lives on the PC).
    var hasText: Bool { true }

    func insertText(_ text: String) {
        // Return is a key press on the PC; everything else is Unicode text.
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
        for (i, line) in lines.enumerated() {
            if i > 0 { input.tapKey(KeyMap.VK_RETURN) }
            if !line.isEmpty { input.text(String(line)) }
        }
    }

    func deleteBackward() {
        input.tapKey(KeyMap.VK_BACK)
    }

    // Physical keys reach the first responder before text input: send them
    // to the PC as key presses (the PC's layout decides the character).
    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let rest = presses.filter { !(onHardwareKey?($0, true) ?? false) }
        if !rest.isEmpty { super.pressesBegan(rest, with: event) }
    }

    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let rest = presses.filter { !(onHardwareKey?($0, false) ?? false) }
        if !rest.isEmpty { super.pressesEnded(rest, with: event) }
    }

    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let rest = presses.filter { !(onHardwareKey?($0, false) ?? false) }
        if !rest.isEmpty { super.pressesCancelled(rest, with: event) }
    }
}
