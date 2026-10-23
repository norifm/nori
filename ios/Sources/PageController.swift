import UIKit

/// Any page the library reads: a list, shelves, a grid, or a collection under its hero; or, made with
/// `given`, sections already in hand (a shelf opened in full).
class PageController: UITableViewController {
    let kind: Int32
    let arg: String
    private let given: [Section]?
    private(set) var token: UInt64 = 0
    private(set) var answer: PageAnswer?
    /// Each table section: a list, a shelf (one row), or a grid (two prints a row).
    private var rows: [(section: Section, layout: Layout)] = []
    private var more = false
    private var nextPage = 0
    private var loadingMore = false
    /// The list runs in the order of a name: the table shows the A–Z index.
    private var letters = false
    /// An index letter not loaded yet: pages load until it is.
    private var wantedLetter: String?
    private static let index = ["#"] + "ABCDEFGHIJKLMNOPQRSTUVWXYZ".map(String.init)
    private let empty = EmptyState(title: "", detail: "")
    /// The colors the page wears: the theme's, or under a hero the season's paper dyed with its cover.
    private(set) var look = Theme.page
    /// What stands over the list on a page without a hero (the library's chips and places).
    var top: (() -> UIView)?

    enum Layout { case list, shelf, grid }

    /// Pages drawn under a hero with their cover and name.
    private static let headed: Set<Int32> = [NORI_PAGE_ALBUM, NORI_PAGE_DOWNLOADED_ALBUM, NORI_PAGE_ARTIST, NORI_PAGE_PLAYLIST, NORI_PAGE_MIX, NORI_PAGE_SMART]
    /// Pages whose prints stand in a grid rather than on shelves.
    private static let grids: Set<Int32> = [NORI_PAGE_ALBUMS, NORI_PAGE_GENRE, NORI_PAGE_HOME_ROW]
    /// Under a hero with its cover and name.
    var headed: Bool { PageController.headed.contains(kind) }

    init(kind: Int32, arg: String = "", title: String) {
        self.kind = kind
        self.arg = arg
        given = nil
        super.init(style: .plain)
        self.title = title
    }

