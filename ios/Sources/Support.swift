import UIKit

/// Settings → About → Support nori, as Android's page on a small screen: the stamp book (a seal per payment
/// in the accent of the season it was paid in, five to a row), then what to give in one card (the amounts
/// offered and Other, typed; every month; the currency on a page of its own) and Give, which opens Stripe's
/// checkout page in Safari. While that page waits, a line says so with a QR code to pay from a phone, for a
/// Safari too old for the checkout page. The choice and its words' amounts are the core's
/// (crates/ios/src/support.rs).
final class SupportPage: CardsPage {
    private var page: [String: Any] = [:]
    private var choice: [String: Any] = [:]
    /// The checkout page given, until a seal more than then comes.
    private var given: (url: String, seals: Int)?
    private var seen = -1
    private let book = StampBook()
    /// The thanks over the window while it plays.
    private var thanks: UIView?
    private let amounts = ChipStrip(spacing: 6, fit: true)
    private let other = UITextField()
    private let limit = label(Fonts.font(13, .regular, .footnote), Theme.secondary)
    private lazy var otherRow = row([other, limit])
    private let monthly = UISwitch()
    private lazy var currency = GroupRow(Say.supportCurrency, chevron: true) { [weak self] in self?.pickCurrency() }
    private lazy var give = PillButton(Say.supportChoose, nil)

