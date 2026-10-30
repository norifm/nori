import UIKit

/// The library, as Android's: a strip of chips for its sections and the one chosen below it, the places it
/// keeps (Favorites, Downloads) beside its title, and the list's bar of order, filter and Shuffle.
final class LibraryPage: UIViewController {
    /// Each chip with the core's LibrarySection it is, and the page it reads.
    private static let all: [(String, Int, Int32)] = [
        (Say.albumsTitle, 0, NORI_PAGE_ALBUMS),
        (Say.artists, 2, NORI_PAGE_ARTISTS),
        (Say.songsTitle, 3, NORI_PAGE_SONGS),
        (Say.playlists, 4, NORI_PAGE_PLAYLISTS),
        (Say.genres, 7, NORI_PAGE_GENRES),
    ]
    /// The chips the open profile has (a jam guest's: the host's albums, artists, songs and genres).
    private var chips: [(String, Int, Int32)] = []
    private static let kept = "nori.library.section"
    private var shown: PageController?

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Theme.background
        NotificationCenter.default.addObserver(self, selector: #selector(lay), name: .noriOpened, object: nil)
        lay()
    }

    @objc private func lay() {
        let sections = Core.shared.rules.sections
        chips = LibraryPage.all.filter { sections.contains($0.1) }
        navigationItem.rightBarButtonItems = places().reversed()
        let picked = UserDefaults.standard.integer(forKey: LibraryPage.kept)
        show(chips.firstIndex { $0.2 == Int32(picked) } ?? 0)
    }

    /// Shows chip `i`'s section under the strip and the places.
    private func show(_ i: Int) {
        shown?.willMove(toParent: nil)
        shown?.view.removeFromSuperview()
        shown?.removeFromParent()
        guard chips.indices.contains(i) else { return }
        let (name, _, kind) = chips[i]
        UserDefaults.standard.set(Int(kind), forKey: LibraryPage.kept)
        let page = PageController(kind: kind, title: name)
        page.top = { [unowned self, unowned page] in self.head(chosen: i, page: page) }
        addChild(page)
        page.view.frame = view.bounds
        page.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(page.view)
        page.didMove(toParent: self)
        shown = page
    }

    /// The strip of chips, Smart playlists over the playlists and, for a list with orders, its [SortBar]: the
    /// albums' with Favorites where the profile has favorites, Shuffle where the account's own library is.
    private func head(chosen: Int, page: PageController) -> UIView {
        let kind = page.kind
        let strip = ChipStrip(spacing: 6)
        strip.set(chips.map { $0.0 }, chosen: chosen)
        strip.picked = { [weak self] i in self?.show(i) }
        var parts: [UIView] = [strip]
        // The smart playlists are under Playlists.
        if kind == NORI_PAGE_PLAYLISTS && Core.shared.rules.sections.contains(5) {
            parts.append(GroupCard([GroupRow(Say.smartPlaylists, icon: Glyph.draw(Iconoir.magicWand, 20), chevron: true) { [weak self] in
                self?.navigationController?.pushViewController(PageController(kind: NORI_PAGE_SMARTS, title: Say.smartPlaylists), animated: true)
            }]))
        }
        let favorites = kind == NORI_PAGE_ALBUMS && Core.shared.rules.stars ? page.arg == NORI_ALBUMS_FAVORITES : nil
        let filtered = { [weak page] (on: Bool) in
            page?.arg = on ? NORI_ALBUMS_FAVORITES : ""
            page?.reload()
        }
        let albums: Int32 = kind == NORI_PAGE_ALBUMS ? 1 : 0
        let shuffle: (() -> Void)? = Core.shared.rules.account ? {
            DispatchQueue.global(qos: .userInitiated).async {
                if nori_ios_shuffle_library(albums) == 0 {
                    DispatchQueue.main.async { Toast.show(Say.note(NORI_NOTE_NOTHING_TO_PLAY, 0) ?? Say.failed) }
                }
            }
        } : nil
        if let bar = SortBar(kind: kind, favorites: favorites, filtered: filtered, shuffle: shuffle, changed: { [weak self] in self?.shown?.reload() }, asking: { [weak self] in self?.present($0, animated: true) }) { parts.append(bar) }
        let column = UIStackView(arrangedSubviews: parts)
        column.axis = .vertical
        column.spacing = 10
        column.isLayoutMarginsRelativeArrangement = true
        column.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 4, leading: 0, bottom: 10, trailing: 0)
        return column
    }

    /// The places the library keeps, not kinds of music: a glyph each beside its title, each opening its page.
    private func places() -> [UIBarButtonItem] {
        let rules = Core.shared.rules
        var items: [UIBarButtonItem] = []
        if rules.sections.contains(1) {
            items.append(BarGlyph(Glyph.draw(Iconoir.sealOutline, 22), Say.favorites) { [weak self] in
                self?.navigationController?.pushViewController(PageController(kind: NORI_PAGE_STARRED, title: Say.favorites), animated: true)
            })
        }
        if rules.sections.contains(11) {
            items.append(BarGlyph(Glyph.draw(Iconoir.download, 22), Say.downloads) { [weak self] in
                self?.navigationController?.pushViewController(DownloadsPage(), animated: true)
            })
        }
        return items
    }
}

/// A bar button that is a glyph and runs `tapped`.
final class BarGlyph: UIBarButtonItem {
    private var tapped: () -> Void = {}

