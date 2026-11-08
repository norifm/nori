import CoreText
import UIKit

/// Nori Seasons' shapes for UIKit, as Android's Design.kt has them: radii, spacing, type and the handful
/// of pieces every screen is built from. Everything is static: a page that sits on screen costs nothing
/// between frames.
enum Radius {
    static let cover: CGFloat = 8
    static let card: CGFloat = 12
    static let tile: CGFloat = 18
    static let sheet: CGFloat = 26
}

enum Space {
    /// The side margin every screen shares.
    static let gutter: CGFloat = 16
    static let tight: CGFloat = 8
    static let row: CGFloat = 12
    static let section: CGFloat = 26
    /// How far a page's title rises into the artwork's melt.
    static let titleRise: CGFloat = 34
    /// A grouped card's margin from the page's sides, and its rows' inset.
    static let groupMargin: CGFloat = 12
    static let groupInset: CGFloat = 14
}

/// A hairline: the edge of a raised surface, one pixel.
let hairlineWidth: CGFloat = 1 / UIScreen.main.scale

/// Inter for everything, Literata for what a page is called: both variable fonts in the bundle
/// (`UIAppFonts`), each weight a real instance of the font. Dynamic Type scales them.
enum Fonts {
    private static var made: [String: UIFont] = [:]

    /// Inter at `size` points and `weight`, scaled as `style` is.
    static func font(_ size: CGFloat, _ weight: UIFont.Weight = .regular, _ style: UIFont.TextStyle = .body) -> UIFont {
        UIFontMetrics(forTextStyle: style).scaledFont(for: variable("InterVariable", size, weight))
    }

    /// Inter with figures of one width, for times and counts that change in place.
    static func digits(_ size: CGFloat, _ weight: UIFont.Weight = .regular) -> UIFont {
        let base = variable("InterVariable", size, weight)
        let tabular = base.fontDescriptor.addingAttributes([.featureSettings: [[
            UIFontDescriptor.FeatureKey.featureIdentifier: kNumberSpacingType,
            UIFontDescriptor.FeatureKey.typeIdentifier: kMonospacedNumbersSelector,
        ]]])
        return UIFont(descriptor: tabular, size: size)
    }

    /// Literata, for a page's name: semibold, or bold at display sizes.
    static func title(_ size: CGFloat, _ style: UIFont.TextStyle = .title1) -> UIFont {
        UIFontMetrics(forTextStyle: style).scaledFont(for: variable("Literata-Regular", size, size >= 28 ? .bold : .semibold))
    }

    /// `name` at `weight` on its `wght` axis; the system's face if the bundle has no such font.
    private static func variable(_ name: String, _ size: CGFloat, _ weight: UIFont.Weight) -> UIFont {
        let key = "\(name) \(size) \(weight.rawValue)"
        if let f = made[key] { return f }
        let wght = [UIFont.Weight.regular: 400, .medium: 500, .semibold: 600, .bold: 700][weight] ?? 400
        let axis = UIFontDescriptor.AttributeName(rawValue: kCTFontVariationAttribute as String)
        let d = UIFontDescriptor(fontAttributes: [.name: name, axis: [0x7767_6874: wght]])
        let f = UIFont(name: name, size: size) == nil ? UIFont.systemFont(ofSize: size, weight: weight) : UIFont(descriptor: d, size: size)
        made[key] = f
        return f
    }
}

func hairlineView(_ color: UIColor = Theme.hairline) -> UIView {
    let v = UIView()
    v.backgroundColor = color
    v.translatesAutoresizingMaskIntoConstraints = false
    v.heightAnchor.constraint(equalToConstant: hairlineWidth).isActive = true
    return v
}

/// A label in one of the app's styles.
func label(_ font: UIFont, _ color: UIColor, lines: Int = 1) -> UILabel {
    let l = UILabel()
    l.font = font
    l.textColor = color
    l.numberOfLines = lines
    l.adjustsFontForContentSizeCategory = true
    return l
}

