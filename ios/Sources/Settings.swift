import UIKit

/// Settings, as Android's: its groups on one card, each with its icon and a line of what is in it; a group
/// opens its rows on grouped cards under small capitals. The rows are the core's settings
/// (`nori_ios_settings`), the ones this client shows (`Say.settings`), changed through `nori_ios_set`.
final class SettingsPage: CardsPage {
    private var server = Say.none
    private var values: [String: [String: Any]] = [:]
    private var facts: [String: Any] = [:]

    /// A setting's value, from the open session.
    static func value(_ name: String) -> String? {
        guard let rows = takenJSON(nori_ios_settings()) as? [[String: Any]],
              let row = rows.first(where: { $0["name"] as? String == name }) else { return nil }
        return row["value"] as? String
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        NotificationCenter.default.addObserver(self, selector: #selector(load), name: .noriOpened, object: nil)
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        load()
    }

    @objc private func load() {
        let dir = dataDirectory().path
        DispatchQueue.global(qos: .userInitiated).async {
            let active = dir.withCString { takenJSON(nori_ios_active($0)) } as? [String: Any] ?? [:]
            let label = (active["label"] as? String).flatMap { $0.isEmpty ? nil : $0 }
                ?? (active["kind"] as? String).map { Say.provider($0).name } ?? ""
            DispatchQueue.main.async {
                self.server = label.isEmpty ? Say.none : label
                self.lay()
            }
        }
    }

    private func lay() {
        let open = Core.shared.isOpen
        let push = { [weak self] (page: UIViewController) in self?.navigationController?.pushViewController(page, animated: true) }
        var rows = [GroupRow(Say.servers, subtitle: server, icon: Glyph.draw(Iconoir.server, 20), chevron: true) { push(ServersPage()) }]
        if open {
            rows += SettingsGroup.all.map { g in
                GroupRow(g.title, subtitle: g.summary, icon: Glyph.draw(g.icon, 20), chevron: true) {
                    push(g.id == "look" ? AppearancePage() : SettingsGroupPage(g))
                }
            }
        } else {
            rows.append(GroupRow(Say.appearance, subtitle: Say.lookSummary, icon: Glyph.draw(Iconoir.palette, 20), chevron: true) { push(AppearancePage()) })
        }
        set([GroupCard(rows)])
    }
}

/// One group of the settings: its title, its line on the root, its icon, and the iPod's sections in it
/// (`Say.settings`' titles) with the rows of its own (`SettingsGroupPage.extras`).
struct SettingsGroup {
    let id: String
    let title: String
    let summary: String
    let icon: [GlyphPart]
    let sections: [String]

    static let all = [
        SettingsGroup(id: "playing", title: Say.groupPlaying, summary: Say.groupPlayingSummary, icon: Iconoir.play,
                      sections: ["Playback", "Queue", "When something goes wrong", "Remote control"]),
        SettingsGroup(id: "sound", title: Say.sound, summary: Say.groupSoundSummary, icon: Iconoir.controlSlider, sections: ["Sound"]),
        SettingsGroup(id: "look", title: Say.appearance, summary: Say.lookSummary, icon: Iconoir.palette, sections: []),
        SettingsGroup(id: "lyrics", title: Say.lyrics, summary: Say.groupLyricsSummary, icon: Iconoir.quoteMessage, sections: ["Lyrics"]),
        SettingsGroup(id: "library", title: Say.library, summary: Say.groupLibrarySummary, icon: Iconoir.albumList, sections: ["Song rows", "Server"]),
        SettingsGroup(id: "about", title: Say.about, summary: Say.groupAboutSummary, icon: Iconoir.infoCircle, sections: []),
    ]
}

/// A group's page: its sections as grouped cards under small capitals.
final class SettingsGroupPage: CardsPage {
    private let group: SettingsGroup
    private var values: [String: [String: Any]] = [:]
    private var facts: [String: Any] = [:]

    init(_ group: SettingsGroup) {
        self.group = group
        super.init(nibName: nil, bundle: nil)
        title = group.title
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        navigationItem.largeTitleDisplayMode = .never
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        load()
    }