    convenience init(_ glyph: UIImage, _ name: String, tapped: @escaping () -> Void) {
        self.init(image: glyph, style: .plain, target: nil, action: #selector(fire))
        target = self
        self.tapped = tapped
        accessibilityLabel = name
    }

    @objc private func fire() { tapped() }
}

/// A long list's bar, as Android's: the order it is in and which way that runs (`nori_ios_sorts`: the core's,
/// in its order), which opens the others on a sheet (`asking` puts it up); with `favorites`, a Favorites
/// filter apart from them, its seal lit while on, that `filtered` follows; Shuffle at the end (`shuffle`).
final class SortBar: UIView {
    private let kind: Int32
    private let all: [String]
    private let ways: [Int]
    private var now: Int
    private let order = Chip("", chosen: true)
    private var filter: Chip?
    private let filtered: (Bool) -> Void
    private let shuffle: (() -> Void)?
    private let changed: () -> Void
    private let asking: (UIViewController) -> Void

    init?(kind: Int32, favorites: Bool?, filtered: @escaping (Bool) -> Void, shuffle: (() -> Void)?, changed: @escaping () -> Void, asking: @escaping (UIViewController) -> Void) {
        guard let d = takenJSON(nori_ios_sorts(kind)) as? [String: Any], let all = d["all"] as? [String] else { return nil }
        self.kind = kind
        self.all = all
        ways = d["ways"] as? [Int] ?? []
        now = all.firstIndex { $0 == d["now"] as? String } ?? 0
        self.filtered = filtered
        self.shuffle = shuffle
        self.changed = changed
        self.asking = asking
        super.init(frame: .zero)
        order.addTarget(self, action: #selector(choose), for: .touchUpInside)
        var parts: [UIView] = [order]
        if let on = favorites {
            let f = Chip(Say.favorites, chosen: on)
            f.layer.borderWidth = hairlineWidth
            f.imageEdgeInsets = UIEdgeInsets(top: 0, left: -3, bottom: 0, right: 3)
            f.titleEdgeInsets = UIEdgeInsets(top: 0, left: 3, bottom: 0, right: -3)
            f.addTarget(self, action: #selector(flip), for: .touchUpInside)
            filter = f
            seal()
            parts.append(f)
        }
        parts.append(UIView())
        if shuffle != nil {
            let s = UIButton(type: .system)
            s.setImage(Glyph.shuffle, for: .normal)
            s.tintColor = Theme.label
            s.accessibilityLabel = Say.shuffle
            s.addTarget(self, action: #selector(shuffled), for: .touchUpInside)
            parts.append(s)
        }
        let row = UIStackView(arrangedSubviews: parts)
        row.spacing = 8
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Space.gutter),
            row.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -Space.gutter),
            row.topAnchor.constraint(equalTo: topAnchor),
            row.bottomAnchor.constraint(equalTo: bottomAnchor),
            heightAnchor.constraint(equalToConstant: 34),
        ])
        name()
    }

    required init?(coder: NSCoder) { fatalError() }

    /// The order's words, then which way it runs and the chevron that says it opens.
    private func name() {
        let font = Fonts.font(14, .medium, .subheadline)
        let words = NSMutableAttributedString(string: Say.sort(all[now]), attributes: [.font: font, .foregroundColor: Theme.label])
        let glyph = { (image: UIImage, colour: UIColor, side: CGFloat) in
            let a = NSTextAttachment()
            a.image = image.tinted(colour)
            a.bounds = CGRect(x: 0, y: (font.capHeight - side) / 2, width: side, height: side)
            words.append(NSAttributedString(string: " "))
            words.append(NSAttributedString(attachment: a))
        }
        let way = ways[safe: now] ?? 0
        if way != 0 { glyph(Glyph.draw(way > 0 ? Iconoir.arrowUp : Iconoir.arrowDown, 14), Theme.label, 14) }
        glyph(Glyph.draw(Iconoir.navArrowDown, 16), Theme.secondary, 16)
        order.setAttributedTitle(words, for: .normal)
        order.accessibilityLabel = "\(Say.sortBy) \(Say.sort(all[now]))"
    }

    private func seal() {
        guard let f = filter else { return }
        f.setImage(Glyph.draw(f.chosen ? Iconoir.seal : Iconoir.sealOutline, 16).tinted(f.chosen ? Theme.accent : Theme.secondary), for: .normal)
    }

    @objc private func choose() {
        let sheet = UIAlertController(title: Say.sortBy, message: nil, preferredStyle: .actionSheet)
        for (i, name) in all.enumerated() {
            let action = UIAlertAction(title: Say.sort(name), style: .default) { [weak self] _ in
                guard let self = self, i != self.now else { return }
                self.now = i
                name.withCString { nori_ios_sort(self.kind, $0) }
                self.name()
                self.changed()
            }
            action.setValue(i == now, forKey: "checked")
            sheet.addAction(action)
        }
        sheet.addAction(UIAlertAction(title: Say.cancel, style: .cancel))
        sheet.popoverPresentationController?.sourceView = order
        sheet.popoverPresentationController?.sourceRect = order.bounds
        asking(sheet)
    }

    @objc private func flip() {
        guard let f = filter else { return }
        f.chosen.toggle()
        f.layer.borderWidth = hairlineWidth
        seal()
        filtered(f.chosen)
    }

    @objc private func shuffled() { shuffle?() }
}