    override func viewDidLoad() {
        super.viewDidLoad()
        title = Say.supportTitle
        amounts.picked = { [weak self] i in self?.choose(pick: i == (self?.offered.count ?? 0) ? -1 : i) }
        other.keyboardType = .decimalPad
        other.keyboardAppearance = Theme.keyboard
        other.placeholder = "0"
        other.textColor = Theme.label
        other.tintColor = Theme.accent
        other.font = Fonts.font(26, .semibold)
        other.addTarget(self, action: #selector(typed), for: .editingChanged)
        limit.setContentHuggingPriority(.required, for: .horizontal)
        monthly.onTintColor = Theme.switchOn
        monthly.addTarget(self, action: #selector(switched), for: .valueChanged)
        give.addTarget(self, action: #selector(giveTapped), for: .touchUpInside)
        NotificationCenter.default.addObserver(self, selector: #selector(answered(_:)), name: .noriSupport, object: nil)
        // Back from Safari, the payment given there may have gone through.
        NotificationCenter.default.addObserver(self, selector: #selector(refresh), name: UIApplication.didBecomeActiveNotification, object: nil)
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        read()
        refresh()
    }

    @objc private func refresh() { nori_ios_support_refresh() }

    private var offered: [String] { choice["offered"] as? [String] ?? [] }
    private var pick: Int { choice["pick"] as? Int ?? 0 }

    private func read() {
        let zone = TimeZone.current
        let region = Locale.current.regionCode ?? ""
        let p = zone.identifier.withCString { z in
            region.withCString { r in
                takenJSON(nori_ios_support(Int64(Date().timeIntervalSince1970), Int32(zone.secondsFromGMT() / 60), z, Theme.dark ? 1 : 0, r))
            }
        }
        guard let p = p as? [String: Any] else { return }
        page = p
        if choice.isEmpty { choice = p["first"] as? [String: Any] ?? [:] }
        let seals = p["seals"] as? [[String: Any]] ?? []
        if let g = given, seals.count > g.seals { given = nil }
        // A seal more than the page saw is thanked for, the newest when several came at once.
        let fresh = seen >= 0 && seals.count > seen
        book.show(seals, next: p["next"] as? Int ?? 0, thanked: fresh ? seals.count - 1 : book.thanked)
        seen = seals.count
        lay()
        if fresh, let s = seals.last { thank(s) }
    }

    /// The thanks for `seal`, over the whole window: the page under a veil of its paper, the seal pressed down
    /// in the middle as a stamp is, "Thank you" under it; then the seal carried to its empty
    /// place in the book (scrolled into view) and pressed there, so the book's own takes over at rest. A tap
    /// ends it at once. Under Reduce Motion it is the thanks alone, still.
    private func thank(_ seal: [String: Any]) {
        guard let window = view.window, thanks == nil else { return }
        let season = seal["season"] as? Int ?? 0
        let accent = UIColor(argb: UInt32(seal["accent"] as? Int ?? 0))
        let cover = UIView(frame: window.bounds)
        let veil = UIView(frame: cover.bounds)
        veil.backgroundColor = Theme.background.withAlphaComponent(0.92)
        let side: CGFloat = 140
        let home = CGPoint(x: cover.bounds.midX, y: cover.bounds.midY - 34)
        let big = SealView(season: season, accent: accent)
        big.bounds = CGRect(x: 0, y: 0, width: side, height: side)
        big.center = home
        let words = label(Fonts.title(28), Theme.label)
        words.text = Say.supportThanks
        words.sizeToFit()
        // 10 pt under the seal (drawn 2 of 24 inside its box) to the top of the capitals.
        let top = home.y + side / 2 - side / 12 + 10 - (words.font.ascender - words.font.capHeight)
        // A touch left of center: "Th" is heavy, "you" light, and centered it leaned right.
        words.center = CGPoint(x: home.x - 1.5, y: top + words.bounds.height / 2)
        cover.addSubview(veil)
        cover.addSubview(big)
        cover.addSubview(words)
        cover.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(skipThanks)))
        window.addSubview(cover)
        thanks = cover
        if Motion.plain {
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.8) { [weak self] in self?.endThanks() }
            return
        }
        veil.alpha = 0
        big.alpha = 0
        big.transform = CGAffineTransform(translationX: 0, y: -32).scaledBy(x: 1.6, y: 1.6)
        words.alpha = 0
        words.transform = CGAffineTransform(translationX: 0, y: 10)
        UIView.animate(withDuration: 0.32, delay: 0, options: .curveEaseOut, animations: { veil.alpha = 1 })
        let fall = UIViewPropertyAnimator(duration: 0.44, timingParameters: UICubicTimingParameters(controlPoint1: CGPoint(x: 0.55, y: 0), controlPoint2: CGPoint(x: 0.9, y: 0.45)))
        fall.addAnimations { big.alpha = 1; big.transform = .identity }
        fall.addCompletion { [weak self] _ in
            guard let self = self, self.thanks === cover else { return }
            UIView.animateKeyframes(withDuration: 0.22, delay: 0, options: [], animations: {
                UIView.addKeyframe(withRelativeStartTime: 0, relativeDuration: 0.35) { big.transform = CGAffineTransform(scaleX: 1.05, y: 0.95) }
                UIView.addKeyframe(withRelativeStartTime: 0.35, relativeDuration: 0.65) { big.transform = .identity }
            })
            UIView.animate(withDuration: 0.42, delay: 0.14, options: .curveEaseOut, animations: { words.alpha = 1; words.transform = .identity }) { _ in
                self.bringSlot()
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.3) { self.carry(big, words: words, veil: veil, in: cover) }
            }
        }
        fall.startAnimation(afterDelay: 0.12)
    }

    /// The thanked seal's empty place scrolled into the page's view.
    private func bringSlot() {
        guard let i = book.thanked else { return }
        var v = book.superview
        while let s = v, !(s is UIScrollView) { v = s.superview }
        guard let scroll = v as? UIScrollView else { return }
        scroll.scrollRectToVisible(book.convert(book.slot(i), to: scroll).insetBy(dx: 0, dy: -40), animated: true)
    }

    /// The seal carried from the middle to its empty place: lifted a little and turned to its tilt, set down,
    /// pressed and settled, while the thanks and the veil go.
    private func carry(_ big: SealView, words: UIView, veil: UIView, in cover: UIView) {
        guard thanks === cover, let i = book.thanked else { return }
        let to = book.convert(book.slot(i), to: cover)
        let (dx, dy) = (to.midX - big.center.x, to.midY - big.center.y)
        let k = to.width / big.bounds.width
        let tilt = CGFloat((page["seals"] as? [[String: Any]])?[safe: i]?["tilt"] as? Double ?? 0) * .pi / 180
        let pose = { (x: CGFloat, y: CGFloat, turn: CGFloat, sx: CGFloat, sy: CGFloat) in
            CGAffineTransform(translationX: x, y: y).rotated(by: turn).scaledBy(x: sx, y: sy)
        }
        UIView.animate(withDuration: 0.22, delay: 0, options: .curveEaseIn, animations: { words.alpha = 0 })
        UIView.animate(withDuration: 0.64, delay: 0.18, options: .curveEaseInOut, animations: { veil.alpha = 0 })
        UIView.animateKeyframes(withDuration: 0.98, delay: 0, options: [.calculationModeCubic], animations: {
            UIView.addKeyframe(withRelativeStartTime: 0, relativeDuration: 0.42) { big.transform = pose(dx * 0.35, dy * 0.35 - 36, tilt / 2, (1 + k) / 2 + 0.06, (1 + k) / 2 + 0.06) }
            UIView.addKeyframe(withRelativeStartTime: 0.42, relativeDuration: 0.38) { big.transform = pose(dx, dy, tilt, k, k) }
            UIView.addKeyframe(withRelativeStartTime: 0.8, relativeDuration: 0.09) { big.transform = pose(dx, dy, tilt, k * 1.06, k * 0.93) }
            UIView.addKeyframe(withRelativeStartTime: 0.89, relativeDuration: 0.11) { big.transform = pose(dx, dy, tilt, k, k) }
        }) { [weak self] _ in self?.endThanks(carried: true) }
    }

    @objc private func skipThanks() { endThanks() }

    /// The thanks over: the seal in its place, the next empty place growing in beside it if it was carried.
    private func endThanks(carried: Bool = false) {
        guard let cover = thanks else { return }
        thanks = nil
        cover.removeFromSuperview()
        book.landed(growing: carried && !Motion.plain)
    }

    private func choose(currency c: String? = nil, monthly m: Bool? = nil, pick p: Int? = nil, other o: String? = nil) {
        let code = c ?? choice["currency"] as? String ?? ""
        let text = o ?? choice["other"] as? String ?? ""
        let next = code.withCString { cc in
            text.withCString { t in
                takenJSON(nori_ios_support_choice(cc, (m ?? (choice["monthly"] as? Bool ?? false)) ? 1 : 0, Int32(p ?? pick), t))
            }
        }
        choice = next as? [String: Any] ?? choice
        lay()
        if pick == -1, p == -1 { other.becomeFirstResponder() }
        if pick != -1 { other.resignFirstResponder() }
    }

    @objc private func typed() { choose(other: other.text ?? "") }
    @objc private func switched() { choose(monthly: monthly.isOn) }

    private func pickCurrency() {
        other.resignFirstResponder()
        let all = page["currencies"] as? [[String: String]] ?? []
        let now = choice["currency"] as? String
        navigationController?.pushViewController(CurrencyPage(all, chosen: now) { [weak self] code in self?.choose(currency: code) }, animated: true)
    }

    @objc private func giveTapped() {
        guard choice["amount"] is String else { return }
        other.resignFirstResponder()
        let zone = TimeZone.current
        let code = choice["currency"] as? String ?? ""
        let text = choice["other"] as? String ?? ""
        code.withCString { c in
            text.withCString { t in
                zone.identifier.withCString { z in
                    nori_ios_support_give(c, (choice["monthly"] as? Bool ?? false) ? 1 : 0, Int32(pick), t, Int64(Date().timeIntervalSince1970), Int32(zone.secondsFromGMT() / 60), z)
                }
            }
        }
    }

    @objc private func answered(_ n: Notification) {
        guard let r = n.object as? SupportAnswer else { return }
        switch r.flag {
        case NORI_SUPPORT_GIVEN:
            given = (r.text, (page["seals"] as? [Any])?.count ?? 0)
            URL(string: r.text).map { UIApplication.shared.open($0) }
            lay()
        case NORI_SUPPORT_PAGE:
            URL(string: r.text).map { UIApplication.shared.open($0) }
        case NORI_SUPPORT_REFRESHED:
            read()
        default:
            Toast.show(r.otherCurrency ? Say.supportOtherCurrency : Say.supportFailed)
        }
    }

    private func lay() {
        let names = offered + [Say.supportOther]
        amounts.set(names, chosen: pick == -1 ? names.count - 1 : pick)
        let typing = pick == -1
        otherRow.isHidden = !typing
        if other.text != choice["other"] as? String { other.text = choice["other"] as? String }
        let least = choice["least"] as? String ?? "", most = choice["most"] as? String ?? ""
        limit.text = !least.isEmpty ? Say.supportLeast(least) : !most.isEmpty ? Say.supportMost(most) : nil
        monthly.isOn = choice["monthly"] as? Bool ?? false
        let code = choice["currency"] as? String
        currency.value.text = (page["currencies"] as? [[String: String]])?.first { $0["code"] == code }?["name"]
        currency.value.isHidden = false
        let amount = choice["amount"] as? String
        give.set(amount.map { Say.supportGive($0, monthly: monthly.isOn) } ?? Say.supportChoose, nil)
        give.alpha = amount == nil ? 0.38 : 1
        var views: [UIView] = [indent(words(Say.supportLine, Fonts.font(14), Theme.secondary)), indent(book)]
        let amountRow = UIStackView(arrangedSubviews: [words(Say.supportAmount, Fonts.font(16), Theme.label), amounts])
        amountRow.axis = .vertical
        amountRow.spacing = 10
        views.append(GroupCard([row([amountRow]), otherRow, GroupRow(Say.supportEveryMonth, trailing: monthly), currency]))
        views.append(indent(give))
        if page["running"] as? Bool == true {
            views.append(GroupCard([
                GroupRow(Say.supportManage, chevron: true) { nori_ios_support_page(0) },
                GroupRow(Say.supportStop, chevron: true) { nori_ios_support_page(1) },
            ]))
        }
        if let g = given {
            views.append(indent(words(Say.supportInSafari, Fonts.font(13, .regular, .footnote), Theme.secondary, lines: 0)))
            if let code = (g.url.withCString { takenJSON(nori_ios_qr($0)) }) as? [String: Any] { views.append(indent(QRCode(code))) }
        }
        set(views)
        column.setCustomSpacing(Space.section, after: views[1])
        column.setCustomSpacing(Space.row, after: views[2])
    }

    /// `views` in a row of the card, inset as its rows are.
    private func row(_ views: [UIView]) -> UIView {
        let s = UIStackView(arrangedSubviews: views)
        s.spacing = 12
        s.alignment = .center
        s.isLayoutMarginsRelativeArrangement = true
        s.layoutMargins = UIEdgeInsets(top: 12, left: Space.groupInset, bottom: 12, right: Space.groupInset)
        return s
    }

    /// `v` on the page's gutter.
    private func indent(_ v: UIView) -> UIView {
        let s = UIStackView(arrangedSubviews: [v])
        s.isLayoutMarginsRelativeArrangement = true
        s.layoutMargins = UIEdgeInsets(top: 0, left: Space.gutter, bottom: 0, right: Space.gutter)
        return s
    }
}

/// A `NORI_REPORT_SUPPORT` as the Support page reads it.
final class SupportAnswer {
    let flag: Int32
    let text: String
    let otherCurrency: Bool
    init(flag: Int32, text: String, otherCurrency: Bool) {
        self.flag = flag
        self.text = text
        self.otherCurrency = otherCurrency
    }
}

extension Notification.Name {
    /// The support service answered: the object is a `SupportAnswer`.
    static let noriSupport = Notification.Name("noriSupport")
}

/// The currencies support is offered in, the one chosen checked; a tap picks one and goes back.
final class CurrencyPage: CardsPage {
    private let all: [[String: String]]
    private let chosen: String?
    private let picked: (String) -> Void