    private func load() {
        DispatchQueue.global(qos: .userInitiated).async {
            let rows = takenJSON(nori_ios_settings()) as? [[String: Any]] ?? []
            let facts = takenJSON(nori_ios_facts()) as? [String: Any] ?? [:]
            DispatchQueue.main.async {
                self.values = Dictionary(rows.compactMap { r in (r["name"] as? String).map { ($0, r) } }, uniquingKeysWith: { a, _ in a })
                self.facts = facts
                self.lay()
            }
        }
    }

    private func lay() {
        var views: [UIView] = []
        let push = { [weak self] (page: UIViewController) in self?.navigationController?.pushViewController(page, animated: true) }
        // The group's own rows first: what opens a page of its own.
        switch group.id {
        case "sound":
            let eq = (values["eq"]?["value"] as? String) == "true" ? Say.on : Say.off
            views.append(GroupCard([
                GroupRow(Say.equalizer, value: eq, chevron: true) { push(EqualizerPage()) },
                GroupRow(Say.devices, chevron: true) { push(SoundPage()) },
            ]))
        case "library":
            let songs = facts["songs"] as? Int ?? 0
            views.append(GroupCard([GroupRow(Say.refreshIndex, value: Say.songs(songs)) {
                if Core.shared.rules.settings.contains(1) { nori_ios_sync() }
            }]))
        case "about":
            views.append(GroupCard([
                GroupRow(Say.version, value: String(cString: nori_ios_version())),
                GroupRow(Say.streamed, value: Fmt.bytes(facts["stream"] as? Int ?? 0)),
                GroupRow(Say.freeSoftware, value: "AGPL-3.0"),
                GroupRow(Say.licences, chevron: true) { push(CreditsPage()) },
            ]))
        default:
            break
        }
        let shown = { (name: String) in
            self.values[name] != nil && Say.shownWhen[name].map { self.values[$0]?["value"] as? String == "true" } ?? true
        }
        for s in Say.settings where group.sections.contains(s.section) {
            var rows = s.rows.filter { shown($0.name) }.map { row($0.name, $0.label) }
            if s.section == "Remote control" {
                rows.append(GroupRow(Say.joinJam, chevron: true) { [weak self] in
                    guard let self else { return }
                    JamJoin.ask(from: self)
                })
            }
            guard !rows.isEmpty else { continue }
            views.append(GroupLabel(s.section))
            views.append(GroupCard(rows))
        }
        set(views)
    }

    /// One setting's row: a switch, or its value and a sheet of the others.
    private func row(_ name: String, _ label: String) -> GroupRow {
        let spec = values[name] ?? [:]
        let value = spec["value"] as? String ?? ""
        if spec["kind"] as? String == "switch" {
            let toggle = UISwitch()
            toggle.isOn = value == "true"
            toggle.onTintColor = Theme.switchOn
            toggle.addAction { [weak self] in self?.set(name, toggle.isOn ? "true" : "false") }
            let r = GroupRow(label, trailing: toggle)
            r.title.numberOfLines = 2
            return r
        }
        let r = GroupRow(label, value: Say.choice(name, value), chevron: true) { [weak self] in self?.choose(name, label, spec) }
        r.title.numberOfLines = 2
        return r
    }

    private func choose(_ name: String, _ label: String, _ spec: [String: Any]) {
        guard let kind = spec["kind"] as? String else { return }
        let current = spec["value"] as? String ?? ""
        let options: [String]
        if kind == "choice" {
            options = spec["options"] as? [String] ?? []
        } else {
            let lo = spec["min"] as? Double ?? 0, hi = spec["max"] as? Double ?? 0
            let step = max(1, ((hi - lo) / 8).rounded())
            options = stride(from: lo, through: hi, by: step).map { $0 == $0.rounded() ? "\(Int($0))" : "\($0)" }
        }
        let sheet = Sheet(label)
        for o in options {
            sheet.add(Say.choice(name, o), checked: o == current) { [weak self] in self?.set(name, o) }
        }
        sheet.show(from: self)
    }

