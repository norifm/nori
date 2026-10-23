import AVFoundation
import UIKit

/// Home, Library, Settings and Search, as on Android: each a navigation stack, with the bottom chrome (the
/// mini player over the tabs, one outlined pill) floating over them; the pages scroll under it.
final class ShellController: UIViewController {
    private(set) var tabs: [UINavigationController] = []
    private(set) var selectedIndex = 0
    let chrome = BottomChrome()

    /// The tab shown.
    var current: UINavigationController { tabs[selectedIndex] }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Theme.background
        tabs = [
            tab(Say.home, HomePage()),
            tab(Say.library, LibraryPage()),
            tab(Say.settingsTitle, SettingsPage()),
            tab(Say.search, SearchPage()),
        ]
        chrome.tabs.picked = { [weak self] i in self?.select(i) }
        chrome.mini.opened = { [weak self] in self?.openPlayer() }
        chrome.mini.dragged = { [weak self] in self?.dragPlayer($0) }
        chrome.mini.devicesOpened = { [weak self] in self?.present(Core.shared.now.jam ? QueueSheet() : DevicesSheet(), animated: true) }
        chrome.resized = { [weak self] in self?.view.setNeedsLayout() }
        view.addSubview(chrome)
        lay()
        select(0)
        NotificationCenter.default.addObserver(self, selector: #selector(lay), name: .noriOpened, object: nil)
    }

    /// The tabs the open profile has: Settings where it opens any (a jam guest's: this iPod's own).
    @objc private func lay() {
        let settings = !Core.shared.rules.settings.isEmpty
        chrome.tabs.shown(settings ? [0, 1, 2, 3] : [0, 1, 3])
        if !settings && selectedIndex == 2 { select(0) }
    }

    /// Shows tab `i`; the tab already shown goes back to its first page.
    func select(_ i: Int) {
        guard tabs.indices.contains(i) else { return }
        if i == selectedIndex, current.parent != nil {
            current.popToRootViewController(animated: true)
            return
        }
        let old = current
        old.willMove(toParent: nil)
        old.view.removeFromSuperview()
        old.removeFromParent()
        selectedIndex = i
        let nav = current
        addChild(nav)
        nav.view.frame = view.bounds
        nav.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.insertSubview(nav.view, belowSubview: chrome)
        nav.didMove(toParent: self)
        chrome.tabs.chosen = i
        setNeedsStatusBarAppearanceUpdate()
    }

    override var childForStatusBarStyle: UIViewController? { current }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        let height = chrome.height
        chrome.frame = CGRect(x: BottomChrome.side, y: view.bounds.height - BottomChrome.below - height,
                              width: view.bounds.width - BottomChrome.side * 2, height: height)
        // The pages end above the chrome; what they hold scrolls under it.
        for nav in tabs { nav.additionalSafeAreaInsets.bottom = height + BottomChrome.below }
    }

    func openPlayer() {
        guard Core.shared.now.song != nil else { return }
        present(PlayerCard(), animated: true)
    }

    /// The card's opening while a finger drags the mini player up.
    private var rising: UIPercentDrivenInteractiveTransition?

    /// Follows a drag up from the mini player: the card's top under the finger, then open or back down
    /// as far and fast as it went.
    private func dragPlayer(_ g: UIPanGestureRecognizer) {
        let height = view.bounds.height
        let lift = -g.translation(in: view).y
        switch g.state {
        case .began:
            guard Core.shared.now.song != nil, presentedViewController == nil else { return }
            let card = PlayerCard()
            let drive = UIPercentDrivenInteractiveTransition()
            card.transition.interaction = drive
            rising = drive
            present(card, animated: true)
        case .changed:
            rising?.update(min(1, max(0, lift / height)))
        case .ended, .cancelled:
            guard let drive = rising else { return }
            rising = nil
            let opens = g.state == .ended && (lift > height * 0.3 || g.velocity(in: view).y < -800)
            if opens { drive.finish() } else { drive.cancel() }
        default:
            break
        }
    }

    private func tab(_ title: String, _ root: UIViewController) -> UINavigationController {
        root.title = title
        let nav = NavController(rootViewController: root)
        Theme.apply(nav: nav.navigationBar)
        nav.view.backgroundColor = Theme.background
        return nav
    }
}