/// The small capitals over a group card: "PLAYBACK".
final class GroupLabel: UIView {
    let text = label(Fonts.font(11.5, .semibold, .caption1), Theme.secondary)

    init(_ words: String) {
        super.init(frame: .zero)
        set(words)
        text.translatesAutoresizingMaskIntoConstraints = false
        addSubview(text)
        NSLayoutConstraint.activate([
            text.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Space.groupMargin + Space.groupInset + 2),
            text.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -Space.groupMargin - Space.groupInset - 2),
            text.topAnchor.constraint(equalTo: topAnchor, constant: 22),
            text.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func set(_ words: String) {
        text.attributedText = NSAttributedString(string: words.uppercased(), attributes: [.kern: 0.7])
    }
}

/// The name of a whole screen, as Android's LargeTitle: big, on the gutter, what acts on the page at its end.
final class PageTitle: UIView {
    init(_ words: String, trailing: [UIView] = []) {
        super.init(frame: .zero)
        let name = label(Fonts.title(31), Theme.label)
        name.text = words
        name.setContentHuggingPriority(.defaultLow, for: .horizontal)
        let row = UIStackView(arrangedSubviews: [name] + trailing)
        row.alignment = .center
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Space.gutter),
            row.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -Space.tight),
            row.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            row.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -2),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }
}

/// Rows grouped on one raised plate: the surface, a hairline edge, the tile's corners, inset from the
/// page's sides. Its rows are `GroupRow`s, the last without a divider.
final class GroupCard: UIView {
    let rows = UIStackView()

    init(_ rows: [UIView] = [], margin: CGFloat = Space.groupMargin) {
        super.init(frame: .zero)
        let plate = UIView()
        plate.backgroundColor = Theme.row
        plate.layer.cornerRadius = Radius.tile
        plate.layer.borderWidth = hairlineWidth
        plate.layer.borderColor = Theme.hairline.cgColor
        plate.clipsToBounds = true
        self.rows.axis = .vertical
        for v in [plate, self.rows] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
        }
        addSubview(plate)
        plate.addSubview(self.rows)
        NSLayoutConstraint.activate([
            plate.leadingAnchor.constraint(equalTo: leadingAnchor, constant: margin),
            plate.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -margin),
            plate.topAnchor.constraint(equalTo: topAnchor),
            plate.bottomAnchor.constraint(equalTo: bottomAnchor),
            self.rows.leadingAnchor.constraint(equalTo: plate.leadingAnchor),
            self.rows.trailingAnchor.constraint(equalTo: plate.trailingAnchor),
            self.rows.topAnchor.constraint(equalTo: plate.topAnchor),
            self.rows.bottomAnchor.constraint(equalTo: plate.bottomAnchor),
        ])
        set(rows)
    }

    required init?(coder: NSCoder) { fatalError() }

    /// The card's rows, each but the last with its divider.
    func set(_ all: [UIView]) {
        rows.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for (i, r) in all.enumerated() {
            (r as? GroupRow)?.divider.isHidden = i == all.count - 1
            rows.addArrangedSubview(r)
        }
    }
}

/// A row of a `GroupCard`: an optional icon in ink, a title and maybe a quiet line under it, a quiet value,
/// then a trailing view (a switch) or a chevron. Its divider starts where the text does.
final class GroupRow: UIControl {
    let title = label(Fonts.font(16), Theme.label)
    let subtitle = label(Fonts.font(13, .regular, .footnote), Theme.secondary, lines: 0)
    let value = label(Fonts.font(13.5, .regular, .footnote), Theme.secondary)
    let icon = UIImageView()
    let divider = hairlineView()
    private let chevron = UIImageView(image: Glyph.arrow)
    var tapped: (() -> Void)?

