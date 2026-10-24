import UIKit

/// The library, as Android's: a strip of chips for its sections and the one chosen below it, with the places
/// it keeps (Favorites, Downloads and Shuffle) on a card at the top, and the list's orders as pills.
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

    /// The strip of chips, the places and, for a list with orders, its pills: the albums' after a
    /// Favorites chip where the profile has favorites.
    private func head(chosen: Int, page: PageController) -> UIView {
        let kind = page.kind
        let strip = ChipStrip(spacing: 6)
        strip.set(chips.map { $0.0 }, chosen: chosen)
        strip.picked = { [weak self] i in self?.show(i) }
        var parts: [UIView] = [strip]
        if let places = places(kind: kind) { parts.append(places) }
        let favorites = kind == NORI_PAGE_ALBUMS && Core.shared.rules.stars ? page.arg == NORI_ALBUMS_FAVORITES : nil
        let filtered = { [weak page] (on: Bool) in
            page?.arg = on ? NORI_ALBUMS_FAVORITES : ""
            page?.reload()
        }
        if let pills = SortPills(kind: kind, favorites: favorites, filtered: filtered, changed: { [weak self] in self?.shown?.reload() }).made { parts.append(pills) }
        let column = UIStackView(arrangedSubviews: parts)
        column.axis = .vertical
        column.spacing = 10
        column.isLayoutMarginsRelativeArrangement = true
        column.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 4, leading: 0, bottom: 10, trailing: 0)
        return column
    }

    /// The places the library keeps, not kinds of music, ending in Shuffle where the list shown has one (the
    /// account's own library only).
    private func places(kind: Int32) -> UIView? {
        let rules = Core.shared.rules
        var rows: [GroupRow] = []
        if rules.sections.contains(1) {
            rows.append(GroupRow(Say.favorites, icon: Glyph.draw(Iconoir.sealOutline, 20), chevron: true) { [weak self] in
                self?.navigationController?.pushViewController(PageController(kind: NORI_PAGE_STARRED, title: Say.favorites), animated: true)
            })
        }
        if rules.sections.contains(11) {
            let downloads = GroupRow(Say.downloads, icon: Glyph.draw(Iconoir.download, 20), chevron: true) { [weak self] in
                self?.navigationController?.pushViewController(DownloadsPage(), animated: true)
            }
            rows.append(downloads)
            DispatchQueue.global(qos: .utility).async {
                let bytes = (takenJSON(nori_ios_facts()) as? [String: Any])?["downloadBytes"] as? Int ?? 0
                DispatchQueue.main.async {
                    downloads.value.text = Fmt.bytes(bytes)
                    downloads.value.isHidden = false
                }
            }
        }
        // The smart playlists are under Playlists.
        if kind == NORI_PAGE_PLAYLISTS && rules.sections.contains(5) {
            rows.append(GroupRow(Say.smartPlaylists, icon: Glyph.draw(Iconoir.sparks, 20), chevron: true) { [weak self] in
                self?.navigationController?.pushViewController(PageController(kind: NORI_PAGE_SMARTS, title: Say.smartPlaylists), animated: true)
            })
        }
        if rules.account && (kind == NORI_PAGE_ALBUMS || kind == NORI_PAGE_SONGS) {
            let albums: Int32 = kind == NORI_PAGE_ALBUMS ? 1 : 0
            rows.append(GroupRow(Say.shuffle, icon: Glyph.draw(Iconoir.shuffle, 20)) {
                DispatchQueue.global(qos: .userInitiated).async {
                    if nori_ios_shuffle_library(albums) == 0 {
                        DispatchQueue.main.async { Toast.show(Say.note(NORI_NOTE_NOTHING_TO_PLAY, 0) ?? Say.failed) }
                    }
                }
            })
        }
        return rows.isEmpty ? nil : GroupCard(rows)
    }
}

/// A long list's orders as pills (`nori_ios_sorts`: the core's, in its order): the one it is in lit, a tap
/// chooses another and the list reads again. With `favorites`, a Favorites chip before them, on or off,
/// that `filtered` follows.
struct SortPills {
    let made: ChipStrip?

    init(kind: Int32, favorites: Bool? = nil, filtered: @escaping (Bool) -> Void = { _ in }, changed: @escaping () -> Void) {
        guard let d = takenJSON(nori_ios_sorts(kind)) as? [String: Any], let all = d["all"] as? [String] else {
            made = nil
            return
        }
        let strip = ChipStrip()
        strip.set(all.map(Say.sort), chosen: all.firstIndex { $0 == d["now"] as? String } ?? 0, filter: favorites.map { (Say.favorites, $0) })
        strip.toggled = filtered
        strip.picked = { i in
            all[i].withCString { nori_ios_sort(kind, $0) }
            changed()
        }
        made = strip
    }
}
