// Applies a rounded-rect mask (macOS icon style: ~22% radius, 10% inset)
// to a square PNG. usage: swift tools/round_icon.swift in.png out.png
import AppKit

let args = CommandLine.arguments
guard args.count >= 3, let image = NSImage(contentsOfFile: args[1]) else { exit(1) }
let size = Int(image.size.width)
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8,
                           samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                           colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
let inset = CGFloat(size) * 0.10
let rect = NSRect(x: inset, y: inset, width: CGFloat(size) - 2 * inset, height: CGFloat(size) - 2 * inset)
let path = NSBezierPath(roundedRect: rect, xRadius: rect.width * 0.225, yRadius: rect.height * 0.225)
path.addClip()
image.draw(in: rect)
NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: args[2]))