    init(_ text: String, subtitle line: String? = nil, value v: String? = nil, icon glyph: UIImage? = nil,
         chevron: Bool = false, trailing: UIView? = nil, minHeight: CGFloat = 52, tapped: (() -> Void)? = nil) {
        let tapped = tapped ?? (trailing as? UISwitch).map(GroupRow.flip)
        self.tapped = tapped
        super.init(frame: .zero)
        title.text = text
        subtitle.text = line
        subtitle.isHidden = (line ?? "").isEmpty
        value.text = v
        value.isHidden = (v ?? "").isEmpty
        // Only the words take the row's spare width.
        for v in ([value, self.chevron, trailing] as [UIView?]).compactMap({ $0 }) {
            v.setContentHuggingPriority(.required, for: .horizontal)
            v.setContentCompressionResistancePriority(.required, for: .horizontal)
        }
        icon.image = glyph
        icon.tintColor = Theme.label
        icon.isHidden = glyph == nil
        self.chevron.tintColor = Theme.secondary
        self.chevron.isHidden = !chevron
        let words = UIStackView(arrangedSubviews: [title, subtitle])
        words.axis = .vertical
        words.spacing = 1
        let strip = UIStackView(arrangedSubviews: [icon, words, value] + (trailing.map { [$0] } ?? []) + [self.chevron])
        strip.alignment = .center
        strip.spacing = Space.groupInset
        strip.setCustomSpacing(4, after: value)
        for v in [strip, divider] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        NSLayoutConstraint.activate([
            icon.widthAnchor.constraint(equalToConstant: 20),
            strip.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Space.groupInset),
            strip.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -Space.groupInset),
            strip.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            strip.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
            heightAnchor.constraint(greaterThanOrEqualToConstant: minHeight),
            divider.leadingAnchor.constraint(equalTo: words.leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: trailingAnchor),
            divider.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
        if tapped != nil { addTarget(self, action: #selector(fire), for: .touchUpInside) }
        isAccessibilityElement = trailing == nil
        accessibilityLabel = [text, line ?? "", v ?? ""].filter { !$0.isEmpty }.joined(separator: ", ")
        accessibilityTraits = tapped != nil ? .button : .staticText
    }

    required init?(coder: NSCoder) { fatalError() }

    /// A switch's row turns it, as the switch itself does.
    private static func flip(_ toggle: UISwitch) -> () -> Void {
        {
            toggle.setOn(!toggle.isOn, animated: true)
            toggle.sendActions(for: .valueChanged)
        }
    }

    /// A tap anywhere on the row is the row's, but for a control at its end (a switch, whose touches land
    /// on its own subviews).
    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        guard let hit = super.hitTest(point, with: event) else { return nil }
        var view: UIView? = hit
        while let v = view, v !== self {
            if v is UIControl { return hit }
            view = v.superview
        }
        return self
    }

    override var isHighlighted: Bool {
        didSet { backgroundColor = isHighlighted && tapped != nil ? Theme.page.veil(0.06).withAlphaComponent(0.6) : .clear }
    }

    override var isEnabled: Bool {
        didSet { alpha = isEnabled ? 1 : 0.38 }
    }

    @objc private func fire() { tapped?() }
}

/// A section's title on the gutter; with `more`, the whole title opens the section's full view, and a quiet
/// arrow ends it.
final class SectionTitle: UIControl {
    let text = label(Fonts.font(19, .semibold, .title3), Theme.label)
    private let arrow = UIImageView(image: Glyph.arrow)
    var more: (() -> Void)? { didSet { arrow.isHidden = more == nil; accessibilityTraits = more == nil ? .header : [.header, .button] } }

    init(_ words: String = "", compact: Bool = false) {
        super.init(frame: .zero)
        text.text = words
        arrow.tintColor = Theme.secondary
        arrow.isHidden = true
        arrow.setContentHuggingPriority(.required, for: .horizontal)
        for v in [text, arrow] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        NSLayoutConstraint.activate([
            text.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Space.gutter),
            text.topAnchor.constraint(equalTo: topAnchor, constant: compact ? 12 : Space.section),
            text.bottomAnchor.constraint(equalTo: bottomAnchor, constant: compact ? -2 : -12),
            arrow.leadingAnchor.constraint(greaterThanOrEqualTo: text.trailingAnchor, constant: 4),
            arrow.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -Space.gutter),
            arrow.centerYAnchor.constraint(equalTo: text.centerYAnchor),
        ])
        addTarget(self, action: #selector(fire), for: .touchUpInside)
        isAccessibilityElement = true
        accessibilityTraits = .header
    }

