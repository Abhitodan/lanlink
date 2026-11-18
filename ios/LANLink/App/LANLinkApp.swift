import AVFoundation
import Combine
import SwiftUI
import UIKit

@main
struct LANLinkApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var model: AppModel

    init() {
        let args = ProcessInfo.processInfo.arguments
        let m: AppModel
        if args.contains(UITestSeed.remoteDemoArgument) {
            m = UITestSeed.makeRemoteDemoModel()
        } else if args.contains(UITestSeed.argument) {
            m = UITestSeed.makeModel()
        } else {
            m = AppModel()
        }
        _model = StateObject(wrappedValue: m)
    }

    var body: some Scene {
        WindowGroup {
            AppRoot(model: model)
                .onOpenURL { url in
                    // A link can come from any web page or app, so it only
                    // proposes pairing; nothing connects until the user
                    // confirms on screen. Onboarding is completed only if
                    // the user accepts (acceptOffer). iOS delivers each
                    // open once.
                    guard url.scheme?.lowercased() == "lanlink", url.host?.lowercased() == "pair" else { return }
                    model.go(.pair())
                    model.offerPairFromLink(url.absoluteString)
                }
        }
    }
}

/// UI tests only (`-uiTestSeed`): the app runs on stores of its own,
/// emptied at each launch and holding just the fictional "Studio PC", with
/// onboarding done. The app's real stores are not touched.
@MainActor
enum UITestSeed {
    static let argument = "-uiTestSeed"
    /// The remote screen on a still test frame, with no network at all.
    static let remoteDemoArgument = "-uiTestRemoteDemo"

    /// The seeded model, already on the remote screen with a demo session
    /// (`DemoRemoteSession`) in place of `RemoteSession`.
    static func makeRemoteDemoModel() -> AppModel {
        let seeded = makeModel(makeSession: { host, _ in DemoRemoteSession(host: host) })
        // Upright, like docs/media/compact-keyboard.png (and XCUITest cannot
        // capture an interface the app turned sideways on an upright simulator).
        seeded.settings.landscape = false
        if let host = seeded.hosts.first { seeded.connect(host) }
        return seeded
    }

    static func makeModel(makeSession: ((PairedHost, String?) -> SessionControl)? = nil) -> AppModel {
        let settings = Settings(defaults: fresh("uitest.settings"))
        let repository = HostRepository(defaults: fresh("uitest.hosts"))
        settings.onboarded = true
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let day: Int64 = 24 * 3600 * 1000
        repository.put(PairedHost(
            fingerprint: "5a3c9e17b2d84f60a1c7e9b35d2f8046c1e7a93b5d28f4061ac3e7b95d1f2a84",
            name: "Studio PC",
            addresses: ["192.168.1.20"],
            port: 47990,
            pairedAt: now - 3 * day,
            lastConnected: now - day))
        return AppModel(
            settings: settings,
            repository: repository,
            identity: IdentityStore(service: IdentityStore.defaultService + ".uitest"),
            makeSession: makeSession)
    }

    private static func fresh(_ suite: String) -> UserDefaults {
        let d = UserDefaults(suiteName: suite) ?? .standard
        d.removePersistentDomain(forName: suite)
        return d
    }
}

/// Which orientations the app allows (`supportedInterfaceOrientations`):
/// landscape only while controlling the PC when the setting asks for it.
enum OrientationLock {
    static var mask: UIInterfaceOrientationMask = .allButUpsideDown

    @MainActor
    static func set(_ m: UIInterfaceOrientationMask) {
        mask = m
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        for scene in scenes {
            for w in scene.windows { w.rootViewController?.setNeedsUpdateOfSupportedInterfaceOrientations() }
            scene.requestGeometryUpdate(.iOS(interfaceOrientations: m))
        }
    }
}

final class AppDelegate: NSObject, UIApplicationDelegate {
    func application(
        _ application: UIApplication, supportedInterfaceOrientationsFor window: UIWindow?
    ) -> UIInterfaceOrientationMask {
        OrientationLock.mask
    }
}

