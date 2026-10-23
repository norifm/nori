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
        SettingsGroup(id: "lyrics", title: Say.lyrics, summary: Say.groupLyricsSummary, icon: Iconoir.emojiSingRightNote, sections: ["Lyrics"]),
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

/// Appearance: the four modes, then the season tiles, each its kanji over its paper, accent and "on", the
/// one worn raised; following the calendar under them. The core owns the settings and the colors; a
/// change builds the interface again in them.
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

/// A season's tile: its kanji over three dots (its paper, accent and "on"); the one worn raised on the
/// surface with a hairline edge. Its name is for VoiceOver.
final class SeasonTile: UIControl {
    private let picked: () -> Void

    init(_ season: Int, _ p: NoriPalette, chosen: Bool, picked: @escaping () -> Void) {
        self.picked = picked
        super.init(frame: .zero)
        layer.cornerRadius = Radius.tile
        backgroundColor = chosen ? Theme.row : .clear
        layer.borderWidth = chosen ? hairlineWidth : 0
        layer.borderColor = Theme.hairline.cgColor
        let kanji = UIImageView(image: Glyph.seasons[season])
        kanji.tintColor = chosen ? Theme.label : Theme.secondary
        let dots = UIStackView(arrangedSubviews: [p.page, p.accent, p.on].enumerated().map { i, c in
            let d = UIView()
            d.backgroundColor = UIColor(argb: c)
            d.layer.cornerRadius = 4
            d.layer.borderWidth = i == 0 ? hairlineWidth : 0
            d.layer.borderColor = Theme.hairline.cgColor
            d.translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([d.widthAnchor.constraint(equalToConstant: 8), d.heightAnchor.constraint(equalToConstant: 8)])
            return d
        })
        dots.spacing = 5
        let column = UIStackView(arrangedSubviews: [kanji, dots])
        column.axis = .vertical
        column.alignment = .center
        column.spacing = 8
        column.isUserInteractionEnabled = false
        column.translatesAutoresizingMaskIntoConstraints = false
        addSubview(column)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 88),
            column.centerXAnchor.constraint(equalTo: centerXAnchor),
            column.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        addTarget(self, action: #selector(tap), for: .touchUpInside)
        isAccessibilityElement = true
        accessibilityLabel = Say.seasons[season].0
        accessibilityTraits = chosen ? [.button, .selected] : .button
    }

    required init?(coder: NSCoder) { fatalError() }

    @objc private func tap() { picked() }
}

/// The graphic equalizer: one switch and a slider per band. While it is in sight and touched, the
/// output runs its short buffer so a move is heard at once.
final class EqualizerPage: UIViewController {
    private let toggle = UISwitch()
    private var sliders: [UISlider] = []
    private let stack = UIStackView()

    override func viewDidLoad() {
        super.viewDidLoad()
        title = Say.equalizer
        navigationItem.largeTitleDisplayMode = .never
        view.backgroundColor = Theme.background
        toggle.onTintColor = Theme.switchOn
        toggle.addTarget(self, action: #selector(switched), for: .valueChanged)
        navigationItem.rightBarButtonItem = UIBarButtonItem(customView: toggle)
        stack.axis = .vertical
        stack.spacing = 4
        stack.translatesAutoresizingMaskIntoConstraints = false
        let scroll = UIScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
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
        load()
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        nori_ios_tuning(1, 0)
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        nori_ios_tuning(0, 0)
    }

    private func load() {
        guard let eq = takenJSON(nori_ios_equalizer()) as? [String: Any] else {
            stack.addArrangedSubview(EmptyState(title: Say.noServer, detail: Say.noServerDetail))
            toggle.isEnabled = false
            return
        }
        toggle.isOn = eq["on"] as? Bool ?? false
        let presets = PillButton(Say.presets, nil, prominent: false)
        presets.addTarget(self, action: #selector(presetsTapped), for: .touchUpInside)
        stack.addArrangedSubview(presets)
        stack.setCustomSpacing(12, after: presets)
        let gains = eq["gains"] as? [Double] ?? []
        let hz = eq["hz"] as? [Double] ?? []
        for (i, g) in gains.enumerated() {
            let band = label(Fonts.digits(12), Theme.secondary)
            let f = i < hz.count ? hz[i] : 0
            band.text = f >= 1000 ? String(format: "%g kHz", f / 1000) : String(format: "%g Hz", f)
            band.widthAnchor.constraint(equalToConstant: 64).isActive = true
            let slider = UISlider()
            slider.minimumValue = -12
            slider.maximumValue = 12
            slider.value = Float(g)
            slider.tag = i
            slider.minimumTrackTintColor = Theme.label
            slider.maximumTrackTintColor = Theme.track
            slider.isEnabled = toggle.isOn
            slider.addTarget(self, action: #selector(moved(_:)), for: .valueChanged)
            sliders.append(slider)
            let row = UIStackView(arrangedSubviews: [band, slider])
            row.spacing = 8
            row.heightAnchor.constraint(equalToConstant: 40).isActive = true
            stack.addArrangedSubview(row)
        }
    }

    @objc private func switched() {
        _ = "eq".withCString { n in (toggle.isOn ? "true" : "false").withCString { v in nori_ios_set(n, v) } }
        sliders.forEach { $0.isEnabled = toggle.isOn }
        nori_ios_tuning(1, 1)
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

    /// The switch and sliders as the settings now hold them.
    private func paint() {
        guard let eq = takenJSON(nori_ios_equalizer()) as? [String: Any] else { return }
        toggle.isOn = eq["on"] as? Bool ?? false
        let gains = eq["gains"] as? [Double] ?? []
        for (slider, g) in zip(sliders, gains) {
            slider.isEnabled = toggle.isOn
            slider.setValue(Float(g), animated: true)
        }
    }

    @objc private func moved(_ s: UISlider) {
        nori_ios_tuning(1, 1)
        let rounded = (s.value * 2).rounded() / 2
        s.value = nori_ios_equalizer_band(UInt32(s.tag), rounded)
    }
}