    required init?(coder: NSCoder) { fatalError() }

    override var accessibilityLabel: String? { get { text.text } set {} }

    @objc private func fire() { more?() }
}

/// A choice chip: the chosen one raised on the surface with a hairline edge and in ink, the others quiet
/// words.
final class Chip: UIButton {
    var chosen = false { didSet { paint() } }

    init(_ words: String, chosen on: Bool = false, inset: CGFloat = 14) {
        super.init(frame: .zero)
        setTitle(words, for: .normal)
        titleLabel?.font = Fonts.font(14, .medium, .subheadline)
        titleLabel?.adjustsFontSizeToFitWidth = true
        titleLabel?.minimumScaleFactor = 0.8
        contentEdgeInsets = UIEdgeInsets(top: 0, left: inset, bottom: 0, right: inset)
        layer.cornerRadius = 17
        translatesAutoresizingMaskIntoConstraints = false
        heightAnchor.constraint(equalToConstant: 34).isActive = true
        chosen = on
        paint()
    }

    required init?(coder: NSCoder) { fatalError() }

    private func paint() {
        backgroundColor = chosen ? Theme.row : .clear
        layer.borderWidth = chosen ? hairlineWidth : 0
        layer.borderColor = Theme.hairline.cgColor
        setTitleColor(chosen ? Theme.label : Theme.secondary, for: .normal)
        accessibilityTraits = chosen ? [.button, .selected] : .button
    }
}

/// Chips in a row that scrolls sideways, from the gutter, or with `fit` shares its own width equally; one is
/// chosen, a tap chooses another.
final class ChipStrip: UIScrollView {
    private let row = UIStackView()
    private let fit: Bool
    private(set) var chips: [Chip] = []
    var picked: ((Int) -> Void)?

    init(spacing: CGFloat = 8, fit: Bool = false) {
        self.fit = fit
        super.init(frame: .zero)
        showsHorizontalScrollIndicator = false
        row.spacing = spacing
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        let side = fit ? 0 : Space.gutter
        if fit {
            isScrollEnabled = false
            row.distribution = .fillEqually
            row.widthAnchor.constraint(equalTo: frameLayoutGuide.widthAnchor).isActive = true
        }
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: contentLayoutGuide.leadingAnchor, constant: side),
            row.trailingAnchor.constraint(equalTo: contentLayoutGuide.trailingAnchor, constant: -side),
            row.topAnchor.constraint(equalTo: contentLayoutGuide.topAnchor),
            row.bottomAnchor.constraint(equalTo: contentLayoutGuide.bottomAnchor),
            row.heightAnchor.constraint(equalTo: frameLayoutGuide.heightAnchor),
            heightAnchor.constraint(equalToConstant: 34),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func set(_ words: [String], chosen: Int) {
        chips.forEach { $0.removeFromSuperview() }
        chips = words.enumerated().map { i, w in
            let c = fit ? Chip(w, chosen: i == chosen, inset: 2) : Chip(w, chosen: i == chosen)
            c.tag = i
            c.addTarget(self, action: #selector(tap(_:)), for: .touchUpInside)
            row.addArrangedSubview(c)
            return c
        }
    }

    @objc private func tap(_ c: Chip) {
        guard !c.chosen else { return }
        chips.forEach { $0.chosen = $0 === c }
        picked?(c.tag)
    }
}

/// The button a page is built around: an ink pill on the page's paper (Play), or the surface's (the others).
final class PillButton: UIButton {
    init(_ words: String, _ glyph: UIImage?, prominent: Bool = true, look: PageLook = Theme.page, height: CGFloat = 46) {
        super.init(frame: .zero)
        titleLabel?.font = Fonts.font(15, .semibold, .headline)
        set(words, glyph)
        paint(look, prominent: prominent)
        translatesAutoresizingMaskIntoConstraints = false
        heightAnchor.constraint(greaterThanOrEqualToConstant: height).isActive = true
        contentEdgeInsets = UIEdgeInsets(top: 0, left: 16, bottom: 0, right: 16)
        imageEdgeInsets = UIEdgeInsets(top: 0, left: -4, bottom: 0, right: 4)
        titleEdgeInsets = UIEdgeInsets(top: 0, left: 4, bottom: 0, right: -4)
        adjustsImageWhenHighlighted = false
    }

