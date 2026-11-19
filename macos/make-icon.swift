// Draws the gh-watch icon (a cat's face on a dusky rounded square) into an
// .iconset folder, which `iconutil` turns into AppIcon.icns:
//
//   swift make-icon.swift AppIcon.iconset && iconutil -c icns AppIcon.iconset
//
// build.sh does this on every build.

import AppKit

func drawIcon(size: Int) -> Data {
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let ctx = NSGraphicsContext.current!.cgContext

    // Draw on Apple's 1024-point icon grid, scaled to this size.
    let scale = CGFloat(size) / 1024
    ctx.scaleBy(x: scale, y: scale)

    // The rounded square, inset like other macOS icons, with a soft shadow.
    let body = NSRect(x: 100, y: 100, width: 824, height: 824)
    let shape = NSBezierPath(roundedRect: body, xRadius: 186, yRadius: 186)
    NSGraphicsContext.saveGraphicsState()
    let shadow = NSShadow()
    shadow.shadowColor = NSColor.black.withAlphaComponent(0.3)
    shadow.shadowOffset = NSSize(width: 0, height: -10)
    shadow.shadowBlurRadius = 24
    shadow.set()
    NSColor.black.setFill()
    shape.fill()
    NSGraphicsContext.restoreGraphicsState()

    // Dusk: plum at the top to deep navy.
    NSGraphicsContext.saveGraphicsState()
    shape.addClip()
    NSGradient(colors: [
        NSColor(srgbRed: 0.42, green: 0.30, blue: 0.62, alpha: 1),
        NSColor(srgbRed: 0.22, green: 0.20, blue: 0.45, alpha: 1),
        NSColor(srgbRed: 0.10, green: 0.11, blue: 0.27, alpha: 1),
    ])!.draw(in: body, angle: -70)
    NSGradient(colors: [
        NSColor.white.withAlphaComponent(0.14),
        NSColor.white.withAlphaComponent(0),
    ])!.draw(in: NSRect(x: 100, y: 512, width: 824, height: 412), angle: -90)
    NSGraphicsContext.restoreGraphicsState()

    let face = NSColor(srgbRed: 0.98, green: 0.97, blue: 0.95, alpha: 1)
    let ink = NSColor(srgbRed: 0.13, green: 0.12, blue: 0.25, alpha: 1)
    let pink = NSColor(srgbRed: 0.95, green: 0.60, blue: 0.66, alpha: 1)

    // The head: a wide rounded face with two pointed ears. They're filled
    // as separate shapes inside one transparency layer, so overlaps merge
    // (one combined path would cancel out where a mirrored ear crosses
    // the face) and the shadow falls once, under all of it.
    var parts = [NSBezierPath(ovalIn: NSRect(x: 262, y: 250, width: 500, height: 420))]
    for side in [-1.0, 1.0] as [CGFloat] {
        let ear = NSBezierPath()
        ear.move(to: NSPoint(x: 512 + side * 238, y: 520))
        ear.curve(
            to: NSPoint(x: 512 + side * 210, y: 800),
            controlPoint1: NSPoint(x: 512 + side * 250, y: 640),
            controlPoint2: NSPoint(x: 512 + side * 228, y: 740))
        ear.curve(
            to: NSPoint(x: 512 + side * 60, y: 650),
            controlPoint1: NSPoint(x: 512 + side * 170, y: 780),
            controlPoint2: NSPoint(x: 512 + side * 100, y: 700))
        ear.close()
        parts.append(ear)
    }
    ctx.saveGState()
    ctx.setShadow(
        offset: CGSize(width: 0, height: -14), blur: 26,
        color: CGColor(srgbRed: 0.03, green: 0.03, blue: 0.12, alpha: 0.45))
    ctx.beginTransparencyLayer(auxiliaryInfo: nil)
    face.setFill()
    for part in parts { part.fill() }
    ctx.endTransparencyLayer()
    ctx.restoreGState()

    // Inside of the ears.
    pink.withAlphaComponent(0.75).setFill()
    for side in [-1.0, 1.0] as [CGFloat] {
        let inner = NSBezierPath()
        inner.move(to: NSPoint(x: 512 + side * 200, y: 610))
        inner.curve(
            to: NSPoint(x: 512 + side * 196, y: 740),
            controlPoint1: NSPoint(x: 512 + side * 212, y: 660),
            controlPoint2: NSPoint(x: 512 + side * 206, y: 710))
        inner.curve(
            to: NSPoint(x: 512 + side * 110, y: 648),
            controlPoint1: NSPoint(x: 512 + side * 170, y: 720),
            controlPoint2: NSPoint(x: 512 + side * 130, y: 680))
        inner.close()
        inner.fill()
    }

    // Eyes, a little wide and watchful, with a glint each.
    for side in [-1.0, 1.0] as [CGFloat] {
        ink.setFill()
        NSBezierPath(ovalIn: NSRect(x: 512 + side * 105 - 38, y: 440, width: 76, height: 96)).fill()
        NSColor.white.setFill()
        NSBezierPath(ovalIn: NSRect(x: 512 + side * 105 - 8, y: 500, width: 22, height: 22)).fill()
    }

    // Nose and mouth.
    let nose = NSBezierPath()
    nose.move(to: NSPoint(x: 486, y: 418))
    nose.line(to: NSPoint(x: 538, y: 418))
    nose.line(to: NSPoint(x: 512, y: 388))
    nose.close()
    pink.setFill()
    nose.fill()

    let mouth = NSBezierPath()
    mouth.move(to: NSPoint(x: 512, y: 390))
    mouth.line(to: NSPoint(x: 512, y: 366))
    mouth.move(to: NSPoint(x: 512, y: 366))
    mouth.curve(
        to: NSPoint(x: 462, y: 360),
        controlPoint1: NSPoint(x: 500, y: 340),
        controlPoint2: NSPoint(x: 474, y: 340))
    mouth.move(to: NSPoint(x: 512, y: 366))
    mouth.curve(
        to: NSPoint(x: 562, y: 360),
        controlPoint1: NSPoint(x: 524, y: 340),
        controlPoint2: NSPoint(x: 550, y: 340))
    ink.setStroke()
    mouth.lineWidth = 12
    mouth.lineCapStyle = .round
    mouth.stroke()

    // Whiskers, reaching just past the face.
    let whiskers = NSBezierPath()
    for side in [-1.0, 1.0] as [CGFloat] {
        for (i, dy) in ([24, 0, -24] as [CGFloat]).enumerated() {
            let start = NSPoint(x: 512 + side * 130, y: 385 + dy)
            let end = NSPoint(x: 512 + side * 330, y: 400 + dy * 2.2 - CGFloat(i) * 4)
            whiskers.move(to: start)
            whiskers.line(to: end)
        }
    }
    NSColor(srgbRed: 0.62, green: 0.60, blue: 0.72, alpha: 1).setStroke()
    whiskers.lineWidth = 9
    whiskers.lineCapStyle = .round
    whiskers.stroke()

    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

let out = URL(fileURLWithPath: CommandLine.arguments.count > 1
    ? CommandLine.arguments[1] : "AppIcon.iconset")
try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
for points in [16, 32, 128, 256, 512] {
    for factor in [1, 2] {
        let name = factor == 1 ? "icon_\(points)x\(points).png" : "icon_\(points)x\(points)@2x.png"
        try drawIcon(size: points * factor).write(to: out.appendingPathComponent(name))
    }
}
