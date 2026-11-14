// scripts/demo-video/assets.swift
// Renders the still layers used by compose.py:
//   frame   <out.png>                       3200x1800 backdrop with a rounded hole for the window + soft shadow
//   card    <intro|outro> <out.png> <icon>  1920x1080 title / closing cards
//   caption <out.png> <en> <zh> <index>     1920x1080 transparent lower-third caption

import AppKit
import Foundation

let args = CommandLine.arguments
guard args.count >= 3 else {
    FileHandle.standardError.write("usage: assets frame|card|caption ...\n".data(using: .utf8)!)
    exit(2)
}

// Geometry shared with compose.py.
let canvasW = 3200, canvasH = 1800
let winRect = NSRect(x: 320, y: 1800 - 120 - 1440, width: 2560, height: 1440) // AppKit origin is bottom-left
let winRadius: CGFloat = 40 // covers the macOS window corners (~19pt at 2x)

func rgb(_ hex: UInt32, _ a: CGFloat = 1) -> NSColor {
    NSColor(srgbRed: CGFloat((hex >> 16) & 0xff) / 255, green: CGFloat((hex >> 8) & 0xff) / 255,
            blue: CGFloat(hex & 0xff) / 255, alpha: a)
}

func render(_ w: Int, _ h: Int, to path: String, _ draw: (NSRect) -> Void) {
    guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: w, pixelsHigh: h, bitsPerSample: 8,
                                     samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                                     colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0) else { exit(1) }
    rep.size = NSSize(width: w, height: h)
    NSGraphicsContext.saveGraphicsState()
    let ctx = NSGraphicsContext(bitmapImageRep: rep)!
    NSGraphicsContext.current = ctx
    ctx.imageInterpolation = .high
    NSColor.clear.set()
    NSRect(x: 0, y: 0, width: w, height: h).fill(using: .copy)
    draw(NSRect(x: 0, y: 0, width: w, height: h))
    NSGraphicsContext.restoreGraphicsState()
    try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: path))
}

func backdrop(_ r: NSRect) {
    NSGradient(colors: [rgb(0x0b1020), rgb(0x111a33), rgb(0x1a1440)], atLocations: [0, 0.55, 1],
               colorSpace: .sRGB)!.draw(in: r, angle: -35)
    // Two soft colour glows so the backdrop is not a flat slab.
    let glows: [(NSPoint, CGFloat, NSColor)] = [
        (NSPoint(x: r.width * 0.18, y: r.height * 0.85), r.width * 0.42, rgb(0x3b82f6, 0.22)),
        (NSPoint(x: r.width * 0.88, y: r.height * 0.12), r.width * 0.40, rgb(0x8b5cf6, 0.20)),
    ]
    for (c, radius, color) in glows {
        NSGradient(starting: color, ending: color.withAlphaComponent(0))!
            .draw(fromCenter: c, radius: 0, toCenter: c, radius: radius, options: [])
    }
    // Fine dot grid.
    rgb(0xffffff, 0.035).setFill()
    let step = r.width / 64
    var y = step / 2
    while y < r.height {
        var x = step / 2
        while x < r.width {
            NSBezierPath(ovalIn: NSRect(x: x - 1.5, y: y - 1.5, width: 3, height: 3)).fill()
            x += step
        }
        y += step
    }
}

func drawText(_ s: String, font: NSFont, color: NSColor, centerX: CGFloat, y: CGFloat, kern: CGFloat = 0) -> NSSize {
    let attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color, .kern: kern]
    let str = NSAttributedString(string: s, attributes: attrs)
    let size = str.size()
    str.draw(at: NSPoint(x: centerX - size.width / 2, y: y))
    return size
}

func pill(_ label: String, font: NSFont, at origin: NSPoint, fill: NSColor, stroke: NSColor, text: NSColor) -> CGFloat {
    let attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: text]
    let str = NSAttributedString(string: label, attributes: attrs)
    let sz = str.size()
    let h = sz.height + 22
    let rect = NSRect(x: origin.x, y: origin.y, width: sz.width + 44, height: h)
    let path = NSBezierPath(roundedRect: rect, xRadius: h / 2, yRadius: h / 2)
    fill.setFill(); path.fill()
    stroke.setStroke(); path.lineWidth = 1.5; path.stroke()
    str.draw(at: NSPoint(x: rect.minX + 22, y: rect.minY + 11))
    return rect.width
}

func pillsRow(_ labels: [String], font: NSFont, centerX: CGFloat, y: CGFloat) {
    let attrs: [NSAttributedString.Key: Any] = [.font: font]
    let gap: CGFloat = 16
    let widths = labels.map { NSAttributedString(string: $0, attributes: attrs).size().width + 44 }
    var x = centerX - (widths.reduce(0, +) + gap * CGFloat(labels.count - 1)) / 2
    for (i, l) in labels.enumerated() {
        _ = pill(l, font: font, at: NSPoint(x: x, y: y), fill: rgb(0x16203a, 0.9), stroke: rgb(0x2c3d63), text: rgb(0xdbe4f3))
        x += widths[i] + gap
    }
}

