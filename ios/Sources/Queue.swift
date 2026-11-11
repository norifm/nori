import UIKit

/// The queue in the artwork's place on the player, as Android's: the song playing at the top as it opens, what has played above it under
/// "History", what plays next under "Playing next". Shuffle and repeat stay pinned beside the title.
/// Hold a row and drag to move it (when the play order is the list's), swipe to remove, tap to play.
/// The rows, their order and what may move or go are the core's (`queue_rows`). A jam guest's is the
/// host's queue, read only, under the jam: its host and listeners, Listen Here, Leave, and the songs it
/// asked for.
final class QueuePanel: UIViewController, PlayerPanel, UITableViewDataSource, UITableViewDelegate,
    UITableViewDragDelegate, UITableViewDropDelegate {
    private let table = UITableView(frame: .zero, style: .grouped)
    private let name = UILabel()
    private let edge = UnderEdge(Theme.player.page)
    /// The song playing tapped: back to the artwork.
    private let showPlayer: () -> Void
    /// The list rests on the song playing, so the next one comes to the top too.
    private var following = true
    private var followed: String?
    private let shuffle = UIButton(type: .system)
    private let repeatButton = UIButton(type: .system)
    /// After a swipe or Clear: what went and Undo, for a few seconds.
    private let undoBar = UIView()
    private let undoText = UILabel()
    private var taken: Taken?
    private var undoTimer: Timer?
    private var token: UInt64 = 0
    private var history: [Item] = []
    private var now: [Item] = []
    private var upcoming: [Item] = []
    private var reorderable = false
    private var kept: Set<Int> = []
    /// Another device's queue, mirrored: a tap plays there, nothing is edited from here.
    private var mirrored = false
    /// The jam this iPod is a guest in, over its host's queue.
    private var jam: Jam?
    private var opened = false
    /// What the queue was when last read: a new read only when it changes, not at every position tick.
    private var seen = ""

    private enum Part: Int, CaseIterable { case jam, asked, history, now, next }

    /// The jam part's rows.
    private enum JamRow: Int, CaseIterable { case people, listen, leave }

    init(showPlayer: @escaping () -> Void) {
        self.showPlayer = showPlayer
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    deinit { if token != 0 { Core.shared.forget(token) } }

    override func viewDidLoad() {
        super.viewDidLoad()
        name.text = Say.queue
        name.font = Fonts.font(16, .semibold, .headline)
        shuffle.setImage(Glyph.shuffle, for: .normal)
        shuffle.accessibilityLabel = Say.shuffle
        repeatButton.accessibilityLabel = Say.repeatMode
        shuffle.addTarget(self, action: #selector(shuffleTapped), for: .touchUpInside)
        repeatButton.addTarget(self, action: #selector(repeatTapped), for: .touchUpInside)
        let bar = UIStackView(arrangedSubviews: [name, UIView(), shuffle, repeatButton])
        bar.spacing = 4
        bar.alignment = .center

        table.backgroundColor = .clear
        table.separatorStyle = .none
        table.rowHeight = 56
        table.estimatedSectionHeaderHeight = 0
        table.estimatedSectionFooterHeight = 0
        table.tableHeaderView = PageController.noHeader
        table.dataSource = self
        table.delegate = self
        table.dragDelegate = self
        table.dropDelegate = self
        table.dragInteractionEnabled = true
        table.register(QueueCell.self, forCellReuseIdentifier: QueueCell.id)
        table.contentInset.bottom = 24

        for v in [bar, table] {
            v.translatesAutoresizingMaskIntoConstraints = false
            view.addSubview(v)
        }
        NSLayoutConstraint.activate([
            bar.topAnchor.constraint(equalTo: view.topAnchor),
            bar.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: Space.gutter),
            bar.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -8),
            bar.heightAnchor.constraint(equalToConstant: 44),
            shuffle.widthAnchor.constraint(equalToConstant: 44),
            repeatButton.widthAnchor.constraint(equalToConstant: 44),
            table.topAnchor.constraint(equalTo: bar.bottomAnchor, constant: 4),
            table.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            table.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            table.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        edge.pin(in: view, under: table.topAnchor)
        edge.follow(table)
        undoBar.backgroundColor = Theme.Card.track
        undoBar.layer.cornerRadius = 10
        undoBar.isHidden = true
        undoText.font = Fonts.font(13, .regular, .footnote)
        undoText.textColor = Theme.Card.label
        let undo = UIButton(type: .system)
        undo.setTitle(Say.undo, for: .normal)
        undo.tintColor = Theme.Card.label
        undo.titleLabel?.font = Fonts.font(15, .semibold)
        undo.addTarget(self, action: #selector(undoTapped), for: .touchUpInside)
        undo.setContentHuggingPriority(.required, for: .horizontal)
        let undoRow = UIStackView(arrangedSubviews: [undoText, undo])
        undoRow.spacing = 12
        undoRow.alignment = .center
        undoRow.translatesAutoresizingMaskIntoConstraints = false
        undoBar.addSubview(undoRow)
        undoBar.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(undoBar)
        NSLayoutConstraint.activate([
            undoRow.leadingAnchor.constraint(equalTo: undoBar.leadingAnchor, constant: 14),
            undoRow.trailingAnchor.constraint(equalTo: undoBar.trailingAnchor, constant: -10),
            undoRow.topAnchor.constraint(equalTo: undoBar.topAnchor, constant: 4),
            undoRow.bottomAnchor.constraint(equalTo: undoBar.bottomAnchor, constant: -4),
            undoBar.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 12),
            undoBar.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -12),
            undoBar.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -12),
            undoBar.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
        ])

        NotificationCenter.default.addObserver(self, selector: #selector(changed), name: .noriNow, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(jamChanged), name: .noriJam, object: nil)
        repaint()
        read()
    }

    func repaint() {
        name.textColor = Theme.Card.label
        edge.paint(Theme.player.page)
        paintToggles()
        table.reloadData()
    }

    /// The jam changed, and with what its host took, its queue.
    @objc private func jamChanged() {
        read()
    }

    func scrollViewWillBeginDragging(_ scrollView: UIScrollView) { following = false }

    func scrollViewDidEndDragging(_ scrollView: UIScrollView, willDecelerate decelerate: Bool) {
        if !decelerate { noteRest() }
    }

    func scrollViewDidEndDecelerating(_ scrollView: UIScrollView) { noteRest() }

    /// A scroll that ended on the song playing goes on following it.
    private func noteRest() {
        following = table.indexPathsForVisibleRows?.first?.section == Part.now.rawValue
    }

    @objc private func changed() {
        paintToggles()
        if !table.hasActiveDrag { readIfChanged() }
    }

    private func readIfChanged() {
        let n = Core.shared.now
        let key = "\(n.index) \(n.length) \(n.shuffle) \(n.repeatMode) \(n.song?.id ?? "")"
        if key != seen { read() }
    }

    private func paintToggles() {
        let n = Core.shared.now
        // A jam's play order is its host's.
        shuffle.isHidden = n.jam
        repeatButton.isHidden = n.jam
        shuffle.tintColor = n.shuffle ? Theme.Card.accent : Theme.Card.dim
        repeatButton.tintColor = n.repeatMode == 0 ? Theme.Card.dim : Theme.Card.accent
        repeatButton.setImage(n.repeatMode == 1 ? Glyph.repeatOne : Glyph.repeatAll, for: .normal)
        shuffle.accessibilityTraits = n.shuffle ? [.button, .selected] : .button
        repeatButton.accessibilityTraits = n.repeatMode == 0 ? .button : [.button, .selected]
    }

    private func read() {
        let n = Core.shared.now
        seen = "\(n.index) \(n.length) \(n.shuffle) \(n.repeatMode) \(n.song?.id ?? "")"
        if token != 0 { Core.shared.forget(token) }
        token = Core.shared.read(NORI_PAGE_QUEUE) { [weak self] a in self?.took(a) }
    }

    private func took(_ a: PageAnswer) {
        let part = { (key: String) in a.sections.first { $0.key == key }?.items ?? [] }
        history = part("history")
        now = part("now")
        upcoming = part("next")
        reorderable = a.head["reorderable"] as? Bool ?? false
        kept = Set(a.head["kept"] as? [Int] ?? [])
        mirrored = a.head["remote"] as? Bool ?? false
        jam = a.head["jam"] as? Bool == true ? Core.shared.jam : nil
        table.reloadData()
        // Opens on the song playing, what has played above it out of sight.
        let playing = IndexPath(row: 0, section: Part.now.rawValue)
        if !opened, !now.isEmpty {
            opened = true
            table.scrollToRow(at: playing, at: .top, animated: false)
        } else if !now.isEmpty, following, now.first?.id != followed, !table.hasActiveDrag {
            table.scrollToRow(at: playing, at: .top, animated: !Motion.plain)
        }
        followed = now.first?.id
    }

    private func items(_ section: Int) -> [Item] {
        switch Part(rawValue: section) {
        case .jam: return []
        case .asked: return jam?.asks ?? []
        case .history: return history
        case .now: return now
        default: return upcoming
        }
    }

    @objc private func shuffleTapped() {
        nori_ios_shuffle(Core.shared.now.shuffle ? 0 : 1)
        Core.shared.refresh()
    }

    @objc private func repeatTapped() {
        nori_ios_cycle_repeat()
        Core.shared.refresh()
    }

    @objc private func clearTapped() {
        let count = upcoming.count
        nori_ios_clear_upcoming()
        read()
        offerUndo(Say.queueCleared(count), .cleared)
    }

    // MARK: the list

    func numberOfSections(in tableView: UITableView) -> Int { Part.allCases.count }

    func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        Part(rawValue: section) == .jam ? (jam == nil ? 0 : JamRow.allCases.count) : items(section).count
    }

    func tableView(_ tableView: UITableView, heightForHeaderInSection section: Int) -> CGFloat {
        switch Part(rawValue: section) {
        case .asked where !items(section).isEmpty, .next where !upcoming.isEmpty: return 34
        case .history where !history.isEmpty: return 28
        default: return .leastNormalMagnitude
        }
    }

    func tableView(_ tableView: UITableView, heightForFooterInSection section: Int) -> CGFloat { .leastNormalMagnitude }

    func tableView(_ tableView: UITableView, viewForHeaderInSection section: Int) -> UIView? {
        let part = Part(rawValue: section)
        guard part != .now, part != .jam, !items(section).isEmpty else { return nil }
        let header = UIView()
        let label = UILabel()
        let words = part == .asked ? Say.youAskedFor : part == .history ? Say.history : Say.upNext
        label.attributedText = NSAttributedString(string: words.uppercased(), attributes: [.kern: 0.7])
        label.font = Fonts.font(11.5, .semibold, .caption1)
        label.textColor = Theme.Card.secondary
        label.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: header.leadingAnchor, constant: Space.gutter),
            label.bottomAnchor.constraint(equalTo: header.bottomAnchor, constant: -6),
        ])
        if part == .next && !mirrored {
            let clear = UIButton(type: .system)
            clear.setTitle(Say.clear, for: .normal)
            clear.tintColor = Theme.Card.secondary
            clear.titleLabel?.font = Fonts.font(13, .medium, .footnote)
            clear.addTarget(self, action: #selector(clearTapped), for: .touchUpInside)
            clear.translatesAutoresizingMaskIntoConstraints = false
            header.addSubview(clear)
            NSLayoutConstraint.activate([
                clear.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -Space.gutter),
                clear.centerYAnchor.constraint(equalTo: label.centerYAnchor),
            ])
        }
        return header
    }

    func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let part = Part(rawValue: indexPath.section)
        if part == .jam, let jam {
            return jamCell(tableView, JamRow(rawValue: indexPath.row) ?? .people, jam)
        }
        let cell = tableView.dequeueReusableCell(withIdentifier: QueueCell.id, for: indexPath) as! QueueCell
        let item = items(indexPath.section)[indexPath.row]
        if part == .asked {
            cell.show(item, played: false, playing: false, movable: false, note: Say.waitingFor(jam?.host ?? ""))
        } else {
            cell.show(item, played: part == .history, playing: part == .now, movable: part == .next && reorderable)
        }
        return cell
    }

    /// The jam's rows: its host and who listens, Listen Here (and why it does not play), Leave.
    private func jamCell(_ tableView: UITableView, _ row: JamRow, _ jam: Jam) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(withIdentifier: "jam") ?? UITableViewCell(style: .subtitle, reuseIdentifier: "jam")
        cell.backgroundColor = .clear
        cell.selectionStyle = .none
        cell.accessoryView = nil
        cell.textLabel?.textColor = Theme.Card.label
        cell.textLabel?.font = Fonts.font(17, .regular)
        cell.detailTextLabel?.textColor = Theme.Card.secondary
        cell.detailTextLabel?.font = Fonts.font(13, .regular, .footnote)
        cell.detailTextLabel?.numberOfLines = 0
        switch row {
        case .people:
            cell.textLabel?.text = Say.jamOf(jam.host)
            cell.textLabel?.font = Fonts.font(17, .semibold)
            cell.detailTextLabel?.text = ([Say.listening(jam.listeners.count)] + jam.listeners).joined(separator: " · ")
        case .listen:
            cell.textLabel?.text = jam.listening == 1 ? Say.playingHere : Say.listenHere
            cell.detailTextLabel?.text = Say.jamAlong(jam.listening)
            let toggle = UISwitch()
            toggle.isOn = jam.listening != 0
            toggle.onTintColor = Theme.switchOn
            toggle.addTarget(self, action: #selector(listenSwitched(_:)), for: .valueChanged)
            cell.accessoryView = toggle
        case .leave:
            cell.textLabel?.text = Say.leaveJam
            cell.textLabel?.textColor = .systemRed
            cell.detailTextLabel?.text = nil
            cell.selectionStyle = .default
        }
        return cell
    }

    @objc private func listenSwitched(_ s: UISwitch) {
        nori_ios_jam_listen(s.isOn ? 1 : 0)
    }

    func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        if Part(rawValue: indexPath.section) == .jam, JamRow(rawValue: indexPath.row) == .leave {
            nori_ios_jam_leave()
            parent?.dismiss(animated: true)
            return
        }
        // The jam's rows, its requests and its host's queue: nothing a guest plays.
        guard jam == nil else { return }
        if Part(rawValue: indexPath.section) == .now { return showPlayer() }
        following = true
        _ = nori_ios_play_at(Int32(items(indexPath.section)[indexPath.row].index), 0)
    }

    func tableView(_ tableView: UITableView, trailingSwipeActionsConfigurationForRowAt indexPath: IndexPath) -> UISwipeActionsConfiguration? {
        guard let part = Part(rawValue: indexPath.section), [.history, .now, .next].contains(part), jam == nil else { return nil }
        let item = items(indexPath.section)[indexPath.row]
        guard !kept.contains(item.index) else { return nil }
        let remove = UIContextualAction(style: .destructive, title: Say.remove) { [weak self] _, _, done in
            nori_ios_remove(Int32(item.index))
            done(true)
            self?.read()
            self?.offerUndo(Say.queueRemoved(item.title.isEmpty ? item.id : item.title), .song(item.id))
        }
        return UISwipeActionsConfiguration(actions: [remove])
    }

    // MARK: taking a removal back

    /// What the undo puts back: a song taken out, by id, or what a clear took.
    private enum Taken {
        case song(String)
        case cleared
    }

    private func offerUndo(_ text: String, _ what: Taken) {
        taken = what
        undoText.text = text
        undoBar.isHidden = false
        undoTimer?.invalidate()
        undoTimer = Timer.scheduledTimer(withTimeInterval: TimeInterval(nori_ios_undo_ms()) / 1000, repeats: false) { [weak self] _ in self?.undoBar.isHidden = true }
    }

    /// The core puts the songs back where they were (`put_back`, `put_back_cleared`).
    @objc private func undoTapped() {
        undoTimer?.invalidate()
        undoBar.isHidden = true
        switch taken {
        case .song(let id): id.withCString { nori_ios_put_back($0) }
        case .cleared: nori_ios_put_back_cleared()
        case nil: break
        }
        taken = nil
        read()
    }

    // MARK: moving a song: held and dragged, dropped only among those still to come

    func tableView(_ tableView: UITableView, itemsForBeginning session: UIDragSession, at indexPath: IndexPath) -> [UIDragItem] {
        guard reorderable, Part(rawValue: indexPath.section) == .next else { return [] }
        return [UIDragItem(itemProvider: NSItemProvider())]
    }

    /// Changes that came during the drag are read now (a cancelled drag moves nothing, so nothing else reads).
    func tableView(_ tableView: UITableView, dragSessionDidEnd session: UIDragSession) {
        readIfChanged()
    }

    func tableView(_ tableView: UITableView, dropSessionDidUpdate session: UIDropSession, withDestinationIndexPath destination: IndexPath?) -> UITableViewDropProposal {
        guard session.localDragSession != nil, let d = destination, Part(rawValue: d.section) == .next else {
            return UITableViewDropProposal(operation: .forbidden)
        }
        return UITableViewDropProposal(operation: .move, intent: .insertAtDestinationIndexPath)
    }

    func tableView(_ tableView: UITableView, performDropWith coordinator: UITableViewDropCoordinator) {}

    func tableView(_ tableView: UITableView, canMoveRowAt indexPath: IndexPath) -> Bool {
        reorderable && Part(rawValue: indexPath.section) == .next
    }

    func tableView(_ tableView: UITableView, targetIndexPathForMoveFromRowAt source: IndexPath, toProposedIndexPath proposed: IndexPath) -> IndexPath {
        Part(rawValue: proposed.section) == .next ? proposed : IndexPath(row: 0, section: Part.next.rawValue)
    }

    func tableView(_ tableView: UITableView, moveRowAt source: IndexPath, to destination: IndexPath) {
        guard source != destination else { return }
        // In list order (the play order, when moving is offered): the song lands where the row it
        // displaced was.
        let from = upcoming[source.row].index
        let to = upcoming[destination.row].index
        upcoming.insert(upcoming.remove(at: source.row), at: destination.row)
        nori_ios_move(Int32(from), Int32(to))
        read()
    }
}