    init(_ all: [[String: String]], chosen: String?, picked: @escaping (String) -> Void) {
        self.all = all
        self.chosen = chosen
        self.picked = picked
        super.init(nibName: nil, bundle: nil)
        title = Say.supportCurrency
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        set([GroupCard(all.map { c in
            GroupRow(c["name"] ?? "", value: c["code"] == chosen ? "✓" : c["code"]) { [weak self] in
                self?.picked(c["code"] ?? "")
                self?.navigationController?.popViewController(animated: true)
            }
        })])
    }
}

/// The stamp book: the seals five to a row across the page's width, then the next one's place, its frame and
/// season's kanji quiet. A seal is the chop in its paid season's accent with the kanji cut out of it.
final class StampBook: UIView {
    private var seals: [[String: Any]] = []
    private var coming = 0
    /// The seal being thanked for: it keeps its empty place until it is carried in, and the next one waits.
    private(set) var thanked: Int?
    /// The next place growing in, drawn by its own view until it is whole.
    private var growing: SealView?
    private let gap: CGFloat = 12
    private var height: NSLayoutConstraint?

    init() {
        super.init(frame: .zero)
        backgroundColor = .clear
        contentMode = .redraw
    }

    required init?(coder: NSCoder) { fatalError() }

    func show(_ seals: [[String: Any]], next: Int, thanked: Int?) {
        self.seals = seals
        coming = next
        self.thanked = thanked
        setNeedsLayout()
        setNeedsDisplay()
    }