    private func set(_ name: String, _ value: String) {
        _ = name.withCString { n in value.withCString { v in nori_ios_set(n, v) } }
        load()
        // The player offers the devices only while remote control is on.
        if name == "remoteControl" { Core.shared.refresh() }
    }
}

/// Appearance: the four modes, then the season tiles, each its season's paper with its kanji in its accent
/// and its name under it, the one worn ringed in that accent; following the calendar under them. The core
/// owns the settings and the colors; a change builds the interface again in them.
final class AppearancePage: CardsPage {
    override func viewDidLoad() {
        super.viewDidLoad()
        title = Say.appearance
        navigationItem.largeTitleDisplayMode = .never
        lay()
    }

    private func lay() {
        let modes = ChipStrip()
        modes.set(Theme.modes.map(Say.mode), chosen: Theme.modes.firstIndex(of: Theme.mode) ?? 0)
        modes.picked = { [weak self] i in self?.change(mode: Theme.modes[i], season: Theme.season) }
        let following = Theme.season == "FOLLOW"
        let now = Theme.calendar()
        let worn = following ? Int(now.season) : Theme.seasons.firstIndex(of: Theme.season) ?? 0
        let tiles = UIStackView(arrangedSubviews: Theme.tiles().enumerated().map { i, p in
            SeasonTile(i, p, chosen: i == worn) { [weak self] in self?.change(mode: Theme.mode, season: Theme.seasons[i]) }
        })
        tiles.distribution = .fillEqually
        tiles.spacing = 10
        tiles.isLayoutMarginsRelativeArrangement = true
        tiles.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 0, leading: Space.groupMargin, bottom: 0, trailing: Space.groupMargin)
        let follow = UISwitch()
        follow.isOn = following
        follow.onTintColor = Theme.switchOn
        follow.addAction { [weak self] in
            // Off keeps the season the calendar has now.
            self?.change(mode: Theme.mode, season: follow.isOn ? "FOLLOW" : Theme.seasons[Int(now.season)])
        }
        let followCard = GroupCard([GroupRow(Say.followSeasons, subtitle: Say.seasonNow(now), trailing: follow)])
        let gap = UIView()
        gap.heightAnchor.constraint(equalToConstant: 10).isActive = true
        set([GroupLabel(Say.appearance), modeRow(modes), GroupLabel(Say.season), tiles, gap, followCard])
    }

    /// The modes' chips on a card, the same room above and below them.
    private func modeRow(_ chips: ChipStrip) -> UIView {
        let holder = UIView()
        chips.translatesAutoresizingMaskIntoConstraints = false
        holder.addSubview(chips)
        NSLayoutConstraint.activate([
            chips.leadingAnchor.constraint(equalTo: holder.leadingAnchor, constant: -Space.gutter + Space.groupInset),
            chips.trailingAnchor.constraint(equalTo: holder.trailingAnchor),
            chips.topAnchor.constraint(equalTo: holder.topAnchor, constant: 12),
            chips.bottomAnchor.constraint(equalTo: holder.bottomAnchor, constant: -12),
        ])
        return GroupCard([holder])
    }

    private func change(mode: String, season: String) {
        _ = "theme".withCString { n in mode.withCString { v in nori_ios_set(n, v) } }
        _ = "season".withCString { n in season.withCString { v in nori_ios_set(n, v) } }
        guard Theme.keep(mode: mode, season: season) else { return lay() }
        (UIApplication.shared.delegate as? AppDelegate)?.restyle()
    }
}

/// A season's tile: its paper with its kanji in its accent, and its name under it; the one worn ringed in
/// its accent, the others edged with a hairline.
final class SeasonTile: UIControl {
    private let picked: () -> Void

