import UIKit

/// A list row: the cover (or the track number on an album), title and a quiet line; the hairline under
/// it starts at the title, except under a song. The song playing has its title in the accent. A song's marks
/// sit in columns of their own, as Android's rows: its download, then its seal at the edge, each in its
/// place whether or not the other is shown.
final class ItemCell: UITableViewCell {
    static let id = "item"
    let cover = CoverView()
    private let number = label(Fonts.digits(15), Theme.secondary)
    private let title = label(Fonts.font(16), Theme.label)
    private let detail = label(Fonts.font(13, .regular, .footnote), Theme.secondary)
    private let saved = UIImageView()
    private let mark = UIImageView(image: Glyph.sealedRow)
    /// "Asked": a jam guest's request the host has yet to take.
    private let asked = label(Fonts.font(12, .medium, .caption1), Theme.secondary)
    private let bar = UIProgressView(progressViewStyle: .bar)
    private let line = hairlineView()
    private var coverWidth: NSLayoutConstraint!
    private var textStart: NSLayoutConstraint!

    override init(style: UITableViewCell.CellStyle, reuseIdentifier: String?) {
        super.init(style: style, reuseIdentifier: reuseIdentifier)
        backgroundColor = .clear
        let selected = UIView()
        selected.backgroundColor = Theme.page.veil(0.06)
        selectedBackgroundView = selected
        number.textAlignment = .center
        let text = UIStackView(arrangedSubviews: [title, detail])
        text.axis = .vertical
        text.spacing = 2
        asked.text = Say.asked
        let trail = UIStackView(arrangedSubviews: [asked, saved, mark])
        trail.spacing = 8
        trail.setContentHuggingPriority(.required, for: .horizontal)
        trail.setContentCompressionResistancePriority(.required, for: .horizontal)
        for v in [cover, number, text, trail, bar, line] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            contentView.addSubview(v)
        }
        coverWidth = cover.widthAnchor.constraint(equalToConstant: 44)
        textStart = text.leadingAnchor.constraint(equalTo: cover.trailingAnchor, constant: Space.row)
        NSLayoutConstraint.activate([
            cover.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: Space.gutter),
            cover.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
            coverWidth,
            cover.heightAnchor.constraint(equalTo: cover.widthAnchor),
            number.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: Space.gutter - 6),
            number.widthAnchor.constraint(equalToConstant: 30),
            number.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
            textStart,
            text.trailingAnchor.constraint(equalTo: trail.leadingAnchor, constant: -8),
            text.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
            bar.leadingAnchor.constraint(equalTo: text.leadingAnchor),
            bar.trailingAnchor.constraint(equalTo: text.trailingAnchor),
            bar.topAnchor.constraint(equalTo: text.bottomAnchor, constant: 4),
            bar.heightAnchor.constraint(equalToConstant: 2),
            trail.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -Space.gutter),
            trail.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
            saved.widthAnchor.constraint(equalToConstant: 15),
            mark.widthAnchor.constraint(equalToConstant: 15),
            contentView.heightAnchor.constraint(greaterThanOrEqualToConstant: 60),
            line.leadingAnchor.constraint(equalTo: text.leadingAnchor),
            line.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            line.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    /// `numbered`: the album page's rows show the track number instead of a cover. `look`: the page's.
    func show(_ item: Item, numbered: Bool, current: Bool = false, look: PageLook = Theme.page) {
        let line: String
        switch item.kind {
        case "song" where item.progress != nil:
            let p = item.progress!
            line = Say.downloading(percent: p.percent, speed: p.speed, left: p.left)
        case "song":
            line = numbered ? (item.seconds > 0 ? Fmt.clock(ms: item.seconds * 1000) : "")
                : [item.subtitle, item.seconds > 0 ? Fmt.clock(ms: item.seconds * 1000) : ""].filter { !$0.isEmpty }.joined(separator: " · ")
        case "artist":
            line = item.count > 0 ? Say.albums(item.count) : ""
        case "playlist":
            line = item.count > 0 ? Say.songs(item.count) : item.subtitle
        case "genre":
            line = Say.albums(item.count)
        case "smart":
            line = ""
        default:
            line = item.subtitle
        }
        // With no second line (an album's own tracks) the cloud leads the title instead.
        let name: String = {
            if item.kind == "smart", item.title.isEmpty { return Say.smart(item.count) }
            return item.title.isEmpty ? item.id : item.title
        }()
        title.attributedText = remoteText(name, remote: item.external && line.isEmpty, font: title.font,
                                          colour: current ? look.accent : look.ink)
        detail.attributedText = remoteText(line, remote: item.external && !line.isEmpty, font: detail.font, colour: look.quiet)
        number.textColor = current ? look.accent : look.quiet
        self.line.backgroundColor = look.hairline
        self.line.isHidden = item.kind == "song"
        asked.textColor = look.quiet
        asked.isHidden = !Core.shared.isAsked(item)
        let song = item.kind == "song"
        saved.isHidden = !song
        saved.image = item.download == 2 ? Glyph.downloadedRow : item.download == 1 ? Glyph.downloadingRow : nil
        saved.tintColor = look.quiet
        mark.tintColor = look.accent
        mark.isHidden = !song || !asked.isHidden
        mark.alpha = Core.shared.rules.stars && Core.shared.isFavorite(item) ? 1 : 0
        bar.progressTintColor = look.ink
        bar.trackTintColor = look.veil(0.12)
        bar.isHidden = (item.progress?.percent ?? -1) < 0
        bar.progress = Float(max(0, item.progress?.percent ?? 0)) / 100
        detail.isHidden = line.isEmpty
        let bare = numbered || item.kind == "genre" || item.kind == "recent"
        cover.isHidden = bare
        number.isHidden = !numbered
        number.text = item.track > 0 ? "\(item.track)" : ""
        coverWidth.constant = bare ? 24 : 44
        cover.layer.cornerRadius = item.kind == "artist" ? 22 : Radius.cover
        if !cover.isHidden { cover.show(item.cover, points: 44) }
        accessoryType = .none
    }

    override func prepareForReuse() {
        super.prepareForReuse()
        cover.cancel()
    }
}