let mode = args[1]
switch mode {
case "frame":
    render(canvasW, canvasH, to: args[2]) { r in
        backdrop(r)
        let hole = NSBezierPath(roundedRect: winRect, xRadius: winRadius, yRadius: winRadius)
        // Shadow: draw the window shape with a big blurred shadow, then punch the hole.
        NSGraphicsContext.saveGraphicsState()
        let shadow = NSShadow()
        shadow.shadowColor = NSColor.black.withAlphaComponent(0.65)
        shadow.shadowBlurRadius = 90
        shadow.shadowOffset = NSSize(width: 0, height: -36)
        shadow.set()
        NSColor.black.setFill()
        hole.fill()
        NSGraphicsContext.restoreGraphicsState()
        // Hairline highlight around the window edge.
        rgb(0xffffff, 0.14).setStroke()
        let edge = NSBezierPath(roundedRect: winRect.insetBy(dx: -1.5, dy: -1.5), xRadius: winRadius + 1.5, yRadius: winRadius + 1.5)
        edge.lineWidth = 3
        edge.stroke()
        NSGraphicsContext.current?.compositingOperation = .copy
        NSColor.clear.setFill()
        hole.fill()
    }

case "card":
    let kind = args[2], out = args[3], iconPath = args[4]
    render(1920, 1080, to: out) { r in
        backdrop(r)
        let icon = NSImage(contentsOfFile: iconPath)
        if kind == "intro" {
            icon?.draw(in: NSRect(x: 960 - 80, y: 640, width: 160, height: 160))
            _ = drawText("zniff", font: .systemFont(ofSize: 96, weight: .heavy), color: rgb(0xf8fafc), centerX: 960, y: 510, kern: -1)
            _ = drawText("A fast, native HTTP(S) debugging proxy — written in Rust",
                         font: .systemFont(ofSize: 34, weight: .medium), color: rgb(0xcbd5e1), centerX: 960, y: 440)
            _ = drawText("原生、轻快的 HTTP(S) 抓包调试工具",
                         font: .systemFont(ofSize: 30, weight: .regular), color: rgb(0x94a3b8), centerX: 960, y: 388)
            pillsRow(["HTTP/1.1 · HTTP/2 · WebSocket", "Breakpoints · 断点", "14 rewrite rules · 改写规则", "Built-in MCP"],
                     font: .systemFont(ofSize: 22, weight: .medium), centerX: 960, y: 270)
        } else {
            icon?.draw(in: NSRect(x: 960 - 64, y: 650, width: 128, height: 128))
            _ = drawText("zniff", font: .systemFont(ofSize: 84, weight: .heavy), color: rgb(0xf8fafc), centerX: 960, y: 530, kern: -1)
            _ = drawText("github.com/vicanso/zniff", font: .monospacedSystemFont(ofSize: 36, weight: .semibold),
                         color: rgb(0x93c5fd), centerX: 960, y: 450)
            _ = drawText("macOS · Linux · Windows · Web console", font: .systemFont(ofSize: 26, weight: .regular),
                         color: rgb(0x94a3b8), centerX: 960, y: 392)
            let cmd = "cargo install --git https://github.com/vicanso/zniff"
            let font = NSFont.monospacedSystemFont(ofSize: 24, weight: .regular)
            let w = NSAttributedString(string: cmd, attributes: [.font: font]).size().width + 44
            _ = pill(cmd, font: font, at: NSPoint(x: 960 - w / 2, y: 280), fill: rgb(0x0f172a, 0.95),
                     stroke: rgb(0x334155), text: rgb(0xe2e8f0))
        }
    }

case "caption":
    let out = args[2], en = args[3], zh = args[4], index = args.count > 5 ? args[5] : ""
    render(1920, 1080, to: out) { _ in
        let enFont = NSFont.systemFont(ofSize: 34, weight: .semibold)
        let zhFont = NSFont.systemFont(ofSize: 24, weight: .regular)
        let idxFont = NSFont.monospacedDigitSystemFont(ofSize: 22, weight: .bold)
        let enStr = NSAttributedString(string: en, attributes: [.font: enFont, .foregroundColor: rgb(0xf8fafc)])
        let zhStr = NSAttributedString(string: zh, attributes: [.font: zhFont, .foregroundColor: rgb(0xa5b4cf)])
        let idxStr = NSAttributedString(string: index, attributes: [.font: idxFont, .foregroundColor: rgb(0x0b1020)])
        let idxW = index.isEmpty ? 0 : idxStr.size().width + 26
        let textW = max(enStr.size().width, zhStr.size().width)
        let padX: CGFloat = 30, h: CGFloat = 104
        let w = padX * 2 + textW + (idxW > 0 ? idxW + 22 : 0)
        let rect = NSRect(x: 960 - w / 2, y: 26, width: w, height: h)
        NSGraphicsContext.saveGraphicsState()
        let sh = NSShadow()
        sh.shadowColor = NSColor.black.withAlphaComponent(0.55)
        sh.shadowBlurRadius = 30
        sh.shadowOffset = NSSize(width: 0, height: -8)
        sh.set()
        rgb(0x0e1528, 0.92).setFill()
        NSBezierPath(roundedRect: rect, xRadius: 22, yRadius: 22).fill()
        NSGraphicsContext.restoreGraphicsState()
        rgb(0xffffff, 0.10).setStroke()
        let border = NSBezierPath(roundedRect: rect.insetBy(dx: 0.75, dy: 0.75), xRadius: 22, yRadius: 22)
        border.lineWidth = 1.5
        border.stroke()
        var x = rect.minX + padX
        if idxW > 0 {
            let badge = NSRect(x: x, y: rect.midY - 20, width: idxW, height: 40)
            NSGradient(starting: rgb(0x60a5fa), ending: rgb(0xa78bfa))!.draw(in: NSBezierPath(roundedRect: badge, xRadius: 12, yRadius: 12), angle: 0)
            idxStr.draw(at: NSPoint(x: badge.minX + 13, y: badge.midY - idxStr.size().height / 2))
            x += idxW + 22
        }
        enStr.draw(at: NSPoint(x: x, y: rect.minY + 50))
        zhStr.draw(at: NSPoint(x: x, y: rect.minY + 18))
    }

default:
    exit(2)
}
