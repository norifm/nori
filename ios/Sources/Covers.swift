import UIKit

/// Decoded covers by id and pixel size; the system empties it under memory pressure.
enum CoverCache {
    static let images: NSCache<NSString, UIImage> = {
        let c = NSCache<NSString, UIImage>()
        c.totalCostLimit = 16 * 1024 * 1024
        return c
    }()

    static func key(_ id: String, _ px: Int) -> NSString { "\(id)@\(px)" as NSString }

    static func put(_ image: UIImage, _ id: String, _ px: Int) {
        let cost = Int(image.size.width * image.scale * image.size.height * image.scale * 4)
        images.setObject(image, forKey: key(id, px), cost: cost)
    }

    static func trim() {
        images.removeAllObjects()
    }
}

/// A square cover on a soft two-tone plate. The picture the core picks to stand in (the cover kept at
/// another size or rendition) shows while its own comes, at once when it is in memory, and the sharp one
/// replaces it; with none, a sheen crosses the plate and the picture fades in over
/// 260 ms, the sheen going away underneath it. One that never comes settles into the disc mark. Both are
/// layer animations the render server runs, and a picture cached at its own size shows at once with
/// neither. Without `plate` it draws nothing of its own: the picture fades in over what is behind it.
final class CoverView: UIView {
    private let picture = UIImageView()
    private let mark = UIImageView(image: Glyph.disc)
    private let sheen = CAGradientLayer()
    private let plateLayer = CAGradientLayer()
    private let plated: Bool
    private var token: UInt64?
    private var shown: String?

    init(plate: Bool = true) {
        plated = plate
        super.init(frame: .zero)
        if plate {
            plateLayer.colors = [Theme.page.veil(0.13).cgColor, Theme.page.veil(0.06).cgColor]
            plateLayer.startPoint = .zero
            plateLayer.endPoint = CGPoint(x: 1, y: 1)
            layer.addSublayer(plateLayer)
        }
        layer.cornerRadius = 6
        clipsToBounds = true
        mark.tintColor = Theme.secondary
        mark.contentMode = .center
        mark.isHidden = true
        picture.contentMode = .scaleAspectFill
        sheen.startPoint = CGPoint(x: 0, y: 0.5)
        sheen.endPoint = CGPoint(x: 1, y: 0.5)
        let light = Theme.label.withAlphaComponent(0.08).cgColor, clear = Theme.label.withAlphaComponent(0).cgColor
        sheen.colors = [clear, light, clear]
        sheen.locations = [0, 0.15, 0.3]
        sheen.isHidden = true
        layer.addSublayer(sheen)
        for v in [mark, picture] {
            // The plate's size is set from outside: the picture or the mark arriving never moves it.
            for axis in [NSLayoutConstraint.Axis.horizontal, .vertical] {
                v.setContentHuggingPriority(UILayoutPriority(1), for: axis)
                v.setContentCompressionResistancePriority(UILayoutPriority(1), for: axis)
            }
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
            NSLayoutConstraint.activate([
                v.leadingAnchor.constraint(equalTo: leadingAnchor),
                v.trailingAnchor.constraint(equalTo: trailingAnchor),
                v.topAnchor.constraint(equalTo: topAnchor),
                v.bottomAnchor.constraint(equalTo: bottomAnchor),
            ])
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    override func layoutSubviews() {
        super.layoutSubviews()
        sheen.frame = bounds
        plateLayer.frame = bounds
    }

    private func waiting(_ on: Bool) {
        sheen.removeAllAnimations()
        sheen.isHidden = !on
        guard on, !UIAccessibility.isReduceMotionEnabled else { return }
        let sweep = CABasicAnimation(keyPath: "locations")
        sweep.fromValue = [-0.3, -0.15, 0]
        sweep.toValue = [1, 1.15, 1.3]
        sweep.duration = 1.2
        // A cover that never comes (failed, or none) does not keep the plate moving.
        sweep.repeatCount = 3
        sheen.add(sweep, forKey: "sweep")
    }

    /// Shows cover `id` at `points` wide; a cell reused for another row lets go of the old request.
    func show(_ id: String, points: CGFloat) {
        let px = Int(points * UIScreen.main.scale)
        let key = "\(id)@\(px)"
        if shown == key { return }
        cancel()
        shown = key
        picture.image = nil
        mark.alpha = 1
        mark.isHidden = !(id.isEmpty && plated)
        waiting(false)
        guard !id.isEmpty else { return }
        if let cached = CoverCache.images.object(forKey: key as NSString) {
            picture.image = cached
            return
        }
        // A picture in memory comes while the request is made: drawn at once, without a fade.
        var asking = true, standing = false, came = false
        let t = Core.shared.cover(id, px: px) { [weak self] image, standIn in
            if !standIn, let image { CoverCache.put(image, id, px) }
            guard let self, self.shown == key else { return }
            let still = asking || UIAccessibility.isReduceMotionEnabled
            if standIn {
                guard let image, !standing else { return }
                standing = true
                self.waiting(false)
                UIView.transition(with: self.picture, duration: still ? 0 : 0.26, options: .transitionCrossDissolve, animations: { self.picture.image = image })
                return
            }
            came = true
            self.token = nil
            guard let image else {
                self.waiting(false)
                self.shown = nil
                self.settleOnMark()
                return
            }
            // Sharpening a stand-in is a short cross-fade; a first picture fades in.
            let fade = still ? 0 : standing ? 0.15 : 0.26
            UIView.transition(with: self.picture, duration: fade, options: .transitionCrossDissolve, animations: { self.picture.image = image }, completion: { [weak self] _ in
                if self?.shown == key { self?.waiting(false) }
            })
        }
        asking = false
        if !came {
            token = t
            if !standing { waiting(plated) }
        }
    }

    /// No picture came: the disc mark, faded in.
    private func settleOnMark() {
        guard plated else { return }
        mark.alpha = 0
        mark.isHidden = false
        UIView.animate(withDuration: UIAccessibility.isReduceMotionEnabled ? 0 : 0.26) { self.mark.alpha = 1 }
    }

    func cancel() {
        if let t = token {
            Core.shared.cancelCover(t)
            token = nil
        }
        waiting(false)
        shown = nil
    }
}