    required init?(coder: NSCoder) { fatalError() }

    func set(_ words: String, _ glyph: UIImage?) {
        setTitle(words, for: .normal)
        setImage(glyph, for: .normal)
    }

    func paint(_ look: PageLook, prominent: Bool = true) {
        backgroundColor = prominent ? look.ink : look.surface
        tintColor = prominent ? look.page : look.ink
        setTitleColor(prominent ? look.page : look.ink, for: .normal)
        layer.borderWidth = prominent ? 0 : hairlineWidth
        layer.borderColor = look.hairline.cgColor
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        layer.cornerRadius = bounds.height / 2
    }

    override var isHighlighted: Bool {
        didSet { alpha = isHighlighted ? 0.7 : 1 }
    }
}

/// A bare glyph in a 48 pt target: in the page's ink, its quieter ink, or its accent while lit.
final class GlyphButton: UIButton {
    var lit = false { didSet { paint() } }
    var quiet = false { didSet { paint() } }
    var look = Theme.page { didSet { paint() } }

    /// `side`: the target's, 48 pt unless the place gives it more (the player's transport).
    init(_ glyph: UIImage, _ words: String, quiet q: Bool = false, side: CGFloat = 48) {
        super.init(frame: .zero)
        setImage(glyph, for: .normal)
        accessibilityLabel = words
        quiet = q
        translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([widthAnchor.constraint(equalToConstant: side), heightAnchor.constraint(equalToConstant: side)])
        paint()
    }

    required init?(coder: NSCoder) { fatalError() }

    private func paint() {
        tintColor = lit ? look.accent : quiet ? look.quiet : look.ink
        accessibilityTraits = lit ? [.button, .selected] : .button
    }

    override var isHighlighted: Bool {
        didSet { alpha = isHighlighted ? 0.5 : 1 }
    }
}

/// A menu over the page: opaque paper with a hairline edge rising from the bottom, what it is for at its
/// head, and its lines grouped on a card; lines one reaches for rarely on a card of their own.Every menu and
/// choice of the app is one.
class Sheet: UIViewController {
    private struct Line {
        let words: String
        let glyph: UIImage?
        let checked: Bool
        let destructive: Bool
        let enabled: Bool
        let more: Bool
        let subtitle: String?
        /// The line does its work with the sheet still up (it puts something over it).
        let stays: Bool
        let run: () -> Void
    }

    private let heading: String?
    private let message: String?
    private let cover: String?
    private var lines: [Line] = []
    private let dim = UIView()
    private let paper = UIView()
    /// A sheet's cards sit nearer its sides than a page's, their words on the head's line.
    private static let side: CGFloat = 6
    /// Where a sheet's head and its cards' words start.
    private static let words = side + Space.groupInset
    private let card = GroupCard(margin: Sheet.side)
    private let rest = GroupCard(margin: Sheet.side)
    /// Bare glyphs beside the title.
    var titleGlyphs: [UIView] = []
    /// Under the title: what opens the song's artist and album.
    var names: [UIView] = []
    /// The line under the head: quiet words, and bare glyphs at its end.
    var under: (words: String, glyphs: [UIView])?

