import UIKit

/// The top of an album, artist, playlist or mix page, built as Android's HeroPage: the artwork runs edge
/// to edge under the status bar and melts into the page over its bottom rows, the title rises into the
/// melt, then Shuffle, the ink Play pill and the page's own glyph (the seal, or Save as playlist), and the
/// list head: what the list is ("2019 · 8 songs · 36:28") and its glyphs (`nori_ios_page_actions`).
/// Static: one gradient, no blur. The title, the buttons and the list head are `head`, which stays at the
/// top once it reaches it while the songs go on under it.
final class HeroHeader: UIView {
    /// Laid in the table over this header's foot, not in the header: the table's rows pass under it.
    let head = UIView()
    private let backing = UIView()
    private let edge: GradientView
    /// Where `head` stands before it sticks.
    private var rest: CGFloat = 0
    private let statusRoom = UIApplication.shared.statusBarFrame.height
    /// The page's color covers the top of the screen.
    private(set) var stuck = false
    /// Over how much of its way up the page's color comes in behind the head.
    private static let fade: CGFloat = 64
    /// The songs' fade under the head.
    private static let edgeHeight: CGFloat = 18

    var play: (() -> Void)?
    var shuffle: (() -> Void)?
    /// The artist line was tapped.
    var subtitleTapped: (() -> Void)?
    private let playButton: PillButton
    private let shuffleButton: GlyphButton
    let picture: UIView?
    private let look: PageLook

    /// How much taller than square the picture is, the title rising by as much.
    static let deeper: CGFloat = 44
    /// The room an artist line takes; a page without one lets its title down by it.
    private static let artistLine: CGFloat = 28