    init(_ season: Int, _ p: NoriPalette, chosen: Bool, picked: @escaping () -> Void) {
        self.picked = picked
        super.init(frame: .zero)
        let accent = UIColor(argb: p.accent)
        let paper = UIView()
        paper.backgroundColor = UIColor(argb: p.page)
        paper.layer.cornerRadius = Radius.tile
        paper.layer.borderWidth = chosen ? 2 : hairlineWidth
        paper.layer.borderColor = (chosen ? accent : Theme.hairline).cgColor
        let kanji = UIImageView(image: Glyph.seasons[season])
        kanji.tintColor = accent
        kanji.translatesAutoresizingMaskIntoConstraints = false
        paper.addSubview(kanji)
        let name = UILabel()
        name.text = Say.seasons[season].0
        name.font = Fonts.font(12, .medium, .caption1)
        name.textColor = chosen ? Theme.label : Theme.secondary
        let column = UIStackView(arrangedSubviews: [paper, name])
        column.axis = .vertical
        column.alignment = .center
        column.spacing = 6
        column.isUserInteractionEnabled = false
        column.translatesAutoresizingMaskIntoConstraints = false
        addSubview(column)
        NSLayoutConstraint.activate([
            column.topAnchor.constraint(equalTo: topAnchor),
            column.bottomAnchor.constraint(equalTo: bottomAnchor),
            column.leadingAnchor.constraint(equalTo: leadingAnchor),
            column.trailingAnchor.constraint(equalTo: trailingAnchor),
            paper.widthAnchor.constraint(equalTo: column.widthAnchor),
            paper.heightAnchor.constraint(equalToConstant: 72),
            kanji.centerXAnchor.constraint(equalTo: paper.centerXAnchor),
            kanji.centerYAnchor.constraint(equalTo: paper.centerYAnchor),
        ])
        addTarget(self, action: #selector(tap), for: .touchUpInside)
        isAccessibilityElement = true
        accessibilityLabel = Say.seasons[season].0
        accessibilityTraits = chosen ? [.button, .selected] : .button
    }

    required init?(coder: NSCoder) { fatalError() }

    @objc private func tap() { picked() }
}

/// The equalizer's curve, a point on it for each band: drawn from the core's samples, touched to choose
/// a band and drag it (a graphic one only up and down), or on a parametric curve to add a band there.
final class EqCurveView: UIView {
    var eq: [String: Any] = [:] { didSet { setNeedsDisplay() } }
    var chosen = -1 { didSet { setNeedsDisplay() } }
    /// A band touched (its index), dragged to a place across (0..1) and a gain, or a band to add there.
    var touched: ((Int) -> Void)?
    var dragged: ((Int, Float, Float) -> Void)?
    var added: ((Float, Float) -> Void)?
    private var held = -1

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = false
        contentMode = .redraw
        layer.cornerRadius = Radius.card
        isAccessibilityElement = true
        accessibilityLabel = Say.equalizer
    }

    required init?(coder: NSCoder) { fatalError() }

    private var on: Bool { eq["on"] as? Bool ?? false }
    private var reach: CGFloat { CGFloat(eq["reach"] as? Double ?? 15) }
    private var points: [[String: Any]] { eq["points"] as? [[String: Any]] ?? [] }
    private var plot: CGRect { bounds.insetBy(dx: 12, dy: 14) }

    private func xOf(_ x: Double) -> CGFloat { plot.minX + CGFloat(x) * plot.width }
    private func yOf(_ db: Double) -> CGFloat { plot.midY - CGFloat(db) / reach * plot.height / 2 }

    private func trace(_ samples: [Double], closed: Bool) -> UIBezierPath {
        let path = UIBezierPath()
        guard samples.count > 1 else { return path }
        for (i, db) in samples.enumerated() {
            let p = CGPoint(x: plot.minX + CGFloat(i) / CGFloat(samples.count - 1) * plot.width, y: yOf(min(max(db, -Double(reach)), Double(reach))))
            i == 0 ? path.move(to: p) : path.addLine(to: p)
        }
        if closed {
            path.addLine(to: CGPoint(x: plot.maxX, y: yOf(0)))
            path.addLine(to: CGPoint(x: plot.minX, y: yOf(0)))
            path.close()
        }
        return path
    }

