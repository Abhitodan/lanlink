import SwiftUI

/// Line icons drawn for LANLink, ported path-for-path from
/// android/.../ui/theme/Glyphs.kt. Each is drawn on a 24x24 grid and scaled
/// to its frame; `GlyphIcon` strokes it at 1.8 units with round caps/joins.
struct Glyph: Shape {
    let name: String
    /// The path on the 24x24 grid.
    let unitPath: Path

    func path(in rect: CGRect) -> Path {
        let side = min(rect.width, rect.height)
        let scale = side / 24
        let transform = CGAffineTransform(
            translationX: rect.minX + (rect.width - side) / 2,
            y: rect.minY + (rect.height - side) / 2
        ).scaledBy(x: scale, y: scale)
        return unitPath.applying(transform)
    }
}

/// Mirrors Compose's PathBuilder so the ports read like the Kotlin.
private struct GlyphPath {
    var path = Path()

    mutating func moveTo(_ x: CGFloat, _ y: CGFloat) {
        path.move(to: CGPoint(x: x, y: y))
    }

    mutating func lineTo(_ x: CGFloat, _ y: CGFloat) {
        path.addLine(to: CGPoint(x: x, y: y))
    }

    mutating func curveTo(
        _ x1: CGFloat, _ y1: CGFloat, _ x2: CGFloat, _ y2: CGFloat, _ x3: CGFloat, _ y3: CGFloat
    ) {
        path.addCurve(
            to: CGPoint(x: x3, y: y3),
            control1: CGPoint(x: x1, y: y1),
            control2: CGPoint(x: x2, y: y2)
        )
    }

    mutating func close() {
        path.closeSubpath()
    }
}

private func icon(_ name: String, _ block: (inout GlyphPath) -> Void) -> Glyph {
    var p = GlyphPath()
    block(&p)
    return Glyph(name: name, unitPath: p.path)
}

