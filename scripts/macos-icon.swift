// Renders packaging/macos/icon.png, the macOS app icon, from the square
// artwork: the 824 pt rounded square with continuous corners that macOS
// uses, centred on a transparent 1024 canvas. macOS 26 shows icons of any
// other shape on a grey plate.
//
//   swift scripts/macos-icon.swift packaging/artwork/bars-icon-square.png packaging/macos/icon.png
import AppKit
import QuartzCore

let args = CommandLine.arguments
guard args.count == 3,
      let art = NSImage(contentsOfFile: args[1])?.cgImage(forProposedRect: nil, context: nil, hints: nil)
else {
    FileHandle.standardError.write("usage: macos-icon.swift <square artwork> <out.png>\n".data(using: .utf8)!)
    exit(2)
}
let canvas = 1024
let side: CGFloat = 824
let root = CALayer()
root.frame = CGRect(x: 0, y: 0, width: canvas, height: canvas)
let tile = CALayer()
tile.frame = CGRect(x: (CGFloat(canvas) - side) / 2, y: (CGFloat(canvas) - side) / 2, width: side, height: side)
tile.contents = art
tile.contentsGravity = .resize
tile.cornerRadius = side * 0.2237
tile.cornerCurve = .continuous
tile.masksToBounds = true
root.addSublayer(tile)
let ctx = CGContext(
    data: nil, width: canvas, height: canvas, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
root.render(in: ctx)
try NSBitmapImageRep(cgImage: ctx.makeImage()!).representation(using: .png, properties: [:])!
    .write(to: URL(fileURLWithPath: args[2]))