    /// The thanked seal is in its place; `growing`, the next place grows in beside it.
    func landed(growing grows: Bool) {
        thanked = nil
        growing?.removeFromSuperview()
        growing = nil
        if grows {
            let v = SealView(blank: coming)
            v.frame = slot(seals.count)
            v.alpha = 0
            v.transform = CGAffineTransform(scaleX: 0.6, y: 0.6)
            addSubview(v)
            growing = v
            UIView.animate(withDuration: 0.32, delay: 0.06, usingSpringWithDamping: 0.7, initialSpringVelocity: 0, options: [], animations: { v.alpha = 1; v.transform = .identity }) { [weak self] _ in
                guard self?.growing === v else { return }
                v.removeFromSuperview()
                self?.growing = nil
                self?.setNeedsDisplay()
            }
        }
        setNeedsDisplay()
    }

    private var side: CGFloat { max(0, (bounds.width - 4 * gap) / 5) }

    /// Where the `i`th seal's place is.
    func slot(_ i: Int) -> CGRect {
        CGRect(x: CGFloat(i % 5) * (side + gap), y: CGFloat(i / 5) * (side + gap), width: side, height: side)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let rows = CGFloat((seals.count + 1 + 4) / 5)
        let h = rows * side + max(0, rows - 1) * gap
        if height == nil {
            height = heightAnchor.constraint(equalToConstant: h)
            height?.isActive = true
        } else if height?.constant != h {
            height?.constant = h
        }
    }

