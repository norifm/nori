import UIKit

/// Settings → Servers: the saved ones with their marks, the active one ticked. A tap switches to it at
/// once; a tap on its mark chooses its icon; a swipe forgets it; the last row adds one.
final class ServersPage: UITableViewController {
    fileprivate struct Saved {
        let id: String
        let label: String
        let url: String
        let active: Bool
        /// A plugin's profile, which never opens here.
        let waits: Bool
        let kind: String
        let icon: ProfileIcon
        /// What its circle shows for the letter; empty for none.
        let letter: String
    }

    private var servers: [Saved] = []
    private var switching = false

    override func viewDidLoad() {
        super.viewDidLoad()
        title = Say.servers
        navigationItem.largeTitleDisplayMode = .never
        dress(tableView)
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        load()
    }

    private func load() {
        DispatchQueue.global(qos: .userInitiated).async {
            let rows = dataDirectory().path.withCString { takenJSON(nori_ios_servers($0)) } as? [[String: Any]] ?? []
            let saved = rows.map {
                Saved(id: $0["id"] as? String ?? "",
                      label: ($0["label"] as? String).flatMap { $0.isEmpty ? nil : $0 } ?? Say.provider($0["kind"] as? String ?? "").name,
                      url: $0["url"] as? String ?? "", active: $0["active"] as? Bool ?? false,
                      waits: $0["waits"] as? Bool ?? false, kind: $0["kind"] as? String ?? "",
                      icon: ($0["icon"] as? String).flatMap(ProfileIcon.init(rawValue:)) ?? .letter,
                      letter: $0["letter"] as? String ?? "")
            }
            DispatchQueue.main.async {
                self.servers = saved
                self.tableView.reloadData()
            }
        }
    }

    override func numberOfSections(in tableView: UITableView) -> Int { 2 }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        section == 0 ? servers.count : 1
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        if indexPath.section == 1 {
            let cell = row(tableView, id: "add")
            cell.textLabel?.text = Say.addServer
            cell.accessoryType = .disclosureIndicator
            return cell
        }
        let cell = row(tableView, id: "server", style: .subtitle)
        let s = servers[indexPath.row]
        cell.textLabel?.text = s.label
        cell.detailTextLabel?.text = s.waits ? Say.profileWaitsNoPlugins : s.url
        cell.accessoryType = s.active ? .checkmark : .none
        cell.tintColor = Theme.accent
        cell.imageView?.image = ProfileMark.image(s.icon, letter: s.letter, side: ProfileMark.side)
        if let mark = cell.imageView, mark.gestureRecognizers?.isEmpty ?? true {
            mark.isUserInteractionEnabled = true
            mark.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(tappedMark(_:))))
        }
        cell.accessibilityCustomActions = [SpokenAction(Say.profileIcon) { [weak self] in self?.chooseIcon(s) }]
        return cell
    }

    @objc private func tappedMark(_ tap: UITapGestureRecognizer) {
        guard let path = tableView.indexPathForRow(at: tap.location(in: tableView)), path.section == 0 else { return }
        chooseIcon(servers[path.row])
    }

    private func chooseIcon(_ s: Saved) {
        navigationController?.pushViewController(ProfileIconPage(s), animated: true)
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        if indexPath.section == 1 {
            navigationController?.pushViewController(LoginPage(), animated: true)
            return
        }
        let s = servers[indexPath.row]
        guard !s.active, !switching else { return }
        switchTo { dataDirectory().path.withCString { d in s.id.withCString { _ = nori_ios_pick_server(d, $0) } } }
    }

    override func tableView(_ tableView: UITableView, canEditRowAt indexPath: IndexPath) -> Bool {
        indexPath.section == 0
    }

    override func tableView(_ tableView: UITableView, commit editingStyle: UITableViewCell.EditingStyle, forRowAt indexPath: IndexPath) {
        guard editingStyle == .delete, !switching else { return }
        let s = servers[indexPath.row]
        let sheet = Sheet(Say.forgetServer(s.label), Say.forgetServerNote)
        sheet.add(Say.forget, destructive: true) { [weak self] in
            let forget = { dataDirectory().path.withCString { d in s.id.withCString { nori_ios_remove_server(d, $0) } } }
            if s.active {
                self?.switchTo(forget)
            } else {
                DispatchQueue.global(qos: .userInitiated).async {
                    forget()
                    DispatchQueue.main.async { self?.load() }
                }
            }
        }
        sheet.show(from: self)
    }

    /// Runs `change` on the saved list, then reopens on whichever server is active after it.
    private func switchTo(_ change: @escaping () -> Void) {
        switching = true
        let spinner = UIActivityIndicatorView(style: Theme.dark ? .white : .gray)
        spinner.startAnimating()
        navigationItem.rightBarButtonItem = UIBarButtonItem(customView: spinner)
        DispatchQueue.global(qos: .userInitiated).async {
            change()
            let left = (dataDirectory().path.withCString { takenJSON(nori_ios_servers($0)) } as? [Any])?.count ?? 0
            let failed = Core.reopen()
            DispatchQueue.main.async {
                self.switching = false
                self.navigationItem.rightBarButtonItem = nil
                Core.shared.opened()
                if let failed, left > 0 { Toast.show(failed) }
                self.load()
            }
        }
    }
}

