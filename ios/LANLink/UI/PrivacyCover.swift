import UIKit

// The iPhone stand-in for Android's FLAG_SECURE on the remote screen (fix
// M5). iOS cannot block screenshots, so instead:
// - the PC screen is covered as soon as the app starts leaving the
//   foreground (`sceneWillResignActive`), before iOS takes the app-switcher
//   snapshot;
// - while the screen is recorded or mirrored (`UIScreen.isCaptured`) the
//   picture is hidden behind a notice.
// Both are skipped when the user allows the PC screen in recordings and the
// app switcher (`Settings.allowScreenshots`, off by default).

/// Full-screen Night cover with the LANLink logo.
final class PrivacyCoverView: UIView {
    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = Palette.night
        autoresizingMask = [.flexibleWidth, .flexibleHeight]
        accessibilityIdentifier = "privacy-cover"

        // docs/media/logo.svg: a sandstone disc with two linked night rings.
        let logo = CAShapeLayer()
        logo.bounds = CGRect(x: 0, y: 0, width: 100, height: 100)
        logo.path = UIBezierPath(ovalIn: CGRect(x: 2, y: 2, width: 96, height: 96)).cgPath
        logo.fillColor = Palette.sandstone.cgColor
        for cx in [41.0, 59.0] {
            let ring = CAShapeLayer()
            ring.path = UIBezierPath(
                arcCenter: CGPoint(x: cx, y: 50), radius: 15, startAngle: 0, endAngle: .pi * 2, clockwise: true
            ).cgPath
            ring.fillColor = nil
            ring.strokeColor = Palette.night.cgColor
            ring.lineWidth = 5.5
            logo.addSublayer(ring)
        }
        layer.addSublayer(logo)
        self.logo = logo
    }

    private var logo: CALayer?

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override func layoutSubviews() {
        super.layoutSubviews()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        logo?.position = CGPoint(x: bounds.midX, y: bounds.midY)
        logo?.setAffineTransform(CGAffineTransform(scaleX: 0.72, y: 0.72))
        CATransaction.commit()
    }
}

/// Watches the scene and the screen for the remote view: releases held
/// input and covers the PC screen when the app leaves the foreground, ends
/// the connection in the background and reconnects on return (iOS suspends
/// the app, as Android ends the session when the app leaves the screen).
final class PrivacyGuard {
    private let input: RemoteInput
    private let center: NotificationCenter
    /// The view whose window gets the cover (the view itself without one).
    weak var host: UIView?
    /// `Settings.allowScreenshots`: no cover, no capture notice.
    var allowScreenshots = false {
        didSet { applyCapture() }
    }
    /// The app went to the background: end the connection.
    var onBackground: (() -> Void)?
    /// Back in the foreground after `onBackground` stopped a live session: reconnect.
    var onForeground: (() -> Void)?
    /// Whether the session is live (connecting, starting, streaming,
    /// reconnecting). Only a session the background stopped is reconnected;
    /// one that had ended or failed waits for the user, as on Android.
    var isLive: (() -> Bool)?
    /// Whether the picture must be hidden because the screen is captured.
    var onCaptureChanged: ((Bool) -> Void)? {
        didSet { applyCapture() }
    }

    private var cover: PrivacyCoverView?
    private var observers: [NSObjectProtocol] = []
    private var resumeOnForeground = false

    init(input: RemoteInput, center: NotificationCenter = .default) {
        self.input = input
        self.center = center
        // queue: nil runs each handler synchronously on the posting thread:
        // the cover must be in place before the snapshot is taken.
        observe(UIScene.willDeactivateNotification) { $0.willDeactivate() }
        observe(UIScene.didActivateNotification) { $0.hideCover() }
        observe(UIScene.didEnterBackgroundNotification) { $0.didEnterBackground() }
        observe(UIScene.willEnterForegroundNotification) { $0.willEnterForeground() }
        observe(UIScreen.capturedDidChangeNotification) { $0.applyCapture() }
    }

    deinit { stop() }

    private func observe(_ name: Notification.Name, _ handler: @escaping (PrivacyGuard) -> Void) {
        observers.append(center.addObserver(forName: name, object: nil, queue: nil) { [weak self] _ in
            guard let self else { return }
            handler(self)
        })
    }

    /// No more cover or callbacks (the remote screen is going away).
    func stop() {
        for o in observers { center.removeObserver(o) }
        observers.removeAll()
        hideCover()
    }

    var isCovered: Bool { cover != nil }

    private func willDeactivate() {
        // Keys or a button held when the user leaves must not stay down on
        // the PC.
        input.releaseAll()
        if !allowScreenshots { showCover() }
    }

    private func didEnterBackground() {
        // Read before stopping: stop() itself ends the session.
        resumeOnForeground = isLive?() ?? false
        onBackground?()
    }

    private func willEnterForeground() {
        guard resumeOnForeground else { return }
        resumeOnForeground = false
        onForeground?()
    }

    private func showCover() {
        guard cover == nil, let host else { return }
        let container: UIView = host.window ?? host
        let c = PrivacyCoverView(frame: container.bounds)
        container.addSubview(c)
        cover = c
    }

    private func hideCover() {
        cover?.removeFromSuperview()
        cover = nil
    }

    private func applyCapture() {
        onCaptureChanged?(!allowScreenshots && UIScreen.main.isCaptured)
    }
}

extension Phase {
    /// Connected or on the way there (what a trip to the background stops).
    var isLive: Bool {
        switch self {
        case .connecting, .starting, .streaming, .reconnecting: return true
        case .ended, .failed: return false
        }
    }
}