    override func draw(_ rect: CGRect) {
        guard let ctx = UIGraphicsGetCurrentContext(), side > 0 else { return }
        for (i, s) in seals.enumerated() {
            if i == thanked {
                StampBook.blank(s["season"] as? Int ?? 0, in: slot(i))
            } else {
                StampBook.seal(s["season"] as? Int ?? 0, UIColor(argb: UInt32(s["accent"] as? Int ?? 0)), tilt: CGFloat(s["tilt"] as? Double ?? 0), in: slot(i), ctx)
            }
        }
        if thanked == nil, growing == nil { StampBook.blank(coming, in: slot(seals.count)) }
    }

    /// A seal of `season` in `r`: the chop in `accent` turned `tilt` degrees, the kanji cut out of it.
    static func seal(_ season: Int, _ accent: UIColor, tilt: CGFloat, in r: CGRect, _ ctx: CGContext) {
        let k = r.width / 24
        ctx.saveGState()
        ctx.translateBy(x: r.midX, y: r.midY)
        ctx.rotate(by: tilt * .pi / 180)
        ctx.translateBy(x: -r.width / 2, y: -r.height / 2)
        let body = UIBezierPath(roundedRect: CGRect(x: 2 * k, y: 2 * k, width: 20 * k, height: 20 * k), cornerRadius: 3.5 * k)
        body.append(UIBezierPath(roundedRect: CGRect(x: 3.75 * k, y: 3.75 * k, width: 16.5 * k, height: 16.5 * k), cornerRadius: 2.1 * k))
        body.append(UIBezierPath(roundedRect: CGRect(x: 4.35 * k, y: 4.35 * k, width: 15.3 * k, height: 15.3 * k), cornerRadius: 1.6 * k))
        body.usesEvenOddFillRule = true
        accent.setFill()
        body.fill()
        kanji(season, in: CGRect(x: 0, y: 0, width: r.width, height: r.height), scale: 0.62, color: Theme.background)
        ctx.restoreGState()
    }

