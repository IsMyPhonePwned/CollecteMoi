import AppKit
import Foundation

func fail(_ message: String, code: Int32 = 1) -> Never {
    fputs("\(message)\n", stderr)
    exit(code)
}

func color(_ hex: String) -> NSColor? {
    let value = hex.trimmingCharacters(in: CharacterSet(charactersIn: "#"))
    guard value.count == 6, let rgb = UInt32(value, radix: 16) else { return nil }
    return NSColor(
        srgbRed: CGFloat((rgb >> 16) & 0xff) / 255,
        green: CGFloat((rgb >> 8) & 0xff) / 255,
        blue: CGFloat(rgb & 0xff) / 255,
        alpha: 1
    )
}

func font(_ name: String, size: CGFloat) -> NSFont {
    switch name {
    case "rounded":
        return NSFont(name: "AvenirNext-DemiBold", size: size)
            ?? NSFont.systemFont(ofSize: size, weight: .bold)
    case "serif":
        return NSFont(name: "Georgia-Bold", size: size)
            ?? NSFont.systemFont(ofSize: size, weight: .bold)
    case "monospaced":
        return NSFont.monospacedSystemFont(ofSize: size, weight: .bold)
    default:
        return NSFont.systemFont(ofSize: size, weight: .heavy)
    }
}

guard CommandLine.arguments.count == 14 else {
    fail("usage: overlay-icon-text.swift <source> <text> <output> <font> <size> <x> <y> <color> <badge> <badge-color> <opacity> <scale> <rotation>", code: 2)
}

let args = CommandLine.arguments
let sourcePath = args[1]
let label = args[2].trimmingCharacters(in: .whitespacesAndNewlines)
let outputPath = args[3]
let fontName = args[4]
guard let sizePercent = Int(args[5]), (8...32).contains(sizePercent),
      let xPercent = Int(args[6]), (0...100).contains(xPercent),
      let yPercent = Int(args[7]), (0...100).contains(yPercent),
      let opacity = Int(args[11]), (0...100).contains(opacity),
      let scale = Int(args[12]), (40...180).contains(scale),
      let rotation = Int(args[13]), (-180...180).contains(rotation),
      ["system", "rounded", "serif", "monospaced"].contains(fontName),
      ["true", "false"].contains(args[9]),
      let textColor = color(args[8]), let badgeColor = color(args[10]),
      label.count <= 24,
      label.rangeOfCharacter(from: .controlCharacters) == nil,
      let source = NSImage(contentsOfFile: sourcePath),
      let sourceCG = source.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
    fail("invalid launcher text style")
}

let side = 1024
let sourceWidth = CGFloat(sourceCG.width)
let sourceHeight = CGFloat(sourceCG.height)
let sourceMaxDimension = max(sourceWidth, sourceHeight)
let imageSide = CGFloat(side) * CGFloat(scale) / 100
let drawWidth = imageSide * sourceWidth / sourceMaxDimension
let drawHeight = imageSide * sourceHeight / sourceMaxDimension
guard let bitmap = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: side,
    pixelsHigh: side,
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: 0,
    bitsPerPixel: 0
), let context = NSGraphicsContext(bitmapImageRep: bitmap) else {
    fail("could not create icon bitmap")
}

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = context
context.cgContext.saveGState()
context.cgContext.translateBy(x: CGFloat(side) / 2, y: CGFloat(side) / 2)
context.cgContext.rotate(by: -CGFloat(rotation) * .pi / 180)
NSImage(cgImage: sourceCG, size: NSSize(width: sourceWidth, height: sourceHeight)).draw(
    in: NSRect(x: -drawWidth / 2, y: -drawHeight / 2, width: drawWidth, height: drawHeight),
    from: .zero,
    operation: .copy,
    fraction: 1
)
context.cgContext.restoreGState()

if !label.isEmpty {
    let fontSize = CGFloat(side) * CGFloat(sizePercent) / 100
    var attributes: [NSAttributedString.Key: Any] = [
        .font: font(fontName, size: fontSize),
        .foregroundColor: textColor,
    ]
    var textSize = (label as NSString).size(withAttributes: attributes)
    if textSize.width > CGFloat(side - 32) {
        let fittedSize = fontSize * CGFloat(side - 32) / textSize.width
        attributes[.font] = font(fontName, size: fittedSize)
        textSize = (label as NSString).size(withAttributes: attributes)
    }

    let centerX = CGFloat(xPercent) * CGFloat(side) / 100
    let centerY = CGFloat(100 - yPercent) * CGFloat(side) / 100
    let textRect = NSRect(
        x: centerX - textSize.width / 2,
        y: centerY - textSize.height / 2,
        width: textSize.width,
        height: textSize.height
    )
    if args[9] == "true" && opacity > 0 {
        let badge = textRect.insetBy(dx: -max(22, fontSize * 0.16), dy: -max(14, fontSize * 0.12))
        badgeColor.withAlphaComponent(CGFloat(opacity) / 100).setFill()
        NSBezierPath(roundedRect: badge, xRadius: fontSize * 0.18, yRadius: fontSize * 0.18).fill()
    }
    (label as NSString).draw(at: textRect.origin, withAttributes: attributes)
}
NSGraphicsContext.restoreGraphicsState()

guard let png = bitmap.representation(using: .png, properties: [:]) else {
    fail("could not encode icon PNG")
}
do {
    try png.write(to: URL(fileURLWithPath: outputPath), options: .atomic)
} catch {
    fail("could not write output icon: \(error)")
}