    /// A page of `sections` in hand, its prints in a grid.
    init(title: String, sections: [Section]) {
        kind = 0
        arg = ""
        given = sections
        super.init(style: .plain)
        self.title = title
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        tableView.backgroundColor = look.page
        tableView.separatorStyle = .none
        tableView.register(ItemCell.self, forCellReuseIdentifier: ItemCell.id)
        tableView.register(ShelfCell.self, forCellReuseIdentifier: ShelfCell.id)
        tableView.register(PairCell.self, forCellReuseIdentifier: PairCell.id)
        tableView.estimatedRowHeight = 60
        tableView.rowHeight = UITableView.automaticDimension
        tableView.sectionHeaderHeight = UITableView.automaticDimension
        tableView.estimatedSectionHeaderHeight = 60
        tableView.tableFooterView = UIView()
        if navigationController?.viewControllers.first !== self {
            navigationItem.largeTitleDisplayMode = .never
        }
        // A page under its hero carries its name there; the bar above is empty and see-through.
        if headed {
            navigationItem.title = ""
            tableView.contentInsetAdjustmentBehavior = .never
            extendedLayoutIncludesOpaqueBars = true
        }
        if let top { tableView.tableHeaderView = sized(top()) }
        let refresh = UIRefreshControl()
        refresh.tintColor = Theme.secondary
        refresh.addTarget(self, action: #selector(reload), for: .valueChanged)
        refreshControl = given == nil && !headed ? refresh : nil
        empty.translatesAutoresizingMaskIntoConstraints = false
        empty.isHidden = true
        view.addSubview(empty)
        // The view is the table, a scroll view: its own edges are the content's, which has no width.
        let frame = tableView.frameLayoutGuide
        NSLayoutConstraint.activate([
            empty.leadingAnchor.constraint(equalTo: frame.leadingAnchor, constant: 32),
            empty.trailingAnchor.constraint(equalTo: frame.trailingAnchor, constant: -32),
            empty.centerYAnchor.constraint(equalTo: view.safeAreaLayoutGuide.centerYAnchor, constant: -40),
        ])
        NotificationCenter.default.addObserver(self, selector: #selector(reload), name: .noriOpened, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(nowChanged), name: .noriNow, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(favoritesChanged), name: .noriFavorites, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(paintVisibleRows), name: .noriJam, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(reload), name: .noriDownloads, object: nil)
        tableView.sectionIndexColor = Theme.secondary
        tableView.sectionIndexBackgroundColor = .clear
        tableView.addGestureRecognizer(UILongPressGestureRecognizer(target: self, action: #selector(held(_:))))
        reload()
    }

    /// `v` sized to the table's width, for a table header.
    private func sized(_ v: UIView) -> UIView {
        let width = UIScreen.main.bounds.width
        let size = v.systemLayoutSizeFitting(CGSize(width: width, height: UIView.layoutFittingCompressedSize.height),
                                             withHorizontalFittingPriority: .required, verticalFittingPriority: .fittingSizeLevel)
        v.frame = CGRect(x: 0, y: 0, width: width, height: size.height)
        return v
    }

    // A hero's bar is see-through, its ink white over the artwork's shade.
    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        guard headed, let bar = navigationController?.navigationBar else { return }
        bar.setBackgroundImage(UIImage(), for: .default)
        bar.isTranslucent = true
        bar.tintColor = hasPicture ? .white : look.ink
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        guard headed, let bar = navigationController?.navigationBar else { return }
        // The next page puts the plain bar back unless it is a hero too.
        if let next = navigationController?.topViewController as? PageController, next !== self, next.headed { return }
        Theme.apply(nav: bar)
    }

    override var preferredStatusBarStyle: UIStatusBarStyle {
        headed && hasPicture ? .lightContent : Theme.statusBar
    }

    /// Under its hero the list keeps its own insets: room at its foot for the chrome over it.
    override func viewSafeAreaInsetsDidChange() {
        super.viewSafeAreaInsetsDidChange()
        guard headed else { return }
        tableView.contentInset.bottom = view.safeAreaInsets.bottom
        tableView.scrollIndicatorInsets.bottom = view.safeAreaInsets.bottom
    }

    private var hasPicture: Bool { (tableView.tableHeaderView as? HeroHeader)?.picture != nil }

    override func scrollViewDidScroll(_ scrollView: UIScrollView) {
        (tableView.tableHeaderView as? HeroHeader)?.scrolled(scrollView.contentOffset.y)
    }

    /// A long press on a list row: the song menu, or the album's, artist's or playlist's.
    @objc private func held(_ g: UILongPressGestureRecognizer) {
        guard g.state == .began, let path = tableView.indexPathForRow(at: g.location(in: tableView)),
              rows[path.section].layout == .list else { return }
        let item = rows[path.section].section.items[path.row]
        guard item.kind == "song" else { return menu(for: item) }
        SongMenu.show(item, token: token, index: item.index, from: self) { [weak self] page in
            self?.navigationController?.pushViewController(page, animated: true)
        }
    }

    deinit {
        if token != 0 { Core.shared.forget(token) }
    }

    /// An answer came while the list was pulled down to refresh.
    private var releaseEndsRefresh = false

    /// Ends the pull-to-refresh, but only once the finger has let go: ended mid-drag (the stored copy
    /// answers at once), UIKit leaves the list held down by the control's height.
    private func finishRefresh() {
        guard let control = refreshControl, control.isRefreshing else { return }
        if tableView.isDragging {
            releaseEndsRefresh = true
        } else {
            control.endRefreshing()
        }
    }

    override func scrollViewDidEndDragging(_ scrollView: UIScrollView, willDecelerate decelerate: Bool) {
        guard releaseEndsRefresh else { return }
        releaseEndsRefresh = false
        refreshControl?.endRefreshing()
    }

    @objc func reload() {
        if let given {
            return took(PageAnswer(sections: given), appending: false)
        }
        guard Core.shared.isOpen else {
            finishRefresh()
            showEmpty(Say.noServer, Say.noServerDetail)
            return
        }
        if token != 0 { Core.shared.forget(token) }
        loadingMore = false
        wantedLetter = nil
        token = Core.shared.read(kind, arg) { [weak self] a in self?.took(a, appending: false) }
    }

    func took(_ a: PageAnswer, appending: Bool) {
        finishRefresh()
        loadingMore = false
        if a.raw["closed"] != nil {
            showEmpty(Say.noServer, Say.noServerDetail)
            return
        }
        if let code = a.error {
            if rows.isEmpty {
                let said = Say.pageFailure(code, a.detail)
                showEmpty(said.title, said.detail)
            }
            return
        }
        more = a.raw["more"] as? Bool ?? false
        nextPage = a.raw["next"] as? Int ?? 0
        letters = a.raw["letters"] as? Bool ?? false
        if appending, var extra = Optional(a.sections.filter { !$0.items.isEmpty }), !extra.isEmpty, let last = rows.indices.last {
            // A page can answer twice (cached, then the server's): it replaces what it sent before. Its first
            // section goes on with the last one shown when it is the same (an artist's shelf over a page's edge).
            let had = rows[last].section
            if extra[0].key == had.key && extra[0].title == had.title {
                let from = rows.count == 1 ? min(a.raw["from"] as? Int ?? had.items.count, had.items.count) : had.items.count
                rows[last] = (Section(key: had.key, grid: had.grid, items: Array(had.items.prefix(from)) + extra[0].items, title: had.title, raw: had.raw), rows[last].layout)
                extra.removeFirst()
            }
            rows += extra.map { ($0, layout(for: $0)) }
        } else {
            answer = a
            rows = a.sections.filter { !$0.items.isEmpty }.map { ($0, layout(for: $0)) }
            if !a.head.isEmpty { hero(a) }
        }
        if rows.isEmpty && a.head.isEmpty {
            showEmpty(Say.nothingHere, "")
        } else {
            empty.isHidden = true
        }
        paintedCurrent = Core.shared.now.song?.id
        tableView.reloadData()
        if let wanted = wantedLetter {
            wantedLetter = nil
            jump(to: wanted)
        }
    }

    // MARK: the hero

    /// The cover whose colors the page wears, measured once.
    private var measured: String?

    /// Builds the hero from the page's head, in the page's colors; asks for its cover's once.
    private func hero(_ a: PageAnswer) {
        let head = a.head
        let lead = (head["c"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? (head["covers"] as? [String])?.first
        if Theme.coversTint, let lead, measured != lead {
            measured = lead
            Core.shared.coverLook(lead) { [weak self] look in
                guard let self, let look else { return }
                self.wear(look)
            }
        }
        let header = HeroHeader(head: head, title: heroTitle(head), subtitle: head["k"] as? String == "artist" ? nil : (head["s"] as? String),
                                line: heroLine(head), look: look, own: ownGlyphs(head), actions: actionGlyphs(head))
        header.play = { [weak self] in self?.heroPressed(shuffle: false) }
        header.shuffle = { [weak self] in self?.heroPressed(shuffle: true) }
        let artist = head["artistId"] as? String ?? ""
        if !artist.isEmpty {
            header.subtitleTapped = { [weak self] in
                self?.navigationController?.pushViewController(PageController(kind: NORI_PAGE_ARTIST, arg: artist, title: head["s"] as? String ?? ""), animated: true)
            }
        }
        tableView.tableHeaderView = header
        header.paint(hero: heroBits())
        header.scrolled(tableView.contentOffset.y)
        navigationController?.navigationBar.tintColor = header.picture != nil ? .white : look.ink
        navigationController?.setNeedsStatusBarAppearanceUpdate()
    }

    /// The page's colors came: the page and its rows wear them.
    private func wear(_ l: PageLook) {
        guard l != look else { return }
        look = l
        tableView.backgroundColor = l.page
        if let a = answer { hero(a) }
        tableView.reloadData()
    }

    private func heroTitle(_ head: [String: Any]) -> String {
        switch head["k"] as? String {
        case "mix": return Say.mix(head["n"] as? Int ?? -1)
        case "smart":
            let named = head["t"] as? String ?? ""
            return named.isEmpty ? Say.smart(head["n"] as? Int ?? -1) : named
        default: return head["t"] as? String ?? title ?? ""
        }
    }

    /// The list head's words: "2019 · 8 songs · 36:28"; an artist's "14 releases · 162 songs".
    private func heroLine(_ head: [String: Any]) -> String {
        let n = head["n"] as? Int ?? 0
        let seconds = head["sec"] as? Int ?? 0
        switch head["k"] as? String {
        case "artist":
            let songs = head["albumSongs"] as? Int ?? 0
            let own = head["ns"] as? Int ?? 0
            let bits = [n > 0 ? Say.releases(n) : nil, songs > 0 ? Say.songs(songs) : nil].compactMap { $0 }
            return bits.isEmpty ? (own > 0 ? Say.songs(own) : "") : bits.joined(separator: " · ")
        case "mix", "smart":
            let count = head["count"] as? Int ?? 0
            return count > 0 ? Say.listCaption(count, seconds) : ""
        default:
            let year = head["y"] as? Int ?? 0
            return [year > 0 ? "\(year)" : nil, n > 0 ? Say.songsPlural(n) : nil, seconds > 0 ? Fmt.clock(ms: seconds * 1000) : nil]
                .compactMap { $0 }.joined(separator: " · ")
        }
    }

    /// After Play: the record's seal, or a mix's Save as playlist (the account's).
    private func ownGlyphs(_ head: [String: Any]) -> [UIView] {
        let kind = head["k"] as? String ?? ""
        let item = Item(head)
        guard Core.shared.rules.account, !item.external else { return [] }
        if kind == "album" || kind == "artist" {
            let on = Core.shared.isFavorite(item)
            let seal = GlyphButton(on ? Glyph.sealedPage : Glyph.sealPage, Say.favorite)
            seal.look = look
            seal.lit = on
            seal.addAction { [weak self] in
                Core.shared.favorite(item, !Core.shared.isFavorite(item))
                if let a = self?.answer { self?.hero(a) }
            }
            return [seal]
        }
        if kind == "mix", (head["count"] as? Int ?? 0) > 0 {
            let save = GlyphButton(Glyph.draw(Iconoir.plusCircle, 24), Say.saveAsPlaylist)
            save.look = look
            let name = heroTitle(head)
            save.addAction { [weak self] in
                guard let token = self?.token else { return }
                DispatchQueue.global(qos: .userInitiated).async {
                    let made = name.withCString { taken(nori_ios_save_list(token, $0)) }
                    DispatchQueue.main.async { Toast.show(made.map(Say.playlistCreated) ?? Say.failed) }
                }
            }
            return [save]
        }
        return []
    }

    /// The list head's glyphs, the core's (`nori_ios_page_actions`): download, add to the queue, a mix's
    /// new draw.
    private func actionGlyphs(_ head: [String: Any]) -> [UIView] {
        let refreshable: Int32 = head["refreshable"] as? Bool == true ? 1 : 0
        guard let lines = takenJSON(nori_ios_page_actions(token, kind, refreshable)) as? [[String: Int]] else { return [] }
        let (kind, arg, token) = (self.kind, self.arg, self.token)
        return lines.compactMap { line -> UIView? in
            let b: GlyphButton
            switch line["a"] {
            case 0:
                let act = Int32(line["act"] ?? 0)
                b = GlyphButton(Glyph.draw(act == 2 ? Iconoir.checkCircle : Iconoir.download, 24), Say.downloadEntry(Int(act), line["n"] ?? 0), quiet: true)
                b.addAction { PageController.download(act) { nori_ios_download_act(token, $0) } }
            case 1:
                b = GlyphButton(Glyph.draw(Iconoir.download, 24), Say.downloadEverything, quiet: true)
                b.addAction { PageController.download(0) { act in arg.withCString { nori_ios_collection_download_act(kind, $0, act) } } }
            case 2:
                b = GlyphButton(Glyph.queue, Say.addToQueue, quiet: true)
                b.addAction {
                    if kind == NORI_PAGE_ARTIST { arg.withCString { nori_ios_enqueue_collection(kind, $0, 0) } } else { nori_ios_enqueue_list(token, -1, 0) }
                }
            case 3:
                b = GlyphButton(Glyph.draw(Iconoir.refresh, 24), Say.newMix, quiet: true)
                b.addAction { [weak self] in
                    DispatchQueue.global(qos: .userInitiated).async {
                        arg.withCString { nori_ios_mix_refresh($0) }
                        DispatchQueue.main.async { self?.reload() }
                    }
                }
            default:
                return nil
            }
            b.look = look
            return b
        }
    }

    /// Runs download act `act` off the main thread; a removal says how many and has pages read again.
    static func download(_ act: Int32, run: @escaping (Int32) -> Int32) {
        DispatchQueue.global(qos: .userInitiated).async {
            let done = run(act)
            guard act == 2 else { return }
            DispatchQueue.main.async {
                Toast.show(Say.downloadsRemoved(Int(done)))
                NotificationCenter.default.post(name: .noriDownloads, object: nil)
            }
        }
    }

    // MARK: the list

    override func sectionIndexTitles(for tableView: UITableView) -> [String]? {
        letters && rows.count == 1 ? PageController.index : nil
    }

    override func tableView(_ tableView: UITableView, sectionForSectionIndexTitle title: String, at index: Int) -> Int {
        // The table first scrolls to the section's top; the row is found after it.
        DispatchQueue.main.async { self.jump(to: title) }
        return 0
    }

    /// Scrolls to the first row at or past `letter`, loading pages while it is not there yet.
    private func jump(to letter: String) {
        guard let r = rows.first else { return }
        let rank = { (l: String) in PageController.index.firstIndex(of: l) ?? 0 }
        let target = rank(letter)
        guard let at = r.section.items.firstIndex(where: { rank($0.letter) >= target }) else {
            if more {
                wantedLetter = letter
                loadMore()
            } else if let last = r.section.items.indices.last {
                scroll(toItem: last)
            }
            return
        }
        scroll(toItem: at)
    }

    private func scroll(toItem i: Int) {
        let row = rows[0].layout == .grid ? i / 2 : i
        tableView.scrollToRow(at: IndexPath(row: row, section: 0), at: .top, animated: false)
    }

    private func loadMore() {
        guard more, !loadingMore else { return }
        loadingMore = true
        let page = kind == NORI_PAGE_GENRE || kind == NORI_PAGE_HOME_ROW ? "\(arg)\n\(nextPage)" : "\(nextPage)"
        Core.shared.read(kind, page, token: token) { [weak self] a in self?.took(a, appending: true) }
    }

    private func layout(for s: Section) -> Layout {
        guard s.grid else { return .list }
        return (given != nil || PageController.grids.contains(kind)) && s.key != "artistRow" ? .grid : .shelf
    }

    private func showEmpty(_ title: String, _ detail: String) {
        rows = []
        tableView.reloadData()
        empty.set(title: title, detail: detail)
        empty.isHidden = false
    }

    /// The core's word on this page's Play and Shuffle against the queue now (`nori_ios_hero`).
    private func heroBits() -> Int32 {
        let now = Core.shared.now
        return arg.withCString { nori_ios_hero(kind, $0, now.playing ? 1 : 0, now.buffering ? 1 : 0) }
    }

    /// Play or Shuffle pressed: start the page's songs, or (its queue playing) pause and resume it or turn
    /// its shuffle off or on, as the core says.
    private func heroPressed(shuffle: Bool) {
        let press = (heroBits() >> (shuffle ? 4 : 6)) & 3
        switch press {
        case 1: nori_ios_toggle()
        case 2: nori_ios_shuffle(0)
        case 3: nori_ios_shuffle(1)
        default: play(shuffle: shuffle)
        }
    }

    /// The song the rows were last painted with as playing.
    private var paintedCurrent: String?

    @objc private func nowChanged() {
        (tableView.tableHeaderView as? HeroHeader)?.paint(hero: heroBits())
        let id = Core.shared.now.song?.id
        guard id != paintedCurrent else { return }
        paintedCurrent = id
        paintVisibleRows()
    }

    @objc private func favoritesChanged() {
        if kind == NORI_PAGE_STARRED || kind == NORI_PAGE_HOME {
            reload()
        } else {
            paintVisibleRows()
        }
    }

    /// The visible song rows painted again in place (their playing mark, seal and "Asked"); a reload would
    /// close a row swiped open.
    @objc private func paintVisibleRows() {
        for path in tableView.indexPathsForVisibleRows ?? [] {
            guard rows.indices.contains(path.section), rows[path.section].layout == .list,
                  let cell = tableView.cellForRow(at: path) as? ItemCell else { continue }
            let item = rows[path.section].section.items[path.row]
            cell.show(item, numbered: numbered, current: isCurrent(item), look: look)
        }
    }

    private var numbered: Bool { kind == NORI_PAGE_ALBUM || kind == NORI_PAGE_DOWNLOADED_ALBUM }

    func play(shuffle: Bool) {
        if kind == NORI_PAGE_ARTIST {
            arg.withCString { nori_ios_play_collection(kind, $0, shuffle ? 1 : 0) }
        } else {
            nori_ios_play_list(token, 0, shuffle ? 1 : 0)
        }
    }

    override func numberOfSections(in tableView: UITableView) -> Int { rows.count }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        let r = rows[section]
        switch r.layout {
        case .list: return r.section.items.count
        case .shelf: return 1
        case .grid: return (r.section.items.count + 1) / 2
        }
    }

    /// A section's title: none for a page's one list; a shelf's, the row's words, or a source's own.
    private func heading(_ section: Int) -> String? {
        let s = rows[section].section
        if rows.count == 1 && ((headed && !s.grid) || given != nil || (top != nil && s.key != "artistRow")) { return nil }
        if rows.count == 1 && kind != NORI_PAGE_HOME && kind != NORI_PAGE_DOWNLOADS && kind != NORI_PAGE_ALBUMS && kind != NORI_PAGE_ARTIST { return nil }
        if s.key == "release" { return Say.release(s.raw["kind"] as? Int ?? 0, s.raw["tag"] as? String ?? "") }
        let words = s.title.isEmpty ? Say.shelf(s.key) : s.title
        return words.isEmpty ? nil : words
    }

    /// What a section's arrow opens: a Home row in full, an artist's shelf as a grid, an artist's albums.
    private func whole(_ section: Int) -> (() -> Void)? {
        let s = rows[section].section
        let words = heading(section) ?? ""
        let push = { [weak self] (page: UIViewController) in self?.navigationController?.pushViewController(page, animated: true) }
        switch s.key {
        case "artistRow":
            let id = s.raw["artistId"] as? String ?? ""
            return id.isEmpty ? nil : { push(PageController(kind: NORI_PAGE_ARTIST, arg: id, title: words)) }
        case "release", "mixes", "feed":
            return { push(PageController(title: words, sections: [Section(key: s.key, grid: true, items: s.items)])) }
        default:
            guard kind == NORI_PAGE_HOME, s.grid || s.key == "topSongs" else { return nil }
            return { push(PageController(kind: NORI_PAGE_HOME_ROW, arg: s.key, title: words)) }
        }
    }

    override func tableView(_ tableView: UITableView, viewForHeaderInSection section: Int) -> UIView? {
        guard let words = heading(section) else { return nil }
        let header = tableView.dequeueReusableHeaderFooterView(withIdentifier: SectionHeading.id) as? SectionHeading
            ?? SectionHeading(reuseIdentifier: SectionHeading.id)
        header.title.text.text = words
        header.title.text.textColor = look.ink
        header.title.more = whole(section)
        return header
    }

    override func tableView(_ tableView: UITableView, heightForHeaderInSection section: Int) -> CGFloat {
        heading(section) == nil ? 0 : UITableView.automaticDimension
    }

    override func tableView(_ tableView: UITableView, heightForRowAt indexPath: IndexPath) -> CGFloat {
        let r = rows[indexPath.section]
        switch r.layout {
        case .list: return UITableView.automaticDimension
        case .shelf: return ShelfCell.height(r.section.items, side: shelfSide(r.section)) + Space.tight
        case .grid: return PairCell.height(mixes: r.section.items.first?.kind == "mix")
        }
    }

    private func shelfSide(_ s: Section) -> CGFloat { s.key == "artistRow" ? ShelfCell.small : ShelfCell.print }

    func isCurrent(_ item: Item) -> Bool {
        guard let song = Core.shared.now.song else { return false }
        return item.kind == "song" && item.id == song.id
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let r = rows[indexPath.section]
        switch r.layout {
        case .shelf:
            let cell = tableView.dequeueReusableCell(withIdentifier: ShelfCell.id, for: indexPath) as! ShelfCell
            cell.show(r.section.items, side: shelfSide(r.section), look: look)
            cell.picked = { [weak self] in self?.open($0) }
            cell.held = { [weak self] in self?.menu(for: $0) }
            return cell
        case .grid:
            let cell = tableView.dequeueReusableCell(withIdentifier: PairCell.id, for: indexPath) as! PairCell
            let i = indexPath.row * 2
            cell.show(r.section.items[i], i + 1 < r.section.items.count ? r.section.items[i + 1] : nil, look: look)
            cell.picked = { [weak self] in self?.open($0) }
            cell.held = { [weak self] in self?.menu(for: $0) }
            return cell
        case .list:
            let cell = tableView.dequeueReusableCell(withIdentifier: ItemCell.id, for: indexPath) as! ItemCell
            let item = r.section.items[indexPath.row]
            cell.show(item, numbered: numbered, current: isCurrent(item), look: look)
            return cell
        }
    }

    override func tableView(_ tableView: UITableView, willDisplay cell: UITableViewCell, forRowAt indexPath: IndexPath) {
        guard more, indexPath.section == rows.count - 1,
              indexPath.row >= tableView.numberOfRows(inSection: indexPath.section) - 10 else { return }
        loadMore()
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        let item = rows[indexPath.section].section.items[indexPath.row]
        open(item)
    }

    func open(_ item: Item) {
        let push = { (page: UIViewController) in self.navigationController?.pushViewController(page, animated: true) }
        switch item.kind {
        case "song":
            if nori_ios_tap_song(token, Int32(item.index)) == 1 {
                (view.window?.rootViewController as? ShellController)?.openPlayer()
            }
        case "downloadedAlbum": push(PageController(kind: NORI_PAGE_DOWNLOADED_ALBUM, arg: item.id, title: item.title))
        case "album": push(PageController(kind: NORI_PAGE_ALBUM, arg: item.id, title: item.title))
        case "artist": push(PageController(kind: NORI_PAGE_ARTIST, arg: item.id, title: item.title))
        case "playlist": push(PageController(kind: NORI_PAGE_PLAYLIST, arg: item.id, title: item.title))
        case "genre": push(PageController(kind: NORI_PAGE_GENRE, arg: item.id, title: item.title))
        case "mix": push(PageController(kind: NORI_PAGE_MIX, arg: item.id, title: Say.mix(item.count)))
        case "smart":
            let name = item.title.isEmpty ? Say.smart(item.count) : item.title
            push(PageController(kind: NORI_PAGE_SMART, arg: item.id, title: name))
        default:
            break
        }
    }

    override func tableView(_ tableView: UITableView, trailingSwipeActionsConfigurationForRowAt indexPath: IndexPath) -> UISwipeActionsConfiguration? {
        swipe(left: true, at: indexPath)
    }

    override func tableView(_ tableView: UITableView, leadingSwipeActionsConfigurationForRowAt indexPath: IndexPath) -> UISwipeActionsConfiguration? {
        swipe(left: false, at: indexPath)
    }

    /// The one action a sideways swipe on a song row offers: the swipeLeft or swipeRight setting's, or on a
    /// playlist's own page Remove to the left. Never nil, which would give UIKit's own Delete.
    private func swipe(left: Bool, at indexPath: IndexPath) -> UISwipeActionsConfiguration {
        let none = UISwipeActionsConfiguration(actions: [])
        let r = rows[indexPath.section]
        guard r.layout == .list else { return none }
        let item = r.section.items[indexPath.row]
        guard item.kind == "song" else { return none }
        let index = Int32(item.index)
        let action: UIContextualAction
        if left && kind == NORI_PAGE_PLAYLIST {
            let playlist = arg
            action = UIContextualAction(style: .destructive, title: Say.removeFromPlaylist) { [weak self] _, _, done in
                DispatchQueue.global(qos: .userInitiated).async {
                    let ok = playlist.withCString { nori_ios_playlist_remove($0, index) } == 1
                    DispatchQueue.main.async {
                        done(ok)
                        if ok { self?.reload() } else { Toast.show(Say.failed) }
                    }
                }
            }
        } else {
            let code = nori_ios_row_swipe(left ? 1 : 0, Core.shared.isFavorite(item) ? 1 : 0)
            let starring = code == NORI_SWIPE_FAVORITE || code == NORI_SWIPE_UNFAVORITE
            guard code >= 0, !(item.external && (starring || code == NORI_SWIPE_DOWNLOAD)) else { return none }
            let title: String
            switch code {
            case NORI_SWIPE_QUEUE: title = Say.addToQueue
            case NORI_SWIPE_PLAY_NEXT: title = Say.playNext
            case NORI_SWIPE_FAVORITE: title = Say.favorite
            case NORI_SWIPE_UNFAVORITE: title = Say.unfavorite
            default: title = Say.download
            }
            action = UIContextualAction(style: .normal, title: title) { [weak self] _, _, done in
                guard let self else { return done(false) }
                switch code {
                case NORI_SWIPE_QUEUE, NORI_SWIPE_PLAY_NEXT:
                    nori_ios_enqueue_list(self.token, index, code == NORI_SWIPE_PLAY_NEXT ? 1 : 0)
                case NORI_SWIPE_FAVORITE, NORI_SWIPE_UNFAVORITE:
                    Core.shared.favorite(item, code == NORI_SWIPE_FAVORITE)
                    self.tableView.reloadRows(at: [indexPath], with: .none)
                default:
                    nori_ios_download_list(self.token, index)
                }
                done(true)
            }
            action.backgroundColor = look.veil(0.5)
        }
        return UISwipeActionsConfiguration(actions: [action])
    }

    /// A long press on a print: what to do with the whole album, artist or playlist.
    func menu(for item: Item) {
        let kind: Int32
        switch item.kind {
        case "downloadedAlbum": kind = NORI_PAGE_DOWNLOADED_ALBUM
        case "album": kind = NORI_PAGE_ALBUM
        case "artist": kind = NORI_PAGE_ARTIST
        case "playlist": kind = NORI_PAGE_PLAYLIST
        default: return
        }
        // The download entries need the songs (stored, else the server's): read off the main thread.
        DispatchQueue.global(qos: .userInitiated).async {
            let raw = item.external ? nil : item.id.withCString { nori_ios_collection_download_entries(kind, $0) }
            let entries = takenJSON(raw) as? [[String: Int]] ?? []
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                let sheet = Sheet(item.title, item.subtitle.isEmpty ? nil : item.subtitle, cover: item.cover)
                if !Core.shared.rules.asks {
                    sheet.add(Say.play, glyph: Glyph.draw(Iconoir.play, 20)) { item.id.withCString { nori_ios_play_collection(kind, $0, 0) } }
                }
                sheet.add(Say.shuffle, glyph: Glyph.draw(Iconoir.shuffle, 20)) { item.id.withCString { nori_ios_play_collection(kind, $0, 1) } }
                sheet.add(Say.playNext, glyph: Glyph.draw(Iconoir.playlistPlay, 20)) { item.id.withCString { nori_ios_enqueue_collection(kind, $0, 1) } }
                sheet.add(Say.addToQueue, glyph: Glyph.draw(Iconoir.playlist, 20)) { item.id.withCString { nori_ios_enqueue_collection(kind, $0, 0) } }
                PageController.add(entries, to: sheet) { act in
                    item.id.withCString { nori_ios_collection_download_act(kind, $0, act) }
                }
                if kind != NORI_PAGE_PLAYLIST && !item.external && Core.shared.rules.account {
                    let on = Core.shared.isFavorite(item)
                    sheet.add(on ? Say.removeFromFavorites : Say.addToFavorites, glyph: Glyph.draw(on ? Iconoir.seal : Iconoir.sealOutline, 20)) {
                        Core.shared.favorite(item, !on)
                    }
                }
                if kind == NORI_PAGE_PLAYLIST {
                    sheet.add(Say.deleteNamed(item.title), glyph: Glyph.draw(Iconoir.trash, 20), destructive: true) { [weak self] in self?.delete(playlist: item) }
                }
                sheet.show(from: self)
            }
        }
    }

    /// Download entries (`nori_ios_download_entries`) as lines of `sheet`; `run` does one off the main
    /// thread and answers the songs it touched. A removal says how many and has pages read again.
    static func add(_ entries: [[String: Int]], to sheet: Sheet, run: @escaping (Int32) -> Int32) {
        // Two entries: partly downloaded, so removal names how many it gives back.
        let partly = entries.count > 1
        for e in entries {
            guard let act = e["act"], let n = e["n"] else { continue }
            let words = act == 0 ? Say.download : act == 1 ? Say.downloadOther(n) : (partly ? Say.removeDownloaded(n) : Say.removeDownloads)
            sheet.add(words, glyph: Glyph.draw(act == 2 ? Iconoir.trash : Iconoir.download, 20), destructive: act == 2) {
                download(Int32(act), run: run)
            }
        }
    }

    /// Asks, then deletes `playlist` on the server and reads this page again.
    private func delete(playlist: Item) {
        let ask = Sheet(Say.deleteNamed(playlist.title) + "?")
        ask.add(Say.deleteNamed(playlist.title), glyph: Glyph.draw(Iconoir.trash, 20), destructive: true) { [weak self] in
            DispatchQueue.global(qos: .userInitiated).async {
                let ok = playlist.id.withCString { nori_ios_playlist_delete($0) } == 1
                DispatchQueue.main.async {
                    if ok { self?.reload() } else { Toast.show(Say.failed) }
                }
            }
        }
        ask.show(from: self)
    }
}

extension UIControl {
    /// Runs `run` on a tap; the closure is kept with the control.
    func addAction(_ run: @escaping () -> Void) {
        let target = Action(run)
        objc_setAssociatedObject(self, &Action.key, target, .OBJC_ASSOCIATION_RETAIN)
        addTarget(target, action: #selector(Action.fire), for: .touchUpInside)
    }
}

private final class Action: NSObject {
    static var key = 0
    private let run: () -> Void
    init(_ run: @escaping () -> Void) { self.run = run }
    @objc func fire() { run() }
}