    /// `head`: the page's head as the core answers it. `title` and `line`: its words. `own`: the glyphs after
    /// Play; `actions`: the list head's.
    init(head: [String: Any], title: String, subtitle: String?, line: String, look: PageLook, own: [UIView], actions: [UIView]) {
        self.look = look
        edge = GradientView(colours: [look.page, look.page.withAlphaComponent(0)])
        let kind = head["k"] as? String ?? ""
        let width = UIScreen.main.bounds.width
        playButton = PillButton(Say.play, Glyph.playPill, look: look)
        shuffleButton = GlyphButton(Glyph.shufflePage, Say.shuffle)
        shuffleButton.look = look
        let covers = head["covers"] as? [String] ?? []
        let cover = head["c"] as? String ?? ""
        if kind == "mix" && !covers.isEmpty {
            picture = Collage(covers: covers, side: width)
        } else if !cover.isEmpty {
            let one = CoverView()
            one.layer.cornerRadius = 0
            one.show(cover, points: width)
            picture = one
        } else {
            picture = nil
        }
        super.init(frame: CGRect(x: 0, y: 0, width: width, height: 10))
        backgroundColor = .clear

        let name = label(Fonts.title(kind == "artist" ? 32 : 27, .title1), look.ink, lines: 2)
        name.text = title
        name.textAlignment = .center
        let sub = label(Fonts.font(17, .medium, .headline), look.ink)
        sub.text = subtitle
        sub.textAlignment = .center
        sub.isHidden = (subtitle ?? "").isEmpty
        if subtitle != nil {
            sub.isUserInteractionEnabled = true
            sub.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(subTapped)))
        }
        let words = UIStackView(arrangedSubviews: [name, sub])
        words.axis = .vertical
        words.spacing = 3

        playButton.addTarget(self, action: #selector(playTapped), for: .touchUpInside)
        shuffleButton.addTarget(self, action: #selector(shuffleTapped), for: .touchUpInside)
        let buttons = UIStackView(arrangedSubviews: [shuffleButton, playButton] + own)
        buttons.spacing = 4
        buttons.alignment = .center

        let quiet = label(Fonts.font(13, .regular, .footnote), look.quiet)
        quiet.text = line
        quiet.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let listHead = UIStackView(arrangedSubviews: [quiet] + actions)
        listHead.alignment = .center
        let rule = hairlineView(look.hairline)

        rest = 72
        if let picture {
            picture.translatesAutoresizingMaskIntoConstraints = false
            addSubview(picture)
            let melt = GradientView(colours: look.melt, at: look.stops)
            let shade = GradientView(colours: [UIColor.black.withAlphaComponent(0.30), UIColor.black.withAlphaComponent(0)], at: [0, 0.16])
            for v in [melt, shade] {
                v.translatesAutoresizingMaskIntoConstraints = false
                picture.addSubview(v)
                NSLayoutConstraint.activate([
                    v.leadingAnchor.constraint(equalTo: picture.leadingAnchor),
                    v.trailingAnchor.constraint(equalTo: picture.trailingAnchor),
                    v.topAnchor.constraint(equalTo: picture.topAnchor),
                    v.bottomAnchor.constraint(equalTo: picture.bottomAnchor),
                ])
            }
            NSLayoutConstraint.activate([
                picture.leadingAnchor.constraint(equalTo: leadingAnchor),
                picture.trailingAnchor.constraint(equalTo: trailingAnchor),
                picture.topAnchor.constraint(equalTo: topAnchor),
                picture.heightAnchor.constraint(equalToConstant: width + HeroHeader.deeper),
            ])
            rest = width + HeroHeader.deeper - (Space.titleRise + HeroHeader.deeper - (sub.isHidden ? HeroHeader.artistLine : 0))
        }
        let inHead = self.head
        backing.backgroundColor = look.page
        backing.isUserInteractionEnabled = false
        inHead.addSubview(backing)
        inHead.addSubview(edge)
        inHead.layer.zPosition = 1
        for v in [words, buttons, listHead, rule] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            inHead.addSubview(v)
        }
        NSLayoutConstraint.activate([
            words.leadingAnchor.constraint(equalTo: inHead.leadingAnchor, constant: Space.gutter),
            words.trailingAnchor.constraint(equalTo: inHead.trailingAnchor, constant: -Space.gutter),
            words.topAnchor.constraint(equalTo: inHead.topAnchor),
            buttons.leadingAnchor.constraint(equalTo: inHead.leadingAnchor, constant: Space.tight),
            buttons.trailingAnchor.constraint(equalTo: inHead.trailingAnchor, constant: -Space.tight),
            buttons.topAnchor.constraint(equalTo: words.bottomAnchor, constant: 12),
            listHead.leadingAnchor.constraint(equalTo: inHead.leadingAnchor, constant: Space.gutter),
            listHead.trailingAnchor.constraint(equalTo: inHead.trailingAnchor, constant: -Space.tight),
            listHead.topAnchor.constraint(equalTo: buttons.bottomAnchor, constant: 6),
            listHead.heightAnchor.constraint(greaterThanOrEqualToConstant: 48),
            rule.leadingAnchor.constraint(equalTo: inHead.leadingAnchor, constant: Space.gutter),
            rule.trailingAnchor.constraint(equalTo: inHead.trailingAnchor, constant: -Space.gutter),
            rule.topAnchor.constraint(equalTo: listHead.bottomAnchor),
            rule.bottomAnchor.constraint(equalTo: inHead.bottomAnchor, constant: -10),
        ])
        listHead.isHidden = line.isEmpty && actions.isEmpty
        rule.isHidden = listHead.isHidden
        let size = inHead.systemLayoutSizeFitting(
            CGSize(width: width, height: UIView.layoutFittingCompressedSize.height),
            withHorizontalFittingPriority: .required, verticalFittingPriority: .fittingSizeLevel
        )
        inHead.frame = CGRect(x: 0, y: rest, width: width, height: size.height)
        frame.size.height = rest + size.height
    }

    required init?(coder: NSCoder) { fatalError() }

    /// Scrolled `by` points: the head stays at the top once it reaches it, the page's color coming in behind
    /// it on the way, and the picture moves at less than the list's pace and fades a little. True when
    /// `stuck` changed.
    @discardableResult
    func scrolled(_ by: CGFloat) -> Bool {
        let top = by + statusRoom
        let from = max(0, rest - top)
        head.frame.origin.y = max(rest, top)
        let shown = 1 - min(1, from / HeroHeader.fade)
        backing.alpha = shown
        edge.alpha = shown
        let width = head.bounds.width, height = head.bounds.height
        backing.frame = CGRect(x: 0, y: -(from + statusRoom), width: width, height: height + from + statusRoom)
        edge.frame = CGRect(x: 0, y: height, width: width, height: HeroHeader.edgeHeight)
        if let picture {
            let moved = max(0, by)
            picture.transform = CGAffineTransform(translationX: 0, y: moved * 0.4)
            picture.alpha = 1 - min(1, moved / max(1, picture.bounds.height)) * 0.5
        }
        let was = stuck
        stuck = shown > 0.5
        return stuck != was
    }

    /// The buttons as the core says they stand (`nori_ios_hero`): Play is Pause while this page's queue
    /// sounds, and Shuffle lights while that queue shuffles.
    func paint(hero bits: Int32) {
        let pausing = bits & 4 != 0
        playButton.set(pausing ? Say.pause : Say.play, pausing ? Glyph.pausePill : Glyph.playPill)
        shuffleButton.lit = bits & 1 != 0
    }

    @objc private func subTapped() { subtitleTapped?() }
    @objc private func playTapped() { play?() }
    @objc private func shuffleTapped() { shuffle?() }
}

final class GradientView: UIView {
    override class var layerClass: AnyClass { CAGradientLayer.self }

    init(colours: [UIColor], at stops: [CGFloat]? = nil) {
        super.init(frame: .zero)
        let g = layer as! CAGradientLayer
        g.colors = colours.map { $0.cgColor }
        g.locations = stops?.map { NSNumber(value: Double($0)) }
        isUserInteractionEnabled = false
    }

    required init?(coder: NSCoder) { fatalError() }
}