/// UI tests only (`-uiTestRemoteDemo`): a session that is "streaming" a
/// drawn still frame of a fictional desktop. It opens no connection; input
/// goes nowhere except the sticky modifiers, which light up as on a PC.
final class DemoRemoteSession: ObservableObject, RemoteScreenSession {
    @Published private(set) var state: SessionState
    let stillFrame: CGImage?

    weak var cursorSink: CursorSink? {
        didSet {
            guard let sink = cursorSink else { return }
            DispatchQueue.main.async {
                sink.onVideoGeometry(width: 1280, height: 800, sourceWidth: 2560, sourceHeight: 1600)
                sink.onCursorPosition(nx: 32768, ny: 32768, visible: true)
            }
        }
    }

    init(host: PairedHost) {
        var s = SessionState()
        s.phase = .streaming
        s.hostName = host.name
        s.displays = [DisplayItem(id: 1, name: "Display 1", width: 2560, height: 1600, primary: true)]
        s.displayId = 1
        state = s
        stillFrame = Self.drawDesktop()
    }

    func start() {}
    func stop() {}
    func retry() {}
    func attachLayer(_ l: AVSampleBufferDisplayLayer) {}
    func detachLayer() {}
    func selectDisplay(id: Int64) {}
    func applyQuality() {}

    func pointer(x: Int, y: Int) {}
    func pointerBy(dx: Int, dy: Int) {}
    func button(_ button: Int, down: Bool, x: Int, y: Int) {}
    func scroll(dx: Int, dy: Int) {}
    func key(_ vk: Int, down: Bool) {}
    func tapKey(_ vk: Int) {}
    func text(_ t: String) {}
    func setTextInput(_ active: Bool) {}
    func releaseAll() {}
    func combo(vks: Int...) {}

    func toggleSticky(_ vk: Int) {
        if state.modifiers.contains(vk) { state.modifiers.remove(vk) } else { state.modifiers.insert(vk) }
    }

    /// A plain desktop: wallpaper, one window with lines of "text", a taskbar.
    private static func drawDesktop() -> CGImage? {
        let size = CGSize(width: 1280, height: 800)
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        let image = UIGraphicsImageRenderer(size: size, format: format).image { ctx in
            let c = ctx.cgContext
            Palette.blue.setFill()
            c.fill(CGRect(origin: .zero, size: size))
            UIColor(hex: 0x3B5B84).setFill()
            c.fillEllipse(in: CGRect(x: 720, y: 260, width: 760, height: 760))
            let window = CGRect(x: 180, y: 110, width: 760, height: 470)
            Palette.ivory.setFill()
            UIBezierPath(roundedRect: window, cornerRadius: 10).fill()
            Palette.ivoryDeep.setFill()
            UIBezierPath(
                roundedRect: CGRect(x: window.minX, y: window.minY, width: window.width, height: 44),
                byRoundingCorners: [.topLeft, .topRight], cornerRadii: CGSize(width: 10, height: 10)
            ).fill()
            let title: [NSAttributedString.Key: Any] = [
                .font: UIFont.systemFont(ofSize: 18, weight: .medium), .foregroundColor: Palette.graphite,
            ]
            ("Notes" as NSString).draw(at: CGPoint(x: window.minX + 20, y: window.minY + 11), withAttributes: title)
            Palette.line.setFill()
            for (i, w) in [620.0, 540, 660, 410, 590, 300].enumerated() {
                c.fill(CGRect(x: window.minX + 32, y: window.minY + 80 + CGFloat(i) * 44, width: w, height: 14))
            }
            Palette.graphite.setFill()
            c.fill(CGRect(x: 0, y: size.height - 48, width: size.width, height: 48))
            Palette.sandstone.setFill()
            c.fillEllipse(in: CGRect(x: 20, y: size.height - 38, width: 28, height: 28))
        }
        return image.cgImage
    }
}