    /// `cover`: the cover the menu's song or record leads the sheet with.
    init(_ heading: String?, _ message: String? = nil, cover: String? = nil) {
        self.heading = heading
        self.message = message
        self.cover = cover
        super.init(nibName: nil, bundle: nil)
        modalPresentationStyle = .overFullScreen
        modalPresentationCapturesStatusBarAppearance = false
    }

    required init?(coder: NSCoder) { fatalError() }

    func add(_ words: String, subtitle: String? = nil, glyph: UIImage? = nil, checked: Bool = false, destructive: Bool = false, enabled: Bool = true,
             more: Bool = false, stays: Bool = false, run: @escaping () -> Void) {
        lines.append(Line(words: words, glyph: glyph, checked: checked, destructive: destructive, enabled: enabled, more: more, subtitle: subtitle, stays: stays, run: run))
    }

    /// The lines `fill` adds in place of the ones shown: a list that changes while it is open.
    func refill(_ fill: () -> Void) {
        lines = []
        fill()
        lay()
    }

    func show(from host: UIViewController) {
        var top = host
        while let next = top.presentedViewController, !next.isBeingDismissed { top = next }
        top.present(self, animated: false)
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .clear
        dim.backgroundColor = UIColor.black.withAlphaComponent(0.3)
        dim.alpha = 0
        dim.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(closeTapped)))
        paper.backgroundColor = Theme.background
        paper.layer.cornerRadius = Radius.sheet
        paper.layer.maskedCorners = [.layerMinXMinYCorner, .layerMaxXMinYCorner]
        paper.layer.borderWidth = hairlineWidth
        paper.layer.borderColor = Theme.hairline.cgColor
        let head = UIStackView()
        head.axis = .vertical
        head.spacing = 2
        if let heading, !heading.isEmpty {
            head.addArrangedSubview(label(Fonts.font(16, .semibold, .headline), Theme.label, lines: cover == nil ? 2 : 1).then { $0.text = heading })
        }
        if let message, !message.isEmpty {
            head.addArrangedSubview(label(Fonts.font(13, .regular, .footnote), Theme.secondary, lines: 3).then { $0.text = message })
        }
        names.forEach(head.addArrangedSubview)
        let top = UIStackView(arrangedSubviews: [head] + titleGlyphs)
        top.spacing = 12
        titleGlyphs.forEach { top.setCustomSpacing(0, after: $0) }
        top.alignment = .center
        if let cover, !cover.isEmpty {
            let art = CoverView()
            art.layer.cornerRadius = 6
            art.translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([art.widthAnchor.constraint(equalToConstant: 44), art.heightAnchor.constraint(equalToConstant: 44)])
            art.show(cover, points: 44)
            top.insertArrangedSubview(art, at: 0)
        }
        // The head's glyphs end where a group's words do, their touch room past them.
        let glyphEnd = Sheet.words - 10
        top.isLayoutMarginsRelativeArrangement = true
        top.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 0, leading: Sheet.words, bottom: 0, trailing: titleGlyphs.isEmpty ? Sheet.words : glyphEnd)
        var parts: [UIView] = head.arrangedSubviews.isEmpty ? [] : [top]
        var stands: [(UIView, CGFloat)] = parts.isEmpty ? [] : [(top, under == nil ? 14 : 8)]
        if let under {
            let rule = UIStackView(arrangedSubviews: [hairlineView()])
            rule.isLayoutMarginsRelativeArrangement = true
            rule.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 0, leading: Sheet.side, bottom: 0, trailing: Sheet.side)
            let words = label(Fonts.font(13, .regular, .footnote), Theme.secondary).then { $0.text = under.words }
            words.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            let line = UIStackView(arrangedSubviews: [words] + under.glyphs)
            line.alignment = .center
            line.isLayoutMarginsRelativeArrangement = true
            line.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 0, leading: Sheet.words, bottom: 0, trailing: glyphEnd)
            line.heightAnchor.constraint(greaterThanOrEqualToConstant: 40).isActive = true
            parts += [rule, line]
            stands += [(rule, 0), (line, 2)]
        }
        parts += [card, rest]
        let scroll = UIScrollView()
        let column = UIStackView(arrangedSubviews: parts)
        column.axis = .vertical
        column.spacing = 8
        stands.forEach { column.setCustomSpacing($0.1, after: $0.0) }
        column.isLayoutMarginsRelativeArrangement = true
        column.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 22, leading: 0, bottom: 16, trailing: 0)
        for v in [dim, paper, scroll, column] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
        }
        view.addSubview(dim)
        view.addSubview(paper)
        paper.addSubview(scroll)
        scroll.addSubview(column)
        let fits = scroll.heightAnchor.constraint(equalTo: column.heightAnchor)
        fits.priority = .defaultHigh
        NSLayoutConstraint.activate([
            dim.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            dim.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            dim.topAnchor.constraint(equalTo: view.topAnchor),
            dim.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            paper.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            paper.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            paper.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            paper.topAnchor.constraint(greaterThanOrEqualTo: view.safeAreaLayoutGuide.topAnchor, constant: 40),
            scroll.leadingAnchor.constraint(equalTo: paper.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: paper.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: paper.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor),
            fits,
            column.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor),
            column.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor),
            column.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor),
            column.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor),
            column.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor),
        ])
        let pan = UIPanGestureRecognizer(target: self, action: #selector(dragged(_:)))
        paper.addGestureRecognizer(pan)
        lay()
    }

    /// The lines on their cards: what is done with the song, then what opens more or switches a state.
    private func lay() {
        card.set(lines.filter { !$0.more }.map(row))
        let others = lines.filter { $0.more }
        rest.set(others.map(row))
        rest.isHidden = others.isEmpty
    }

    private func row(_ l: Line) -> GroupRow {
        let check = l.checked ? UIImageView(image: Glyph.check).then { $0.tintColor = Theme.secondary } : nil
        let r = GroupRow(l.words, subtitle: l.subtitle, icon: l.glyph, trailing: check, minHeight: 48) { [weak self] in
            if l.stays { l.run() } else { self?.close(then: l.run) }
        }
        if l.destructive { r.title.textColor = UIColor(argb: 0xFFC6_3A3A) }
        r.isEnabled = l.enabled
        return r
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        view.layoutIfNeeded()
        paper.transform = CGAffineTransform(translationX: 0, y: paper.bounds.height)
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        UIView.animate(withDuration: Motion.plain ? 0 : 0.28, delay: 0, options: .curveEaseOut, animations: {
            self.dim.alpha = 1
            self.paper.transform = .identity
        })
    }

    @objc private func closeTapped() { close(then: nil) }

    func close(then run: (() -> Void)?) {
        UIView.animate(withDuration: Motion.plain ? 0 : 0.2, delay: 0, options: .curveEaseIn, animations: {
            self.dim.alpha = 0
            self.paper.transform = CGAffineTransform(translationX: 0, y: self.paper.bounds.height)
        }, completion: { _ in
            // What a line opens (another sheet, a page) comes once the menu is gone.
            self.dismiss(animated: false) { run?() }
        })
    }

    @objc private func dragged(_ g: UIPanGestureRecognizer) {
        let y = max(0, g.translation(in: view).y)
        switch g.state {
        case .changed:
            paper.transform = CGAffineTransform(translationX: 0, y: y)
        case .ended, .cancelled:
            if y > 90 || g.velocity(in: view).y > 700 {
                close(then: nil)
            } else {
                UIView.animate(withDuration: Motion.plain ? 0 : 0.2) { self.paper.transform = .identity }
            }
        default:
            break
        }
    }
}

/// How much the interface moves: the system's reduce motion turns flourishes into direct changes.
enum Motion {
    static var plain: Bool { UIAccessibility.isReduceMotionEnabled }
}

/// Configures `self` in place: `label(...).then { $0.text = "…" }`.
protocol Then {}
extension Then {
    @discardableResult
    func then(_ apply: (Self) -> Void) -> Self {
        apply(self)
        return self
    }
}
extension NSObject: Then {}