/// A queue row: cover, title, artist; quieter once played, in the accent while playing, a handle when it can move.
final class QueueCell: UITableViewCell {
    static let id = "queue"
    private let cover = CoverView()
    private let title = UILabel()
    private let detail = UILabel()
    private let grip = UIImageView(image: Glyph.grip)

    override init(style: UITableViewCell.CellStyle, reuseIdentifier: String?) {
        super.init(style: style, reuseIdentifier: reuseIdentifier)
        backgroundColor = .clear
        selectedBackgroundView = UIView()
        title.font = Fonts.font(16)
        title.adjustsFontForContentSizeCategory = true
        detail.font = Fonts.font(13, .regular, .footnote)
        detail.adjustsFontForContentSizeCategory = true
        grip.setContentHuggingPriority(.required, for: .horizontal)
        let text = UIStackView(arrangedSubviews: [title, detail])
        text.axis = .vertical
        text.spacing = 2
        for v in [cover, text, grip] {
            v.translatesAutoresizingMaskIntoConstraints = false
            contentView.addSubview(v)
        }
        NSLayoutConstraint.activate([
            cover.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: Space.gutter),
            cover.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
            cover.widthAnchor.constraint(equalToConstant: 44),
            cover.heightAnchor.constraint(equalToConstant: 44),
            text.leadingAnchor.constraint(equalTo: cover.trailingAnchor, constant: 12),
            text.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
            text.trailingAnchor.constraint(equalTo: grip.leadingAnchor, constant: -8),
            grip.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -Space.gutter),
            grip.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    /// `note` in place of the artist; who asked for a jam's song after it.
    func show(_ item: Item, played: Bool, playing: Bool, movable: Bool, note: String? = nil) {
        cover.show(item.cover, points: 44)
        title.text = item.title.isEmpty ? item.id : item.title
        detail.text = note ?? ([item.subtitle, item.by].filter { !$0.isEmpty }.joined(separator: " · "))
        title.font = Fonts.font(16, playing ? .semibold : .regular)
        title.textColor = playing ? Theme.Card.accent : Theme.Card.label.withAlphaComponent(played ? 0.45 : 1)
        selectedBackgroundView?.backgroundColor = Theme.Card.track
        grip.tintColor = Theme.Card.dim
        detail.textColor = Theme.Card.secondary.withAlphaComponent(played ? 0.6 : 1)
        cover.alpha = played ? 0.5 : 1
        grip.isHidden = !movable
    }
}
