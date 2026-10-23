import UIKit

/// The app's marks: Iconoir's glyphs, the seal and the seasons' kanji (`Iconoir.swift`, which
/// tools/glyphs.py writes), drawn as templates at the size each place needs. iOS 12 has no SF Symbols.
enum Glyph {
    /// The tab bar's.
    static let home = draw(Iconoir.homeSimple, 21)
    static let library = draw(Iconoir.albumList, 21)
    static let search = draw(Iconoir.search, 21)
    static let settings = draw(Iconoir.settings, 21)
    /// The mini player's and the pills'.
    static let play = draw(Iconoir.playSolid, 22)
    static let pause = draw(Iconoir.pauseSolid, 22)
    /// The player's transport.
    static let bigPlay = draw(Iconoir.playSolid, 60)
    static let bigPause = draw(Iconoir.pauseSolid, 60)
    static let bigNext = draw(Iconoir.forwardSolid, 38)
    static let bigPrevious = draw(Iconoir.rewindSolid, 38)
    static let shuffle = draw(Iconoir.shuffle, 22)
    static let repeatAll = draw(Iconoir.`repeat`, 22)
    static let repeatOne = draw(Iconoir.repeatOnce, 22)
    /// The player's panels.
    static let lyrics = draw(Iconoir.emojiSingRightNote, 24)
    static let queue = draw(Iconoir.playlist, 24)
    /// A jam: someone listens (lit) or no one yet.
    static let jam = draw(Iconoir.community, 22)
    /// A queue row's handle: hold and drag it to move the song.
    static let grip = draw(Iconoir.menu, 22)
    /// A favorite and not one: the seal, on the player and the pages.
    static let sealed = draw(Iconoir.seal, 28)
    static let seal = draw(Iconoir.sealOutline, 28)
    /// The mini player's seal.
    static let sealedSmall = draw(Iconoir.seal, 22)
    static let sealSmall = draw(Iconoir.sealOutline, 22)
    /// The seal a favorite row carries.
    static let sealedRow = draw(Iconoir.seal, 15)
    static let more = draw(Iconoir.moreHoriz, 28)
    static let moreSmall = draw(Iconoir.moreHoriz, 22)
    static let close = draw(Iconoir.navArrowDown, 28)
    /// A fold's chevron, shut and open.
    static let down = draw(Iconoir.navArrowDown, 22)
    static let up = UIImage(cgImage: down.cgImage!, scale: down.scale, orientation: .down).withRenderingMode(.alwaysTemplate)
    /// The navigation bar's way back.
    static let back = draw(Iconoir.arrowLeft, 22)
    /// A pill's glyph.
    static let playPill = draw(Iconoir.playSolid, 18)
    static let pausePill = draw(Iconoir.pauseSolid, 18)
    static let xmark = draw(Iconoir.xmark, 18)
    /// A row title's arrow: the whole row, or the page it opens.
    static let arrow = draw(Iconoir.navArrowRight, 20)
    static let check = draw(Iconoir.check, 18)
    /// The empty cover's mark.
    static let disc = draw(Iconoir.musicNote, 22)

    /// Where the sound goes, by `nori_ios_output_look`'s glyph: this iPod's speaker, headphones,
    /// Bluetooth, anything else. Drawn to the seal's height (tools/glyphs.py).
    static func output(_ glyph: Int, _ side: CGFloat) -> UIImage {
        draw([Iconoir.smartphoneDevice, Iconoir.headset, Iconoir.bluetoothTag, Iconoir.airplay][safe: glyph] ?? Iconoir.airplay, side)
    }

    /// Another nori device, by its kind (`DeviceKind`): a phone, a computer, a terminal, a guest.
    static func device(_ kind: Int, _ side: CGFloat) -> UIImage {
        draw([Iconoir.smartphoneDevice, Iconoir.computer, Iconoir.terminal, Iconoir.user][safe: kind] ?? Iconoir.smartphoneDevice, side)
    }

    /// The seasons' kanji, 春 夏 秋 冬, spring first.
    static let seasons = [Iconoir.spring, Iconoir.summer, Iconoir.autumn, Iconoir.winter].map { draw($0, 30) }

    /// The cloud on a provider's item (not in the library yet), `side` points, in the secondary ink: a text
    /// attachment takes no tint.
    static func cloud(_ side: CGFloat) -> UIImage {
        draw(Iconoir.cloudDownload, side).tinted(Theme.secondary)
    }

    /// `parts` (24 x 24 path data) drawn `side` points square, as a template.
    static func draw(_ parts: [GlyphPart], _ side: CGFloat) -> UIImage {
        let k = side / 24
        UIGraphicsBeginImageContextWithOptions(CGSize(width: side, height: side), false, 0)
        defer { UIGraphicsEndImageContext() }
        for part in parts {
            let path = GlyphPath.parse(part.d)
            path.apply(CGAffineTransform(scaleX: k, y: k))
            path.usesEvenOddFillRule = part.evenOdd
            UIColor(white: 0, alpha: part.alpha).set()
            if part.fill { path.fill() }
            if part.stroke {
                path.lineWidth = part.width * k
                path.lineCapStyle = part.round ? .round : .butt
                path.lineJoinStyle = part.round ? .round : .miter
                path.stroke()
            }
        }
        let image = UIGraphicsGetImageFromCurrentImageContext() ?? UIImage()
        return image.withRenderingMode(.alwaysTemplate)
    }
}

extension UIImage {
    /// The template in `color`, for places that take no tint (a text attachment, a slider's thumb).
    func tinted(_ color: UIColor) -> UIImage {
        UIGraphicsBeginImageContextWithOptions(size, false, scale)
        defer { UIGraphicsEndImageContext() }
        color.set()
        withRenderingMode(.alwaysTemplate).draw(in: CGRect(origin: .zero, size: size))
        UIRectFillUsingBlendMode(CGRect(origin: .zero, size: size), .sourceIn)
        return (UIGraphicsGetImageFromCurrentImageContext() ?? self).withRenderingMode(.alwaysOriginal)
    }
}

/// The path data tools/glyphs.py writes: absolute M, L, C and Z, numbers apart by spaces.
private enum GlyphPath {
    static func parse(_ d: String) -> UIBezierPath {
        let p = UIBezierPath()
        var command: Character = "M"
        var n: [CGFloat] = []
        func flush() {
            switch command {
            case "M" where n.count == 2: p.move(to: CGPoint(x: n[0], y: n[1]))
            case "L" where n.count == 2: p.addLine(to: CGPoint(x: n[0], y: n[1]))
            case "C" where n.count == 6:
                p.addCurve(to: CGPoint(x: n[4], y: n[5]), controlPoint1: CGPoint(x: n[0], y: n[1]), controlPoint2: CGPoint(x: n[2], y: n[3]))
            default: return
            }
            n = []
        }
        for token in d.split(separator: " ") {
            var number = Substring(token)
            if let first = token.first, first.isLetter {
                if first == "Z" { p.close(); continue }
                command = first
                number = token.dropFirst()
            }
            if let v = Double(number) {
                n.append(CGFloat(v))
                flush()
            }
        }
        return p
    }
}