    /// A seal's place before it is paid for in `r`: its frame and `season`'s kanji, quiet.
    static func blank(_ season: Int, in r: CGRect) {
        let k = r.width / 24
        let frame = UIBezierPath(roundedRect: CGRect(x: r.minX + 3 * k, y: r.minY + 3 * k, width: 18 * k, height: 18 * k), cornerRadius: 3.5 * k)
        frame.lineWidth = 1.5 * k
        Theme.secondary.setStroke()
        frame.stroke()
        kanji(season, in: r, scale: 0.56, color: Theme.secondary.withAlphaComponent(0.85))
    }

    /// Season `season`'s kanji filled in `color`, `scale` of `r` about its middle.
    private static func kanji(_ season: Int, in r: CGRect, scale: CGFloat, color: UIColor) {
        let parts = [Iconoir.spring, Iconoir.summer, Iconoir.autumn, Iconoir.winter][max(0, min(3, season))]
        let k = r.width / 24 * scale
        let shift = CGAffineTransform(translationX: r.midX - 12 * k, y: r.midY - 12 * k).scaledBy(x: k, y: k)
        color.setFill()
        for part in parts {
            let p = GlyphPath.parse(part.d)
            p.apply(shift)
            p.fill()
        }
    }
}

/// One seal as its own view, upright (the thanks turns it), or (`blank`) a place before it is paid for.
final class SealView: UIView {
    private let season: Int
    private let accent: UIColor?

    init(season: Int, accent: UIColor) {
        self.season = season
        self.accent = accent
        super.init(frame: .zero)
        backgroundColor = .clear
        contentMode = .redraw
    }

    init(blank season: Int) {
        self.season = season
        accent = nil
        super.init(frame: .zero)
        backgroundColor = .clear
        contentMode = .redraw
    }

    required init?(coder: NSCoder) { fatalError() }

    override func draw(_ rect: CGRect) {
        guard let ctx = UIGraphicsGetCurrentContext() else { return }
        if let a = accent { StampBook.seal(season, a, tilt: 0, in: bounds, ctx) } else { StampBook.blank(season, in: bounds) }
    }
}

/// A QR code, black on white whatever the paper, in a light quiet zone so a camera finds its edges.
final class QRCode: UIView {
    private let size: Int
    private let dark: [Bool]

    init(_ code: [String: Any]) {
        size = code["size"] as? Int ?? 0
        dark = (code["dark"] as? String ?? "").map { $0 == "1" }
        super.init(frame: .zero)
        backgroundColor = .white
        layer.cornerRadius = 8
        translatesAutoresizingMaskIntoConstraints = false
        widthAnchor.constraint(equalToConstant: 132).isActive = true
        heightAnchor.constraint(equalToConstant: 132).isActive = true
    }

    required init?(coder: NSCoder) { fatalError() }

    override func draw(_ rect: CGRect) {
        guard size > 0 else { return }
        let quiet: CGFloat = 8
        let m = (bounds.width - 2 * quiet) / CGFloat(size)
        UIColor.black.setFill()
        for (i, d) in dark.enumerated() where d {
            UIRectFill(CGRect(x: quiet + CGFloat(i % size) * m, y: quiet + CGFloat(i / size) * m, width: m, height: m))
        }
    }
}

/// `text` in a label of `font` and `color`.
private func words(_ text: String, _ font: UIFont, _ color: UIColor, lines: Int = 1) -> UILabel {
    let l = label(font, color, lines: lines)
    l.text = text
    return l
}