extension Glyph {
    static let menu = icon("menu") { p in
        p.moveTo(5, 7); p.lineTo(19, 7); p.moveTo(5, 12); p.lineTo(19, 12); p.moveTo(5, 17); p.lineTo(19, 17)
    }
    static let close = icon("close") { p in
        p.moveTo(6.5, 6.5); p.lineTo(17.5, 17.5); p.moveTo(17.5, 6.5); p.lineTo(6.5, 17.5)
    }
    static let laptop = icon("laptop") { p in
        p.moveTo(5, 6); p.lineTo(19, 6); p.lineTo(19, 15); p.lineTo(5, 15); p.close()
        p.moveTo(3, 18.5); p.lineTo(21, 18.5)
    }
    static let keyboard = icon("keyboard") { p in
        p.moveTo(3, 7); p.lineTo(21, 7); p.lineTo(21, 17); p.lineTo(3, 17); p.close()
        p.moveTo(6.5, 10.5); p.lineTo(7, 10.5); p.moveTo(10, 10.5); p.lineTo(10.5, 10.5)
        p.moveTo(13.5, 10.5); p.lineTo(14, 10.5); p.moveTo(17, 10.5); p.lineTo(17.5, 10.5)
        p.moveTo(8, 14); p.lineTo(16, 14)
    }
    static let mouse = icon("mouse") { p in
        p.moveTo(12, 3.5); p.curveTo(8.5, 3.5, 7, 6, 7, 9); p.lineTo(7, 15)
        p.curveTo(7, 18.5, 9.5, 20.5, 12, 20.5); p.curveTo(14.5, 20.5, 17, 18.5, 17, 15)
        p.lineTo(17, 9); p.curveTo(17, 6, 15.5, 3.5, 12, 3.5); p.close()
        p.moveTo(12, 6.5); p.lineTo(12, 9.5)
    }
    static let touch = icon("touch") { p in
        p.moveTo(9, 11); p.lineTo(9, 5.5); p.curveTo(9, 4.5, 10.5, 4.5, 10.5, 5.5); p.lineTo(10.5, 11)
        p.moveTo(10.5, 10); p.curveTo(10.5, 9, 12, 9, 12, 10); p.lineTo(12, 11.5)
        p.moveTo(12, 10.5); p.curveTo(12, 9.5, 13.5, 9.5, 13.5, 10.5); p.lineTo(13.5, 12)
        p.moveTo(13.5, 11); p.curveTo(13.5, 10, 15, 10, 15, 11); p.lineTo(15, 15)
        p.curveTo(15, 18, 13, 20, 11, 20); p.curveTo(9, 20, 8, 19, 7, 17); p.lineTo(5.5, 14)
        p.curveTo(5, 13, 6.5, 12, 7.5, 13); p.lineTo(9, 14.5)
    }
    static let monitor = icon("monitor") { p in
        p.moveTo(3.5, 5); p.lineTo(20.5, 5); p.lineTo(20.5, 16); p.lineTo(3.5, 16); p.close()
        p.moveTo(9, 20); p.lineTo(15, 20); p.moveTo(12, 16); p.lineTo(12, 20)
    }
    static let gauge = icon("gauge") { p in
        p.moveTo(4, 16); p.curveTo(4, 11, 7.5, 7, 12, 7); p.curveTo(16.5, 7, 20, 11, 20, 16)
        p.moveTo(12, 16); p.lineTo(15.5, 11)
    }
    static let quality = icon("quality") { p in
        p.moveTo(4, 18); p.lineTo(4, 14); p.moveTo(9.3, 18); p.lineTo(9.3, 10)
        p.moveTo(14.6, 18); p.lineTo(14.6, 7); p.moveTo(20, 18); p.lineTo(20, 4)
    }
    static let power = icon("power") { p in
        p.moveTo(12, 3.5); p.lineTo(12, 11)
        p.moveTo(7.5, 6.5); p.curveTo(5.5, 8, 4.5, 10, 4.5, 12.5); p.curveTo(4.5, 16.6, 7.9, 20, 12, 20)
        p.curveTo(16.1, 20, 19.5, 16.6, 19.5, 12.5); p.curveTo(19.5, 10, 18.5, 8, 16.5, 6.5)
    }
    static let zoom = icon("zoom") { p in
        p.moveTo(10.5, 4.5); p.curveTo(13.8, 4.5, 16.5, 7.2, 16.5, 10.5); p.curveTo(16.5, 13.8, 13.8, 16.5, 10.5, 16.5)
        p.curveTo(7.2, 16.5, 4.5, 13.8, 4.5, 10.5); p.curveTo(4.5, 7.2, 7.2, 4.5, 10.5, 4.5); p.close()
        p.moveTo(15, 15); p.lineTo(20, 20); p.moveTo(8, 10.5); p.lineTo(13, 10.5)
    }
    static let shield = icon("shield") { p in
        p.moveTo(12, 3.5); p.lineTo(19, 6); p.lineTo(19, 11.5); p.curveTo(19, 15.5, 16, 19, 12, 20.5)
        p.curveTo(8, 19, 5, 15.5, 5, 11.5); p.lineTo(5, 6); p.close()
    }
    static let qrScan = icon("qr") { p in
        p.moveTo(4, 9); p.lineTo(4, 4); p.lineTo(9, 4); p.moveTo(15, 4); p.lineTo(20, 4); p.lineTo(20, 9)
        p.moveTo(20, 15); p.lineTo(20, 20); p.lineTo(15, 20); p.moveTo(9, 20); p.lineTo(4, 20); p.lineTo(4, 15)
        p.moveTo(8, 12); p.lineTo(16, 12)
    }

    static let all: [Glyph] = [
        .menu, .close, .laptop, .keyboard, .mouse, .touch, .monitor,
        .gauge, .quality, .power, .zoom, .shield, .qrScan,
    ]
}

/// A glyph stroked in the current foreground colour, like Compose's `Icon`
/// (24 pt by default; the stroke scales with the size).
struct GlyphIcon: View {
    let glyph: Glyph
    var size: CGFloat = 24

    init(_ glyph: Glyph, size: CGFloat = 24) {
        self.glyph = glyph
        self.size = size
    }

    var body: some View {
        glyph
            .stroke(style: StrokeStyle(lineWidth: 1.8 * size / 24, lineCap: .round, lineJoin: .round))
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}