    override func draw(_ rect: CGRect) {
        Theme.row.setFill()
        UIBezierPath(roundedRect: bounds, cornerRadius: Radius.card).fill()
        let ink = on ? Theme.accent : Theme.secondary
        Theme.hairline.setStroke()
        for db in [-12.0, 0, 12] {
            let line = UIBezierPath()
            line.move(to: CGPoint(x: plot.minX, y: yOf(db)))
            line.addLine(to: CGPoint(x: plot.maxX, y: yOf(db)))
            line.lineWidth = db == 0 ? 1 : hairlineWidth
            line.stroke()
        }
        let small: [NSAttributedString.Key: Any] = [.font: Fonts.digits(10), .foregroundColor: Theme.secondary]
        for mark in eq["marks"] as? [[String: Any]] ?? [] {
            let x = xOf(mark["x"] as? Double ?? 0)
            let line = UIBezierPath()
            line.move(to: CGPoint(x: x, y: plot.minY))
            line.addLine(to: CGPoint(x: x, y: plot.maxY))
            line.lineWidth = hairlineWidth
            line.stroke()
            ((mark["label"] as? String ?? "") as NSString).draw(at: CGPoint(x: x + 3, y: plot.maxY - 12), withAttributes: small)
        }
        let each = eq["each"] as? [[Double]] ?? []
        if chosen >= 0, chosen < each.count, on {
            ink.withAlphaComponent(0.22).setFill()
            trace(each[chosen], closed: true).fill()
        }
        let curve = eq["curve"] as? [Double] ?? []
        ink.withAlphaComponent(0.12).setFill()
        trace(curve, closed: true).fill()
        ink.setStroke()
        let line = trace(curve, closed: false)
        line.lineWidth = 2
        line.lineJoinStyle = .round
        line.stroke()
        for (i, pt) in points.enumerated() {
            let lit = on && (i == chosen || i == held)
            let r: CGFloat = lit ? 7 : 5
            let c = CGPoint(x: xOf(pt["x"] as? Double ?? 0), y: yOf(pt["db"] as? Double ?? 0))
            let dot = UIBezierPath(ovalIn: CGRect(x: c.x - r, y: c.y - r, width: r * 2, height: r * 2))
            (lit ? Theme.accent : Theme.label).setFill()
            dot.fill()
        }
    }

    /// Where `p` is on the curve: across (0..1) and dB, held to the curve's reach.
    private func place(_ p: CGPoint) -> (Float, Float) {
        let across = Float(min(max((p.x - plot.minX) / plot.width, 0), 1))
        let db = Float(min(max((plot.midY - p.y) / (plot.height / 2) * reach, -reach), reach))
        return (across, (db * 2).rounded() / 2)
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard on, let p = touches.first?.location(in: self) else { return }
        let near = points.enumerated().map { (i, pt) -> (Int, CGFloat) in
            (i, hypot(xOf(pt["x"] as? Double ?? 0) - p.x, yOf(pt["db"] as? Double ?? 0) - p.y))
        }.min { $0.1 < $1.1 }
        if let (i, d) = near, d < 28 {
            held = i
            touched?(i)
        } else if !(eq["graphic"] as? Bool ?? true) {
            let (across, db) = place(p)
            added?(across, db)
        }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard held >= 0, let p = touches.first?.location(in: self) else { return }
        let (across, db) = place(p)
        dragged?(held, across, db)
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) { held = -1; setNeedsDisplay() }
    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) { held = -1; setNeedsDisplay() }
}

/// The equalizer: graphic or parametric, its curve to touch, the chosen band in words and, for a
/// parametric band, its kind, side and width. While it is in sight and touched, the output runs its
/// short buffer so a change is heard at once.
final class EqualizerPage: UIViewController {
    private let toggle = UISwitch()
    private let stack = UIStackView()
    private let curve = EqCurveView()
    private let graphic = Chip(Say.graphic)
    private let parametric = Chip(Say.parametric)
    private let words = label(Fonts.digits(13), Theme.secondary, lines: 2)
    private let bandButton = PillButton(Say.band, nil, prominent: false)
    private let narrower = PillButton(Say.narrower, nil, prominent: false)
    private let wider = PillButton(Say.wider, nil, prominent: false)
    private lazy var bandRow = UIStackView(arrangedSubviews: [bandButton, narrower, wider])
    private var eq: [String: Any] = [:]
    private var chosen = -1