/// `text`, led by the cloud when the item is a provider's (not in the library yet). The cloud comes first
/// so the text, not the mark, is what gets cut short.
func remoteText(_ text: String, remote: Bool, font: UIFont, colour: UIColor) -> NSAttributedString {
    let out = NSMutableAttributedString()
    if remote {
        let mark = NSTextAttachment()
        let side = font.pointSize + 1
        mark.image = Glyph.cloud(side)
        mark.bounds = CGRect(x: 0, y: (font.capHeight - side) / 2, width: side, height: side)
        out.append(NSAttributedString(attachment: mark))
        if !text.isEmpty { out.append(NSAttributedString(string: " ")) }
    }
    out.append(NSAttributedString(string: text, attributes: [.font: font, .foregroundColor: colour]))
    return out
}

/// A print on a shelf or in a grid: an album, playlist, song or artist (round), its name and a quiet line;
/// or a mix's card.
final class CardCell: UICollectionViewCell {
    static let id = "card"
    let cover = CoverView()
    private let title = label(Fonts.font(14, .medium, .subheadline), Theme.label)
    private let detail = label(Fonts.font(12.5, .regular, .caption1), Theme.secondary)
    private var mix: MixCard?

    override init(frame: CGRect) {
        super.init(frame: frame)
        cover.layer.cornerRadius = Radius.cover
        for v in [cover, title, detail] {
            v.translatesAutoresizingMaskIntoConstraints = false
            contentView.addSubview(v)
        }
        NSLayoutConstraint.activate([
            cover.topAnchor.constraint(equalTo: contentView.topAnchor),
            cover.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            cover.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            cover.heightAnchor.constraint(equalTo: cover.widthAnchor),
            title.topAnchor.constraint(equalTo: cover.bottomAnchor, constant: 7),
            title.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            title.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            detail.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 1),
            detail.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            detail.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func show(_ item: Item, width: CGFloat, look: PageLook) {
        mix?.removeFromSuperview()
        mix = nil
        if item.kind == "mix" {
            let card = MixCard(item, width: width)
            card.frame = CGRect(x: 0, y: 0, width: width, height: width * MixCard.tall)
            contentView.addSubview(card)
            mix = card
            cover.cancel()
            cover.isHidden = true
            title.text = nil
            detail.text = nil
            return
        }
        cover.isHidden = false
        title.text = item.title
        title.textColor = look.ink
        let line: String
        switch item.kind {
        case "artist": line = item.count > 0 ? Say.albums(item.count) : ""
        case "playlist": line = item.count > 0 ? Say.songs(item.count) : item.subtitle
        case "album" where item.subtitle.isEmpty: line = item.year > 0 ? "\(item.year)" : ""
        default: line = item.subtitle
        }
        detail.attributedText = remoteText(line, remote: item.external, font: detail.font, colour: look.quiet)
        title.textAlignment = item.kind == "artist" ? .center : .natural
        detail.textAlignment = title.textAlignment
        cover.layer.cornerRadius = item.kind == "artist" ? width / 2 : Radius.cover
        cover.show(item.cover, points: width)
    }

    override func prepareForReuse() {
        super.prepareForReuse()
        cover.cancel()
    }
}

/// A mix's covers as one square: four in a mosaic, or its one when it has fewer.
final class Collage: UIView {
    private var views: [CoverView] = []

    init(covers all: [String], side: CGFloat) {
        super.init(frame: .zero)
        clipsToBounds = true
        let covers = Array(all.prefix(all.count >= 4 ? 4 : 1))
        let half = covers.count == 4
        for (i, id) in covers.enumerated() {
            let v = CoverView()
            v.layer.cornerRadius = 0
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
            let f: CGFloat = half ? 0.5 : 1
            NSLayoutConstraint.activate([
                v.widthAnchor.constraint(equalTo: widthAnchor, multiplier: f),
                v.heightAnchor.constraint(equalTo: heightAnchor, multiplier: f),
                half && i % 2 == 1 ? v.trailingAnchor.constraint(equalTo: trailingAnchor) : v.leadingAnchor.constraint(equalTo: leadingAnchor),
                half && i >= 2 ? v.bottomAnchor.constraint(equalTo: bottomAnchor) : v.topAnchor.constraint(equalTo: topAnchor),
            ])
            v.show(id, points: half ? side / 2 : side)
            views.append(v)
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    override func removeFromSuperview() {
        views.forEach { $0.cancel() }
        super.removeFromSuperview()
    }
}

/// A mix on Home: a portrait card on the season's paper dyed with the mix's color (the core's
/// `mix_card_colours`), its covers a square on top running into that paper, and its name low on the paper,
/// over the end of the fade, in the card's own ink.
final class MixCard: UIView {
    /// The card's height to its width: 186 to 150.
    static let tall: CGFloat = 186 / 150

    init(_ item: Item, width: CGFloat) {
        super.init(frame: .zero)
        var dyed = NoriPalette()
        var light = Theme.look.light, dark = Theme.look.dark
        nori_ios_mix_card(item.id, &light, &dark, Theme.look.dark_theme, &dyed)
        let page = UIColor(argb: dyed.page), ink = UIColor(argb: dyed.ink)
        backgroundColor = page
        layer.cornerRadius = Radius.tile
        clipsToBounds = true
        let art = Collage(covers: item.covers, side: width)
        art.frame = CGRect(x: 0, y: 0, width: width, height: width)
        // The covers rubbed out into the card's paper below their middle.
        let fade = CAGradientLayer()
        fade.frame = art.bounds
        fade.colors = [UIColor.black.cgColor, UIColor.black.cgColor, UIColor.clear.cgColor]
        fade.locations = [0, 0.45, 1]
        art.layer.mask = fade
        let name = label(Fonts.title(20, .headline), ink, lines: 2)
        name.text = Say.mix(item.count)
        let high = name.sizeThatFits(CGSize(width: width - 26, height: .greatestFiniteMagnitude)).height
        name.frame = CGRect(x: 13, y: width * MixCard.tall - 12 - high, width: width - 26, height: high)
        for v in [art, name] { addSubview(v) }
        isUserInteractionEnabled = false
        isAccessibilityElement = true
        accessibilityLabel = name.text
    }

    required init?(coder: NSCoder) { fatalError() }
}

/// A horizontal shelf of prints in one table row.
final class ShelfCell: UITableViewCell, UICollectionViewDataSource, UICollectionViewDelegateFlowLayout {
    static let id = "shelf"
    /// A print on a shelf; an album on an artist's shelf in the library is a little smaller.
    static let print: CGFloat = 128
    static let small: CGFloat = 112
    private static let words: CGFloat = 44
    static let mixWidth: CGFloat = 150

    /// The row's height for `items` printed `side` wide.
    static func height(_ items: [Item], side: CGFloat = print) -> CGFloat {
        items.allSatisfy { $0.kind == "mix" } && !items.isEmpty ? mixWidth * MixCard.tall : side + words
    }

    private let strip: UICollectionView
    private let layout: UICollectionViewFlowLayout
    private var stripHeight: NSLayoutConstraint!
    private var items: [Item] = []
    private var side = ShelfCell.print
    private var look = Theme.page
    var picked: ((Item) -> Void)?
    var held: ((Item) -> Void)?

    override init(style: UITableViewCell.CellStyle, reuseIdentifier: String?) {
        let layout = UICollectionViewFlowLayout()
        self.layout = layout
        layout.scrollDirection = .horizontal
        layout.minimumLineSpacing = Space.row
        layout.sectionInset = UIEdgeInsets(top: 0, left: Space.gutter, bottom: 0, right: Space.gutter)
        strip = UICollectionView(frame: .zero, collectionViewLayout: layout)
        super.init(style: style, reuseIdentifier: reuseIdentifier)
        backgroundColor = .clear
        selectionStyle = .none
        strip.backgroundColor = .clear
        strip.showsHorizontalScrollIndicator = false
        strip.register(CardCell.self, forCellWithReuseIdentifier: CardCell.id)
        strip.dataSource = self
        strip.delegate = self
        strip.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(strip)
        stripHeight = strip.heightAnchor.constraint(equalToConstant: ShelfCell.print + ShelfCell.words)
        NSLayoutConstraint.activate([
            strip.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            strip.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            strip.topAnchor.constraint(equalTo: contentView.topAnchor),
            stripHeight,
            contentView.bottomAnchor.constraint(equalTo: strip.bottomAnchor),
        ])
        strip.addGestureRecognizer(UILongPressGestureRecognizer(target: self, action: #selector(pressed(_:))))
    }

    required init?(coder: NSCoder) { fatalError() }

    func show(_ items: [Item], side: CGFloat = ShelfCell.print, look: PageLook) {
        self.items = items
        self.side = side
        self.look = look
        let height = ShelfCell.height(items, side: side)
        stripHeight.constant = height
        let mixes = items.first?.kind == "mix"
        layout.itemSize = CGSize(width: mixes ? ShelfCell.mixWidth : side, height: height)
        strip.reloadData()
        strip.setContentOffset(.zero, animated: false)
    }

    func collectionView(_ collectionView: UICollectionView, numberOfItemsInSection section: Int) -> Int { items.count }

    func collectionView(_ collectionView: UICollectionView, cellForItemAt indexPath: IndexPath) -> UICollectionViewCell {
        let cell = collectionView.dequeueReusableCell(withReuseIdentifier: CardCell.id, for: indexPath) as! CardCell
        let item = items[indexPath.item]
        cell.show(item, width: item.kind == "mix" ? ShelfCell.mixWidth : side, look: look)
        return cell
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        picked?(items[indexPath.item])
    }

    @objc private func pressed(_ g: UILongPressGestureRecognizer) {
        guard g.state == .began, let path = strip.indexPathForItem(at: g.location(in: strip)) else { return }
        held?(items[path.item])
    }
}

/// A section's title over its rows, as `SectionTitle` draws it; with an arrow, it opens the section in full.
final class SectionHeading: UITableViewHeaderFooterView {
    static let id = "heading"
    let title = SectionTitle()

    override init(reuseIdentifier: String?) {
        super.init(reuseIdentifier: reuseIdentifier)
        backgroundView = UIView()
        title.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(title)
        NSLayoutConstraint.activate([
            title.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            title.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            title.topAnchor.constraint(equalTo: contentView.topAnchor),
            title.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }
}

/// Two prints side by side, for the grids.
final class PairCell: UITableViewCell {
    static let id = "pair"
    private let left = CardButton()
    private let right = CardButton()
    var picked: ((Item) -> Void)?
    var held: ((Item) -> Void)?

    override init(style: UITableViewCell.CellStyle, reuseIdentifier: String?) {
        super.init(style: style, reuseIdentifier: reuseIdentifier)
        backgroundColor = .clear
        selectionStyle = .none
        let row = UIStackView(arrangedSubviews: [left, right])
        row.axis = .horizontal
        row.alignment = .top
        row.distribution = .fillEqually
        row.spacing = Space.row
        row.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: Space.gutter),
            row.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -Space.gutter),
            row.topAnchor.constraint(equalTo: contentView.topAnchor),
            row.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -Space.row),
        ])
        for b in [left, right] {
            b.addTarget(self, action: #selector(tapped(_:)), for: .touchUpInside)
            b.addGestureRecognizer(UILongPressGestureRecognizer(target: self, action: #selector(pressed(_:))))
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    static var width: CGFloat { (UIScreen.main.bounds.width - Space.gutter * 2 - Space.row) / 2 }

    /// The row's height: a print and its words, or a mix's card.
    static func height(mixes: Bool) -> CGFloat { mixes ? width * MixCard.tall + Space.row : width + 44 + Space.row }

    func show(_ a: Item, _ b: Item?, look: PageLook) {
        left.show(a, width: PairCell.width, look: look)
        right.isHidden = b == nil
        if let b { right.show(b, width: PairCell.width, look: look) }
    }

    @objc private func tapped(_ b: CardButton) {
        if let item = b.item { picked?(item) }
    }

    @objc private func pressed(_ g: UILongPressGestureRecognizer) {
        guard g.state == .began, let item = (g.view as? CardButton)?.item else { return }
        held?(item)
    }
}

/// A print that is a button, in a grid.
final class CardButton: UIControl {
    private let card = CardCell(frame: .zero)
    private var tall: NSLayoutConstraint!
    private(set) var item: Item?

    override init(frame: CGRect) {
        super.init(frame: frame)
        card.isUserInteractionEnabled = false
        card.translatesAutoresizingMaskIntoConstraints = false
        addSubview(card)
        tall = card.heightAnchor.constraint(equalToConstant: PairCell.height(mixes: false) - Space.row)
        NSLayoutConstraint.activate([
            card.leadingAnchor.constraint(equalTo: leadingAnchor),
            card.trailingAnchor.constraint(equalTo: trailingAnchor),
            card.topAnchor.constraint(equalTo: topAnchor),
            tall,
            heightAnchor.constraint(greaterThanOrEqualTo: card.heightAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isHighlighted: Bool {
        didSet { alpha = isHighlighted ? 0.6 : 1 }
    }

    func show(_ item: Item, width: CGFloat, look: PageLook) {
        self.item = item
        tall.constant = PairCell.height(mixes: item.kind == "mix") - Space.row
        card.show(item, width: width, look: look)
        accessibilityLabel = item.kind == "mix" ? Say.mix(item.count) : item.title
        isAccessibilityElement = true
        accessibilityTraits = .button
    }
}