/// What a profile's circle shows (`ProfileIcon` in the settings), by the id the core gives it.
enum ProfileIcon: String {
    case letter, provider, sing, surprise, talking
    case lookUp = "look-up"
    case satisfied, think

    /// Every icon, in the picker's order.
    static let all = (takenJSON(nori_ios_profile_icons()) as? [String] ?? []).compactMap(ProfileIcon.init(rawValue:))
}

/// A profile's mark on a circle, as its icon says: its letter, its provider's mark (the server's glyph:
/// the iPod has no plugin icons), or a face. With no letter to show, the provider's.
enum ProfileMark {
    /// A profile row's.
    static let side: CGFloat = 36

    static func image(_ icon: ProfileIcon, letter: String, side: CGFloat) -> UIImage {
        UIGraphicsImageRenderer(size: CGSize(width: side, height: side)).image { _ in
            Theme.track.setFill()
            UIBezierPath(ovalIn: CGRect(x: 0, y: 0, width: side, height: side)).fill()
            if icon == .letter, !letter.isEmpty {
                let text = NSAttributedString(string: letter, attributes: [.font: Fonts.font(side * 0.42, .semibold), .foregroundColor: Theme.label])
                let size = text.size()
                text.draw(at: CGPoint(x: (side - size.width) / 2, y: (side - size.height) / 2))
            } else {
                Glyph.draw(glyph(icon), side * 0.6).tinted(Theme.label).draw(at: CGPoint(x: side * 0.2, y: side * 0.2))
            }
        }
    }

    private static func glyph(_ icon: ProfileIcon) -> [GlyphPart] {
        switch icon {
        case .sing: return Iconoir.emojiSingRight
        case .surprise: return Iconoir.emojiSurpriseAlt
        case .talking: return Iconoir.emojiTalkingHappy
        case .lookUp: return Iconoir.emojiLookUp
        case .satisfied: return Iconoir.emojiSatisfied
        case .think: return Iconoir.emojiThinkRight
        case .letter, .provider: return Iconoir.server
        }
    }
}

/// A profile's icon: every one its circle can show, as it would show it, the chosen one ticked. A tap
/// keeps it and goes back.
final class ProfileIconPage: UITableViewController {
    private let profile: ServersPage.Saved
    private var chosen: ProfileIcon
    private let icons = ProfileIcon.all

    fileprivate init(_ profile: ServersPage.Saved) {
        self.profile = profile
        chosen = profile.icon
        super.init(style: .plain)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        title = Say.profileIcon
        navigationItem.largeTitleDisplayMode = .never
        dress(tableView)
    }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { icons.count }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = row(tableView, id: "icon")
        let icon = icons[indexPath.row]
        cell.imageView?.image = ProfileMark.image(icon, letter: profile.letter, side: ProfileMark.side)
        cell.textLabel?.text = Say.profileIconName(icon, provider: Say.provider(profile.kind).name)
        cell.accessoryType = icon == chosen ? .checkmark : .none
        cell.tintColor = Theme.accent
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        chosen = icons[indexPath.row]
        tableView.reloadData()
        let (id, icon) = (profile.id, chosen.rawValue)
        DispatchQueue.global(qos: .userInitiated).async {
            dataDirectory().path.withCString { d in id.withCString { i in icon.withCString { nori_ios_set_server_icon(d, i, $0) } } }
            DispatchQueue.main.async { self.navigationController?.popViewController(animated: true) }
        }
    }
}

/// A VoiceOver action that runs `run` (iOS 12's actions take a target and a selector).
final class SpokenAction: UIAccessibilityCustomAction {
    private let run: () -> Void

    init(_ name: String, _ run: @escaping () -> Void) {
        self.run = run
        super.init(name: name, target: nil, selector: #selector(SpokenAction.fire))
        target = self
    }

    @objc private func fire() -> Bool {
        run()
        return true
    }
}
