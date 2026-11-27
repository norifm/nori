import UIKit

/// A loading page's placeholders, as the core sketches it (`nori_ios_sketch`): plates in the page's quiet
/// ink where its items will stand, each with its real shape. Unless the system reduces motion, one band of
/// a lighter ink crosses them every pass: a single gradient masked to the plates, moved by Core Animation.
final class SketchView: UIView {
    /// The core's times for it, in seconds: how long a read waits before its sketch shows, a shimmer's
    /// pass, and the fade between the sketch and what was read.
    let after: TimeInterval
    let ease: TimeInterval
    private let pass: TimeInterval
    private let parts: [(p: Int, a: Int)]
    private let look: PageLook
    private let sheen = CAGradientLayer()
    private let plates = CAShapeLayer()
    private let stencil = CAShapeLayer()

    /// The sketch of page `kind` (NORI_PAGE_*), or nil for a page with none.
    init?(kind: Int32, look: PageLook) {
        guard let d = takenJSON(nori_ios_sketch(kind)) as? [String: Any], let parts = d["parts"] as? [[String: Int]] else { return nil }
        let ms = { (k: String) in TimeInterval(d[k] as? Int ?? 0) / 1000 }
        after = ms("after")
        pass = ms("pass")
        ease = ms("ease")
        self.parts = parts.map { ($0["p"] ?? 3, $0["a"] ?? 0) }
        self.look = look
        super.init(frame: .zero)
        isUserInteractionEnabled = false
        clipsToBounds = true
        plates.fillColor = look.veil(0.08).cgColor
        layer.addSublayer(plates)
        let band = look.veil(0.13)
        sheen.colors = [band.withAlphaComponent(0).cgColor, band.cgColor, band.withAlphaComponent(0).cgColor]
        sheen.startPoint = CGPoint(x: 0, y: 0.5)
        sheen.endPoint = CGPoint(x: 1, y: 0.5)
        stencil.fillColor = UIColor.black.cgColor
        sheen.mask = stencil
        if !Motion.plain { layer.addSublayer(sheen) }
    }

    required init?(coder: NSCoder) { fatalError() }

    override func layoutSubviews() {
        super.layoutSubviews()
        let path = shapes(width: bounds.width)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        plates.frame = bounds
        plates.path = path
        let band = bounds.width * 0.6
        sheen.frame = CGRect(x: -band, y: 0, width: band, height: bounds.height)
        stencil.frame = CGRect(x: band, y: 0, width: bounds.width, height: bounds.height)
        stencil.path = path
        CATransaction.commit()
        guard !Motion.plain, sheen.animation(forKey: "pass") == nil, bounds.width > 0 else { return }
        // The band moves across; its stencil (its mask) moves back as far, so the plates it shows stay put.
        let travel = bounds.width + band
        let move = CABasicAnimation(keyPath: "position.x")
        move.byValue = travel
        move.duration = pass
        move.repeatCount = .infinity
        sheen.add(move, forKey: "pass")
        let hold = CABasicAnimation(keyPath: "position.x")
        hold.byValue = -travel
        hold.duration = pass
        hold.repeatCount = .infinity
        stencil.add(hold, forKey: "pass")
    }

    /// The plates' outlines across `width`, top to bottom, cut at the view's foot.
    private func shapes(width w: CGFloat) -> CGPath {
        let path = CGMutablePath()
        let g = Space.gutter
        var y: CGFloat = 8
        let shares: [CGFloat] = [0.62, 0.46, 0.74, 0.54, 0.68, 0.4, 0.58, 0.7]
        func plate(_ r: CGRect, _ radius: CGFloat) { path.addRoundedRect(in: r, cornerWidth: min(radius, r.width / 2), cornerHeight: min(radius, r.height / 2)) }
        func bar(_ x: CGFloat, _ top: CGFloat, _ width: CGFloat, _ height: CGFloat = 9) { plate(CGRect(x: x, y: top, width: max(width, 8), height: height), height / 2) }
        func card(_ x: CGFloat, _ top: CGFloat, _ side: CGFloat, round: Bool, _ i: Int) {
            plate(CGRect(x: x, y: top, width: side, height: side), round ? side / 2 : Radius.cover)
            bar(x, top + side + 11, side * (shares[i % 8] + 0.2))
            bar(x, top + side + 27, side * shares[(i + 3) % 8] * 0.7, 8)
        }
        for (p, a) in parts where y < bounds.height {
            switch p {
            case 0:
                let side = min(w - 2 * g, 260)
                plate(CGRect(x: (w - side) / 2, y: y, width: side, height: side), a == 1 ? side / 2 : Radius.card)
                y += side + 18
                bar(w * 0.2, y, w * 0.6, 16)
                bar(w * 0.32, y + 26, w * 0.36, 11)
                y += 64
            case 1:
                bar(g, y + 10, w * 0.36, 13)
                y += 40
                var x = g
                for i in 0..<6 where x < w {
                    card(x, y, ShelfCell.print, round: a == 1, i)
                    x += ShelfCell.print + 12
                }
                y += ShelfCell.print + 44 + 20
            case 2:
                let side = (w - 2 * g - 12) / 2
                var i = 0
                while y < bounds.height {
                    card(g, y, side, round: a == 1, i)
                    card(g + side + 12, y, side, round: a == 1, i + 1)
                    y += side + 44 + 12
                    i += 2
                }
            default:
                var i = 0
                while y < bounds.height {
                    var x = g
                    switch a {
                    case 0, 1: plate(CGRect(x: x, y: y + 6, width: 44, height: 44), a == 1 ? 22 : Radius.cover); x += 56
                    case 2: bar(x + 4, y + 23, 14, 9); x += 36
                    default: break
                    }
                    let room = w - x - g - 40
                    if a == 3 {
                        bar(x, y + 23, room * shares[i % 8], 10)
                    } else {
                        bar(x, y + 15, room * shares[i % 8], 10)
                        bar(x, y + 32, room * shares[(i + 3) % 8] * 0.6, 8)
                    }
                    y += 56
                    i += 1
                }
            }
        }
        return path
    }
}