    override func viewDidLoad() {
        super.viewDidLoad()
        title = Say.equalizer
        navigationItem.largeTitleDisplayMode = .never
        view.backgroundColor = Theme.background
        toggle.onTintColor = Theme.switchOn
        toggle.addTarget(self, action: #selector(switched), for: .valueChanged)
        navigationItem.rightBarButtonItem = UIBarButtonItem(customView: toggle)
        stack.axis = .vertical
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false
        let scroll = UIScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.delaysContentTouches = false
        view.addSubview(scroll)
        scroll.addSubview(stack)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: view.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: scroll.leadingAnchor, constant: Space.gutter),
            stack.trailingAnchor.constraint(equalTo: scroll.trailingAnchor, constant: -Space.gutter),
            stack.topAnchor.constraint(equalTo: scroll.topAnchor, constant: 16),
            stack.bottomAnchor.constraint(equalTo: scroll.bottomAnchor, constant: -16),
            stack.widthAnchor.constraint(equalTo: scroll.widthAnchor, constant: -Space.gutter * 2),
        ])
        guard takenJSON(nori_ios_equalizer()) is [String: Any] else {
            stack.addArrangedSubview(EmptyState(title: Say.noServer, detail: Say.noServerDetail))
            toggle.isEnabled = false
            return
        }
        graphic.addTarget(self, action: #selector(toGraphic), for: .touchUpInside)
        parametric.addTarget(self, action: #selector(toParametric), for: .touchUpInside)
        let presets = PillButton(Say.presets, nil, prominent: false)
        presets.addTarget(self, action: #selector(presetsTapped), for: .touchUpInside)
        let modes = UIStackView(arrangedSubviews: [graphic, parametric, UIView(), presets])
        modes.spacing = 8
        modes.alignment = .center
        stack.addArrangedSubview(modes)
        curve.heightAnchor.constraint(equalToConstant: 200).isActive = true
        curve.touched = { [weak self] i in self?.choose(i) }
        curve.dragged = { [weak self] i, across, db in self?.drag(i, across, db) }
        curve.added = { [weak self] across, db in self?.add(across, db) }
        stack.addArrangedSubview(curve)
        stack.addArrangedSubview(words)
        bandRow.spacing = 8
        bandRow.distribution = .fillEqually
        bandButton.addTarget(self, action: #selector(bandTapped), for: .touchUpInside)
        narrower.addTarget(self, action: #selector(narrowerTapped), for: .touchUpInside)
        wider.addTarget(self, action: #selector(widerTapped), for: .touchUpInside)
        stack.addArrangedSubview(bandRow)
        paint()
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        nori_ios_tuning(1, 0)
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        nori_ios_tuning(0, 0)
    }

    private var bands: [[String: Any]] { eq["bands"] as? [[String: Any]] ?? [] }
    private var isGraphic: Bool { eq["graphic"] as? Bool ?? true }

    /// Everything as the settings now hold them.
    private func paint() {
        guard let now = takenJSON(nori_ios_equalizer()) as? [String: Any] else { return }
        eq = now
        if chosen >= bands.count { chosen = -1 }
        let on = eq["on"] as? Bool ?? false
        toggle.isOn = on
        graphic.chosen = isGraphic
        parametric.chosen = !isGraphic
        curve.eq = eq
        curve.chosen = chosen
        if chosen >= 0 {
            let b = bands[chosen]
            words.text = Say.eqBand(b["label"] as? String ?? "", b["value"] as? String ?? "", kind: isGraphic ? nil : b["kind"] as? Int,
                                    width: b["width"] as? String ?? "", side: b["channel"] as? Int ?? 0, usesGain: b["usesGain"] as? Bool ?? true)
            words.textColor = on ? Theme.accent : Theme.label
        } else {
            words.text = isGraphic ? Say.eqHintGraphic : Say.eqHintParametric
            words.textColor = Theme.secondary
        }
        bandRow.isHidden = isGraphic || chosen < 0 || !on
    }

    private func choose(_ i: Int) {
        chosen = i
        paint()
    }

    /// Band `i` set as `pos`, `gain` and what else it has, or as its editor changed it.
    private func set(_ i: Int, pos: Float? = nil, gain: Float? = nil, q: Float? = nil, kind: Int? = nil, channel: Int? = nil) {
        guard i >= 0, i < bands.count, i < (eq["points"] as? [[String: Any]] ?? []).count else { return }
        let b = bands[i]
        let point = (eq["points"] as? [[String: Any]] ?? [])[i]
        nori_ios_tuning(1, 1)
        let changed = nori_ios_eq_point(UInt32(i), pos ?? Float(point["x"] as? Double ?? 0), gain ?? Float(b["gain"] as? Double ?? 0),
                                        q ?? Float(b["q"] as? Double ?? 1), Int32(kind ?? b["kind"] as? Int ?? 0), Int32(channel ?? b["channel"] as? Int ?? 0))
        if changed == 1 { paint() }
    }

    private func drag(_ i: Int, _ across: Float, _ db: Float) {
        chosen = i
        let gain = (bands[safe: i]?["usesGain"] as? Bool ?? true) ? db : nil
        set(i, pos: isGraphic ? nil : across, gain: gain)
    }

    private func add(_ across: Float, _ db: Float) {
        nori_ios_tuning(1, 1)
        if nori_ios_eq_band(0, across, db) == 1 {
            chosen = bands.count
            paint()
        }
    }

    @objc private func switched() {
        _ = "eq".withCString { n in (toggle.isOn ? "true" : "false").withCString { v in nori_ios_set(n, v) } }
        nori_ios_tuning(1, 1)
        paint()
    }

    private func mode(_ value: String) {
        chosen = -1
        _ = "eqMode".withCString { n in value.withCString { v in nori_ios_set(n, v) } }
        nori_ios_tuning(1, 1)
        paint()
    }

    @objc private func toGraphic() { if !isGraphic { mode("GRAPHIC") } }
    @objc private func toParametric() { if isGraphic { mode("PARAMETRIC") } }

    @objc private func narrowerTapped() { width(by: 1.25) }
    @objc private func widerTapped() { width(by: 0.8) }

    private func width(by factor: Double) {
        guard let q = bands[safe: chosen]?["q"] as? Double else { return }
        set(chosen, q: Float(q * factor))
    }

    @objc private func bandTapped() {
        guard let b = bands[safe: chosen] else { return }
        let i = chosen
        let sheet = Sheet(Say.band)
        for k in 0..<10 {
            sheet.add(Say.bandKind(k), checked: b["kind"] as? Int == k) { [weak self] in self?.set(i, kind: k) }
        }
        for c in 0..<3 {
            sheet.add(Say.bandSide(c), checked: b["channel"] as? Int == c) { [weak self] in self?.set(i, channel: c) }
        }
        sheet.add(Say.removeBand, destructive: true) { [weak self] in
            nori_ios_tuning(1, 1)
            if nori_ios_eq_band(UInt32(i), -1, 0) == 1 {
                self?.chosen = -1
                self?.paint()
            }
        }
        sheet.show(from: self)
    }

    @objc private func presetsTapped() {
        guard let kinds = takenJSON(nori_ios_presets()) as? [Int] else { return }
        let sheet = Sheet(Say.presets)
        for (i, kind) in kinds.enumerated() {
            sheet.add(Say.preset(kind)) { [weak self] in
                nori_ios_tuning(1, 1)
                if nori_ios_preset(UInt32(i)) == 1 { self?.paint() }
            }
        }
        sheet.show(from: self)
    }
}