/// A tab's stack: the status bar is its bar's, but over a page's artwork that page's own.
final class NavController: UINavigationController {
    override var childForStatusBarStyle: UIViewController? {
        (topViewController as? PageController)?.headed == true ? topViewController : nil
    }
}

/// The chrome at the foot of the screen: one outline holding the mini player (while something plays), a
/// hairline, and the tabs; the page shows faintly through it. No blur: the iPod draws it as a flat plate.
final class BottomChrome: UIView {
    /// Its margins from the screen's sides and foot.
    static let side: CGFloat = 10
    static let below: CGFloat = 6
    private static let tabsHeight: CGFloat = 50
    /// How far up from the screen's foot the chrome reaches at most: what a message keeps clear of.
    static var reach: CGFloat { below + tabsHeight + MiniPlayer.height }

    let mini = MiniPlayer()
    let tabs = TabRow()
    private let divider = UIView()
    var resized: (() -> Void)?

    var height: CGFloat { mini.isHidden ? BottomChrome.tabsHeight : BottomChrome.tabsHeight + MiniPlayer.height }

    override init(frame: CGRect) {
        super.init(frame: frame)
        let page = Theme.page
        backgroundColor = page.page.withAlphaComponent(0.94)
        layer.borderWidth = hairlineWidth
        layer.borderColor = page.ink.withAlphaComponent(0.2).cgColor
        clipsToBounds = true
        divider.backgroundColor = page.ink.withAlphaComponent(0.09)
        for v in [mini, divider, tabs] { addSubview(v) }
        mini.shown = { [weak self] in
            self?.setNeedsLayout()
            self?.resized?()
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    override func layoutSubviews() {
        super.layoutSubviews()
        let w = bounds.width
        mini.frame = CGRect(x: 0, y: 0, width: w, height: mini.isHidden ? 0 : MiniPlayer.height)
        divider.isHidden = mini.isHidden
        divider.frame = CGRect(x: 0, y: mini.frame.maxY, width: w, height: hairlineWidth)
        tabs.frame = CGRect(x: 0, y: mini.frame.maxY, width: w, height: BottomChrome.tabsHeight)
        // A pill alone; with the mini player, the cover's corner and its inset.
        layer.cornerRadius = mini.isHidden ? bounds.height / 2 : 18
    }
}

/// The tabs, each an equal share of the row: a glyph, quiet until it is the one shown, which takes the
/// accent. The names are for VoiceOver.
final class TabRow: UIView {
    private let glyphs = [Glyph.home, Glyph.library, Glyph.settings, Glyph.search]
    private let names = [Say.home, Say.library, Say.settingsTitle, Say.search]
    private var buttons: [UIButton] = []
    private var order: [Int] = []
    var picked: ((Int) -> Void)?
    var chosen = 0 { didSet { paint() } }

    override init(frame: CGRect) {
        super.init(frame: frame)
        buttons = glyphs.enumerated().map { i, g in
            let b = UIButton(type: .custom)
            b.setImage(g, for: .normal)
            b.tag = i
            b.accessibilityLabel = names[i]
            b.addTarget(self, action: #selector(tap(_:)), for: .touchUpInside)
            addSubview(b)
            return b
        }
        order = Array(buttons.indices)
        paint()
    }

    required init?(coder: NSCoder) { fatalError() }

    /// The tabs offered, by index.
    func shown(_ tabs: [Int]) {
        order = tabs
        for (i, b) in buttons.enumerated() { b.isHidden = !tabs.contains(i) }
        setNeedsLayout()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let inset: CGFloat = 6
        let each = (bounds.width - inset * 2) / CGFloat(max(1, order.count))
        for (n, i) in order.enumerated() {
            buttons[i].frame = CGRect(x: inset + each * CGFloat(n), y: 0, width: each, height: bounds.height)
        }
    }

    private func paint() {
        for (i, b) in buttons.enumerated() {
            b.tintColor = i == chosen ? Theme.accent : Theme.secondary
            b.accessibilityTraits = i == chosen ? [.button, .selected] : .button
        }
    }

    @objc private func tap(_ b: UIButton) { picked?(b.tag) }
}

/// What plays: the cover, the song, where the sound goes, the seal and play/pause. Tap, or drag it up, for
/// the card; swipe it left or right for the next or previous song. No progress line: it would tick.
final class MiniPlayer: UIView {
    static let height: CGFloat = 56

    var opened: (() -> Void)?
    var dragged: ((UIPanGestureRecognizer) -> Void)?
    /// The output was tapped with remote control on, or the jam in its place: the devices, or the queue
    /// where the jam is.
    var devicesOpened: (() -> Void)?
    /// It came or went with the song.
    var shown: (() -> Void)?
    private let cover = CoverView()
    private let title = label(Fonts.font(16), Theme.label)
    private let artist = label(Fonts.font(13, .regular, .footnote), Theme.secondary)
    private let output = UIButton(type: .custom)
    private let outputs = OutputPicker()
    private let seal = UIButton(type: .custom)
    private let play = UIButton(type: .custom)
    /// Which way the drag under way goes, decided as it starts: up for the card, sideways to skip.
    private var sideways = false
    /// The cover and the song's lines: what follows a sideways swipe.
    private var sliding: [UIView] = []

    override init(frame: CGRect) {
        super.init(frame: frame)
        let text = UIStackView(arrangedSubviews: [title, artist])
        text.axis = .vertical
        text.spacing = 1
        cover.layer.cornerRadius = 10
        play.tintColor = Theme.label
        play.addTarget(self, action: #selector(toggle), for: .touchUpInside)
        output.addTarget(self, action: #selector(outputTapped), for: .touchUpInside)
        outputs.attach(to: output)
        seal.accessibilityLabel = Say.favorite
        seal.addTarget(self, action: #selector(sealTapped), for: .touchUpInside)
        sliding = [cover, text]
        // The song's side, clipped where the controls start: a swiped song leaves at that edge rather
        // than passing under the controls.
        let lane = UIView()
        lane.clipsToBounds = true
        lane.isUserInteractionEnabled = false
        for v in [lane, output, seal, play] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
        }
        for v in [cover, text] {
            v.translatesAutoresizingMaskIntoConstraints = false
            lane.addSubview(v)
        }
        NSLayoutConstraint.activate([
            lane.leadingAnchor.constraint(equalTo: leadingAnchor),
            lane.trailingAnchor.constraint(equalTo: output.leadingAnchor),
            lane.topAnchor.constraint(equalTo: topAnchor),
            lane.bottomAnchor.constraint(equalTo: bottomAnchor),
            cover.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            cover.centerYAnchor.constraint(equalTo: centerYAnchor),
            cover.widthAnchor.constraint(equalToConstant: 40),
            cover.heightAnchor.constraint(equalToConstant: 40),
            text.leadingAnchor.constraint(equalTo: cover.trailingAnchor, constant: 11),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
            text.trailingAnchor.constraint(lessThanOrEqualTo: output.leadingAnchor, constant: -2),
            output.trailingAnchor.constraint(equalTo: seal.leadingAnchor),
            seal.trailingAnchor.constraint(equalTo: play.leadingAnchor),
            play.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -4),
        ])
        for b in [output, seal, play] {
            NSLayoutConstraint.activate([
                b.centerYAnchor.constraint(equalTo: centerYAnchor),
                b.widthAnchor.constraint(equalToConstant: 40),
                b.heightAnchor.constraint(equalToConstant: 48),
            ])
        }
        addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(tapped)))
        addGestureRecognizer(UIPanGestureRecognizer(target: self, action: #selector(panned(_:))))
        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(changed), name: .noriNow, object: nil)
        center.addObserver(self, selector: #selector(changed), name: .noriJam, object: nil)
        center.addObserver(self, selector: #selector(paintSeal), name: .noriFavorites, object: nil)
        center.addObserver(self, selector: #selector(changed), name: UIApplication.didBecomeActiveNotification, object: nil)
        center.addObserver(self, selector: #selector(paintOutput), name: AVAudioSession.routeChangeNotification, object: nil)
        changed()
    }

    required init?(coder: NSCoder) { fatalError() }

    @objc private func changed() {
        let now = Core.shared.now
        let hidden = now.song == nil
        if isHidden != hidden {
            isHidden = hidden
            shown?()
        }
        guard let song = now.song else { return }
        title.text = song.title
        artist.text = song.subtitle
        artist.isHidden = song.subtitle.isEmpty
        cover.show(song.cover, points: 40)
        // A jam guest plays as its role lets it (the core's jam controls); its skip is a swipe.
        let jam = now.jam ? Core.shared.jam : nil
        play.isHidden = now.jam && (jam?.play ?? 0) == 0
        paintOutput()
        paintSeal()
        let playing = jam?.playing ?? now.playing
        play.setImage(playing ? Glyph.pause : Glyph.play, for: .normal)
        play.accessibilityLabel = playing ? Say.pause : Say.play
    }

    /// The output button: the glyph of what plays (another device's, lit), or the jam in its place.
    @objc private func paintOutput() {
        let now = Core.shared.now
        if now.jam {
            let listened = (Core.shared.jam?.listeners.count ?? 0) > 0
            output.setImage(Glyph.jam, for: .normal)
            output.tintColor = listened ? Theme.accent : Theme.secondary
            output.accessibilityLabel = Core.shared.jam?.strip ?? Say.jamGuest
        } else if let device = now.device {
            output.setImage(Glyph.device(now.deviceKind, 22), for: .normal)
            output.tintColor = Theme.label
            output.accessibilityLabel = Say.playingOn(device)
        } else {
            let look = OutputLook.now()
            output.setImage(Glyph.output(look.glyph, 22), for: .normal)
            output.tintColor = Theme.secondary
            output.accessibilityLabel = Say.outputDescription(port: look.port, name: look.name)
        }
    }

    @objc private func toggle() {
        nori_ios_toggle()
    }

    /// The seal where songs carry favorites, in the accent when the song is one.
    @objc private func paintSeal() {
        guard let song = Core.shared.now.song, !song.external, Core.shared.rules.stars, !Core.shared.now.jam else {
            seal.isHidden = true
            return
        }
        seal.isHidden = false
        let on = Core.shared.isFavorite(song)
        seal.setImage(on ? Glyph.sealedSmall : Glyph.sealSmall, for: .normal)
        seal.tintColor = on ? Theme.accent : Theme.secondary
        seal.accessibilityTraits = on ? [.button, .selected] : .button
    }

    @objc private func sealTapped() {
        guard let song = Core.shared.now.song else { return }
        Core.shared.favorite(song, !Core.shared.isFavorite(song))
        paintSeal()
    }

    @objc private func outputTapped() {
        if Core.shared.now.remote || Core.shared.now.jam { devicesOpened?() } else { outputs.show() }
    }

    @objc private func tapped() {
        opened?()
    }

    @objc private func panned(_ g: UIPanGestureRecognizer) {
        if g.state == .began {
            let v = g.velocity(in: self)
            sideways = abs(v.x) > abs(v.y) && Core.shared.now.song != nil && !Core.shared.now.jam
        }
        guard sideways else { return dragged?(g) ?? () }
        let dx = g.translation(in: self).x
        let moving = sliding
        switch g.state {
        case .changed:
            moving.forEach { $0.transform = CGAffineTransform(translationX: dx, y: 0) }
        case .ended, .cancelled:
            let vx = g.velocity(in: self).x
            let goes = g.state == .ended && (abs(dx) > bounds.width * 0.25 || abs(vx) > 700)
            guard goes else {
                UIView.animate(withDuration: Motion.plain ? 0 : 0.25) { moving.forEach { $0.transform = .identity } }
                return
            }
            // Left for the next song, right for the previous; the new one comes in from the other side.
            let left = (dx != 0 ? dx : vx) < 0
            let away = left ? -bounds.width : bounds.width
            let plain = Motion.plain
            UIView.animate(withDuration: plain ? 0 : 0.18, animations: {
                moving.forEach { $0.transform = CGAffineTransform(translationX: away, y: 0) }
            }, completion: { _ in
                if left { nori_ios_next() } else { nori_ios_previous() }
                moving.forEach { $0.transform = CGAffineTransform(translationX: -away, y: 0) }
                UIView.animate(withDuration: plain ? 0 : 0.22) { moving.forEach { $0.transform = .identity } }
            })
        default:
            break
        }
    }
}

/// The output playing now, as the core reads it (`nori_ios_output_look`).
enum OutputLook {
    static func now() -> (glyph: Int, port: Int, name: String?) {
        let d = takenJSON(nori_ios_output_look()) as? [String: Any] ?? [:]
        return (d["glyph"] as? Int ?? 0, d["port"] as? Int ?? 0, d["name"] as? String)
    }
}

/// Downloads, with the line saying they run only while the app is open or playing.
final class DownloadsPage: PageController {
    init() { super.init(kind: NORI_PAGE_DOWNLOADS, title: Say.downloads) }
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        let note = label(Fonts.font(13, .regular, .footnote), Theme.secondary, lines: 2)
        note.frame = CGRect(x: 0, y: 0, width: UIScreen.main.bounds.width, height: 44)
        note.text = Say.downloadsRun
        note.textAlignment = .center
        tableView.tableFooterView = note
        NotificationCenter.default.addObserver(self, selector: #selector(stopAgain), name: UIApplication.didEnterBackgroundNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(resume), name: UIApplication.didBecomeActiveNotification, object: nil)
    }

    /// Read again a second on while something downloads and the page is on a lit screen.
    private var again: Timer?
    /// A row is swiped open: reading again would reload the table and close it, so it waits.
    private var swiped = false

    override func took(_ a: PageAnswer, appending: Bool) {
        super.took(a, appending: appending)
        stopAgain()
        guard a.raw["running"] as? Bool == true, view.window != nil,
              UIApplication.shared.applicationState == .active else { return }
        again = Timer.scheduledTimer(withTimeInterval: 1, repeats: false) { [weak self] _ in
            guard let self, !self.swiped else { return }
            self.reload()
        }
        again?.tolerance = 0.2
    }

    @objc private func stopAgain() {
        again?.invalidate()
        again = nil
    }

    @objc private func resume() {
        if answer?.raw["running"] as? Bool == true, view.window != nil { reload() }
    }

    override func tableView(_ tableView: UITableView, willBeginEditingRowAt indexPath: IndexPath) {
        swiped = true
    }

    override func tableView(_ tableView: UITableView, didEndEditingRowAt indexPath: IndexPath?) {
        swiped = false
        if answer?.raw["running"] as? Bool == true, again?.isValid != true { reload() }
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        if answer != nil { reload() }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        stopAgain()
    }
}

/// The index at every key, the server after a typing pause.
final class SearchPage: PageController, UISearchResultsUpdating, UISearchBarDelegate, UISearchControllerDelegate {
    private let search = UISearchController(searchResultsController: nil)
    private var pending: DispatchWorkItem?
    private var typed = ""
    /// The pause before the server is asked (the core's), seconds.
    private let delay = Double(nori_ios_search_delay_ms()) / 1000

    init() { super.init(kind: NORI_PAGE_SEARCH, title: Say.search) }
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        refreshControl = nil
        search.obscuresBackgroundDuringPresentation = false
        search.searchBar.placeholder = Say.searchHint
        search.searchBar.barStyle = Theme.bar
        search.searchBar.keyboardAppearance = Theme.keyboard
        search.searchBar.tintColor = Theme.accent
        search.searchBar.searchBarStyle = .minimal
        search.searchResultsUpdater = self
        search.searchBar.delegate = self
        search.delegate = self
        navigationItem.searchController = search
        navigationItem.hidesSearchBarWhenScrolling = false
        definesPresentationContext = true
    }

    /// Opened with nothing typed (the tab tapped): the field takes the keyboard at once. iOS 12 ignores
    /// both steps until the bar is in place, so each waits a turn of the run loop.
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard (search.searchBar.text ?? "").isEmpty else { return }
        DispatchQueue.main.async { self.search.isActive = true }
    }

    func didPresentSearchController(_ searchController: UISearchController) {
        DispatchQueue.main.async { searchController.searchBar.becomeFirstResponder() }
    }

    override func reload() {
        guard Core.shared.isOpen else { return super.reload() }
        query(typed)
    }

    private func query(_ text: String) {
        if token != 0 { Core.shared.forget(token) }
        Core.shared.read(NORI_PAGE_SEARCH, text, token: token == 0 ? nil : token) { [weak self] a in
            self?.took(a, appending: false)
        }
    }

    override func took(_ a: PageAnswer, appending: Bool) {
        if let recent = a.raw["recent"] as? [String], typed.isEmpty {
            let items = recent.map { ["k": "recent", "id": $0, "t": $0] }
            super.took(PageAnswer(["sections": items.isEmpty ? [] : [["key": "", "items": items]]]), appending: false)
            return
        }
        super.took(a, appending: appending)
    }

    func updateSearchResults(for searchController: UISearchController) {
        let text = searchController.searchBar.text ?? ""
        guard text != typed else { return }
        typed = text
        pending?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.query(text) }
        pending = work
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
    }

    func searchBarSearchButtonClicked(_ searchBar: UISearchBar) {
        let text = searchBar.text ?? ""
        text.withCString { nori_ios_remember($0) }
        pending?.cancel()
        query(text)
    }

    override func open(_ item: Item) {
        if item.kind == "recent" {
            search.searchBar.text = item.id
            search.isActive = true
            return
        }
        if !typed.isEmpty { typed.withCString { nori_ios_remember($0) } }
        super.open(item)
    }
}

final class EmptyState: UIView {
    private let heading = label(Fonts.title(22, .title2), Theme.label, lines: 0)
    private let line = label(Fonts.font(15), Theme.secondary, lines: 0)

    init(title: String, detail: String) {
        super.init(frame: .zero)
        heading.textAlignment = .center
        line.textAlignment = .center
        let stack = UIStackView(arrangedSubviews: [heading, line])
        stack.axis = .vertical
        stack.alignment = .fill
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
        set(title: title, detail: detail)
    }

    required init?(coder: NSCoder) { fatalError() }

    func set(title: String, detail: String) {
        heading.text = title
        line.text = detail
        line.isHidden = detail.isEmpty
    }
}

/// A plain table on the page's paper, its separators hairlines.
func dress(_ table: UITableView) {
    table.backgroundColor = Theme.background
    table.separatorColor = Theme.hairline
    table.cellLayoutMarginsFollowReadableWidth = false
}

/// A titled list of grouped cards: the scroll of a settings page and of the library's top.
class CardsPage: UIViewController {
    let column = UIStackView()
    private let scroll = UIScrollView()

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Theme.background
        column.axis = .vertical
        column.spacing = 0
        for v in [scroll, column] as [UIView] { v.translatesAutoresizingMaskIntoConstraints = false }
        view.addSubview(scroll)
        scroll.addSubview(column)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: view.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            column.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor),
            column.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor),
            column.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 8),
            column.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -Space.section),
            column.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor),
        ])
    }

    /// The column's contents, again.
    func set(_ views: [UIView]) {
        column.arrangedSubviews.forEach { $0.removeFromSuperview() }
        views.forEach(column.addArrangedSubview)
    }
}

/// A cell of a plain settings table on the page's surface, in the app's type.
func row(_ table: UITableView, id: String, style: UITableViewCell.CellStyle = .default) -> UITableViewCell {
    let cell = table.dequeueReusableCell(withIdentifier: id) ?? UITableViewCell(style: style, reuseIdentifier: id)
    cell.backgroundColor = Theme.row
    cell.textLabel?.textColor = Theme.label
    cell.textLabel?.font = Fonts.font(16)
    cell.textLabel?.adjustsFontForContentSizeCategory = true
    cell.detailTextLabel?.textColor = Theme.secondary
    cell.detailTextLabel?.font = Fonts.font(style == .subtitle ? 13 : 15, .regular, .footnote)
    cell.detailTextLabel?.adjustsFontForContentSizeCategory = true
    let selected = UIView()
    selected.backgroundColor = Theme.page.veil(0.06)
    cell.selectedBackgroundView = selected
    return cell
}
