// Renders an SVG to PNG at its declared size (times an optional scale)
// using AppKit, so layout checks do not depend on Quick Look thumbnails.
// usage: swift tools/svg2png.swift in.svg out.png [scale]
import AppKit

let args = CommandLine.arguments
guard args.count >= 3, let image = NSImage(contentsOfFile: args[1]) else {
    FileHandle.standardError.write("usage: svg2png in.svg out.png [scale]\n".data(using: .utf8)!)
    exit(1)
}
let scale = args.count > 3 ? Double(args[3]) ?? 1.0 : 1.0
let w = Int(Double(image.size.width) * scale)
let h = Int(Double(image.size.height) * scale)
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: w, pixelsHigh: h, bitsPerSample: 8,
                           samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                           colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
image.draw(in: NSRect(x: 0, y: 0, width: w, height: h))
NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: args[2]))
print("wrote \(args[2]) \(w)x\(h)")
