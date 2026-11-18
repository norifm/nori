import UIKit

/// Settings → Servers: the saved ones in their order with their marks, the active one ticked where it
/// stands. A tap switches to it at once; a tap on its mark chooses its icon; holding and dragging one moves
/// it in the order (the core's `profilePlace`); a swipe renames or forgets it; the last row adds one.
final class ServersPage: TablePage, UITableViewDragDelegate, UITableViewDropDelegate {
    fileprivate struct Saved {
        let id: String
        let label: String
        /// The name it was given; empty, it is called after who signs in or its address.
        let name: String
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
        dress(tableView)
        tableView.dragDelegate = self
        tableView.dropDelegate = self
        tableView.dragInteractionEnabled = true
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
                      name: $0["name"] as? String ?? "",
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
        cell.accessibilityCustomActions = [SpokenAction(Say.profileIcon) { [weak self] in self?.chooseIcon(s) },
                                           SpokenAction(Say.rename) { [weak self] in self?.rename(s) },
                                           SpokenAction(Say.scrobbling) { [weak self] in self?.scrobbling(s) }]
        return cell
    }

    @objc private func tappedMark(_ tap: UITapGestureRecognizer) {
        guard let path = tableView.indexPathForRow(at: tap.location(in: tableView)), path.section == 0 else { return }
        chooseIcon(servers[path.row])
    }

    private func scrobbling(_ s: Saved) {
        navigationController?.pushViewController(ScrobblingPage(id: s.id), animated: true)
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

    override func tableView(_ tableView: UITableView, trailingSwipeActionsConfigurationForRowAt indexPath: IndexPath) -> UISwipeActionsConfiguration? {
        guard indexPath.section == 0, !switching else { return nil }
        let s = servers[indexPath.row]
        let forget = UIContextualAction(style: .destructive, title: Say.forget) { [weak self] _, _, done in
            done(false)
            self?.forget(s)
        }
        let rename = UIContextualAction(style: .normal, title: Say.rename) { [weak self] _, _, done in
            done(true)
            self?.rename(s)
        }
        rename.backgroundColor = Theme.secondary
        let scrobbling = UIContextualAction(style: .normal, title: Say.scrobbling) { [weak self] _, _, done in
            done(true)
            self?.scrobbling(s)
        }
        scrobbling.backgroundColor = Theme.secondary
        let actions = UISwipeActionsConfiguration(actions: [forget, rename, scrobbling])
        actions.performsFirstActionWithFullSwipe = false
        return actions
    }

    /// Asks for `s`'s new name; left blank, it is called after who signs in or its address again.
    private func rename(_ s: Saved) {
        let ask = UIAlertController(title: Say.renameServer, message: nil, preferredStyle: .alert)
        ask.view.tintColor = .black
        ask.addTextField { field in
            field.text = s.name
            field.placeholder = s.label
            field.autocapitalizationType = .words
            field.clearButtonMode = .whileEditing
            field.returnKeyType = .done
        }
        ask.addAction(UIAlertAction(title: Say.cancel, style: .cancel))
        ask.addAction(UIAlertAction(title: Say.save, style: .default) { [weak self] _ in
            let name = ask.textFields?.first?.text ?? ""
            DispatchQueue.global(qos: .userInitiated).async {
                dataDirectory().path.withCString { d in s.id.withCString { i in name.withCString { nori_ios_rename_server(d, i, $0) } } }
                DispatchQueue.main.async { self?.load() }
            }
        })
        present(ask, animated: true)
    }

    private func forget(_ s: Saved) {
        guard !switching else { return }
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

    // MARK: moving a server: held and dragged among the saved ones

    func tableView(_ tableView: UITableView, itemsForBeginning session: UIDragSession, at indexPath: IndexPath) -> [UIDragItem] {
        guard indexPath.section == 0, !switching else { return [] }
        return [UIDragItem(itemProvider: NSItemProvider())]
    }

    func tableView(_ tableView: UITableView, dropSessionDidUpdate session: UIDropSession, withDestinationIndexPath destination: IndexPath?) -> UITableViewDropProposal {
        guard session.localDragSession != nil, destination?.section == 0 else { return UITableViewDropProposal(operation: .forbidden) }
        return UITableViewDropProposal(operation: .move, intent: .insertAtDestinationIndexPath)
    }

    func tableView(_ tableView: UITableView, performDropWith coordinator: UITableViewDropCoordinator) {}

    override func tableView(_ tableView: UITableView, canMoveRowAt indexPath: IndexPath) -> Bool {
        indexPath.section == 0
    }

    override func tableView(_ tableView: UITableView, targetIndexPathForMoveFromRowAt source: IndexPath, toProposedIndexPath proposed: IndexPath) -> IndexPath {
        proposed.section == 0 ? proposed : IndexPath(row: servers.count - 1, section: 0)
    }

    override func tableView(_ tableView: UITableView, moveRowAt source: IndexPath, to destination: IndexPath) {
        guard source != destination else { return }
        let moved = servers.remove(at: source.row)
        servers.insert(moved, at: destination.row)
        let place = "\(moved.id):\(destination.row)"
        DispatchQueue.global(qos: .userInitiated).async {
            _ = "profilePlace".withCString { n in place.withCString { v in nori_ios_set(n, v) } }
            DispatchQueue.main.async { self.load() }
        }
    }

    /// Runs `change` on the saved list, then reopens on whichever server is active after it.
    private func switchTo(_ change: @escaping () -> Void) {
        switching = true
        let spinner = UIActivityIndicatorView(style: Theme.dark ? .white : .gray)
        spinner.startAnimating()
        nameEnd = [spinner]
        DispatchQueue.global(qos: .userInitiated).async {
            change()
            let left = (dataDirectory().path.withCString { takenJSON(nori_ios_servers($0)) } as? [Any])?.count ?? 0
            let failed = Core.reopen()
            DispatchQueue.main.async {
                self.switching = false
                self.nameEnd = []
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
final class ProfileIconPage: TablePage {
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

/// A profile's scrobbling accounts, whatever it plays from: each with its switch, a swipe removes it; the
/// last rows add one, signed in by the core (`Client::sign_in_scrobbler`), Last.fm after its page allowed
/// nori in Safari (`Client::scrobbler_approval`).
final class ScrobblingPage: TablePage {
    private struct Account {
        let id: String
        let kind: String
        let detail: String
        let on: Bool
    }

    /// A kind of account to add: the fields it asks, by id; whether it signs in on its page; where its
    /// fields come from, if anywhere.
    private struct Kind {
        let id: String
        let fields: [String]
        let browser: Bool
        let help: URL?
    }

    private let profile: String
    /// The server itself, first: 1 told what plays, 0 not, -1 where it takes none.
    private var itself: Int32 = -1
    private var accounts: [Account] = []
    private let kinds: [Kind] = (takenJSON(nori_ios_scrobbler_kinds()) as? [[String: Any]] ?? []).map {
        Kind(id: $0["kind"] as? String ?? "", fields: ($0["fields"] as? [[String: Any]] ?? []).map { $0["id"] as? String ?? "" },
             browser: $0["signIn"] as? String == "browser", help: ($0["help"] as? String).flatMap(URL.init(string:)))
    }

    init(id: String) {
        profile = id
        super.init(style: .grouped)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        title = Say.scrobbling
        dress(tableView)
        load()
    }

    private func load() {
        let id = profile
        DispatchQueue.global(qos: .userInitiated).async {
            let rows = dataDirectory().path.withCString { d in id.withCString { takenJSON(nori_ios_scrobblers(d, $0)) } } as? [[String: Any]] ?? []
            let itself = dataDirectory().path.withCString { d in id.withCString { nori_ios_scrobble_itself(d, $0) } }
            let accounts = rows.map {
                Account(id: $0["id"] as? String ?? "", kind: $0["kind"] as? String ?? "",
                        detail: [$0["user"] as? String ?? "", $0["host"] as? String ?? ""].filter { !$0.isEmpty }.joined(separator: " · "),
                        on: $0["on"] as? Bool ?? false)
            }
            DispatchQueue.main.async {
                self.itself = itself
                self.accounts = accounts
                self.tableView.reloadData()
            }
        }
    }

    override func numberOfSections(in tableView: UITableView) -> Int { 2 }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        section == 0 ? first + accounts.count : kinds.count
    }

    /// Rows before the accounts: the server itself, where it takes plays.
    private var first: Int { itself >= 0 ? 1 : 0 }

    override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? {
        section == 1 ? Say.addScrobblerTitle : nil
    }

    override func tableView(_ tableView: UITableView, titleForFooterInSection section: Int) -> String? {
        section == 1 ? Say.scrobblingNote : nil
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        if indexPath.section == 1 {
            let cell = row(tableView, id: "add", style: .subtitle)
            let kind = kinds[indexPath.row].id
            cell.textLabel?.text = Say.scrobbler(kind)
            cell.detailTextLabel?.text = Say.scrobblerCost(kind)
            cell.accessoryType = .disclosureIndicator
            return cell
        }
        if indexPath.row < first {
            let cell = row(tableView, id: "itself", style: .subtitle)
            cell.textLabel?.text = Say.scrobblerServer
            cell.detailTextLabel?.text = Say.scrobblerBuiltIn
            let toggle = UISwitch()
            toggle.isOn = itself == 1
            toggle.onTintColor = Theme.switchOn
            let id = profile
            toggle.addAction { [weak self] in
                let on: Int32 = toggle.isOn ? 1 : 0
                DispatchQueue.global(qos: .userInitiated).async {
                    dataDirectory().path.withCString { d in id.withCString { nori_ios_switch_scrobble_itself(d, $0, on) } }
                    DispatchQueue.main.async { self?.load() }
                }
            }
            cell.accessoryView = toggle
            cell.selectionStyle = .none
            return cell
        }
        let cell = row(tableView, id: "account", style: .subtitle)
        let a = accounts[indexPath.row - first]
        cell.textLabel?.text = Say.scrobbler(a.kind)
        cell.detailTextLabel?.text = a.detail
        let toggle = UISwitch()
        toggle.isOn = a.on
        toggle.onTintColor = Theme.switchOn
        let (id, account) = (profile, a.id)
        toggle.addAction { [weak self] in
            let on: Int32 = toggle.isOn ? 1 : 0
            DispatchQueue.global(qos: .userInitiated).async {
                dataDirectory().path.withCString { d in id.withCString { i in account.withCString { nori_ios_switch_scrobbler(d, i, $0, on) } } }
                DispatchQueue.main.async { self?.load() }
            }
        }
        cell.accessoryView = toggle
        cell.selectionStyle = .none
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        if indexPath.section == 1 { add(kinds[indexPath.row]) }
    }

    override func tableView(_ tableView: UITableView, trailingSwipeActionsConfigurationForRowAt indexPath: IndexPath) -> UISwipeActionsConfiguration? {
        guard indexPath.section == 0, indexPath.row >= first else { return nil }
        let (id, account) = (profile, accounts[indexPath.row - first].id)
        let remove = UIContextualAction(style: .destructive, title: Say.remove) { [weak self] _, _, done in
            done(true)
            DispatchQueue.global(qos: .userInitiated).async {
                dataDirectory().path.withCString { d in id.withCString { i in account.withCString { nori_ios_remove_scrobbler(d, i, $0) } } }
                DispatchQueue.main.async { self?.load() }
            }
        }
        return UISwipeActionsConfiguration(actions: [remove])
    }

    /// Asks for the fields a `kind` account signs in with, then signs in, or for one signing in on its
    /// page opens it first; a failure says why and asks again.
    private func add(_ kind: Kind, failed: String? = nil) {
        let ask = UIAlertController(title: Say.addScrobbler(kind.id), message: failed ?? Say.scrobblerHint(kind.id), preferredStyle: .alert)
        ask.view.tintColor = .black
        for id in kind.fields {
            ask.addTextField { field in
                field.placeholder = Say.scrobblerField(id)
                field.isSecureTextEntry = ["token", "secret", "password"].contains(id)
                field.autocapitalizationType = .none
                field.autocorrectionType = .no
            }
        }
        if let help = kind.help {
            ask.addAction(UIAlertAction(title: Say.scrobblerHelp(kind.id), style: .default) { [weak self] _ in
                UIApplication.shared.openURL(help)
                self?.add(kind)
            })
        }
        ask.addAction(UIAlertAction(title: kind.browser ? Say.scrobblerContinue(kind.id) : Say.add, style: .default) { [weak self] _ in
            let typed = ask.textFields?.map { $0.text ?? "" } ?? []
            let form: [String: Any] = ["kind": kind.id, "fields": Dictionary(uniqueKeysWithValues: zip(kind.fields, typed))]
            kind.browser ? self?.approve(kind, form) : self?.signIn(kind, form)
        })
        ask.addAction(UIAlertAction(title: Say.cancel, style: .cancel))
        present(ask, animated: true)
    }

    private func json(_ form: [String: Any]) -> String {
        (try? JSONSerialization.data(withJSONObject: form)).flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
    }

    /// Asks for the token to allow on the service's page, opens the page in Safari, and waits for Done.
    private func approve(_ kind: Kind, _ form: [String: Any]) {
        let text = json(form)
        busy(true)
        DispatchQueue.global(qos: .userInitiated).async {
            var code: Int32 = NORI_LOGIN_OK
            var detail: UnsafeMutablePointer<CChar>?
            let got = text.withCString { f in withUnsafeMutablePointer(to: &detail) { nori_ios_scrobbler_approval(f, &code, $0) } }
            let extra = taken(detail)
            let approval = takenJSON(got) as? [String: String]
            DispatchQueue.main.async {
                self.busy(false)
                guard let token = approval?["token"], let page = approval?["page"].flatMap(URL.init(string:)) else {
                    return self.add(kind, failed: Say.failure(code, extra))
                }
                UIApplication.shared.openURL(page)
                var approved = form
                approved["approved"] = token
                let done = UIAlertController(title: Say.addScrobbler(kind.id), message: Say.scrobblerAllow(kind.id), preferredStyle: .alert)
                done.view.tintColor = .black
                done.addAction(UIAlertAction(title: Say.cancel, style: .cancel))
                done.addAction(UIAlertAction(title: Say.done, style: .default) { [weak self] _ in self?.signIn(kind, approved) })
                self.present(done, animated: true)
            }
        }
    }

    private func signIn(_ kind: Kind, _ form: [String: Any]) {
        let (id, text) = (profile, json(form))
        busy(true)
        DispatchQueue.global(qos: .userInitiated).async {
            var detail: UnsafeMutablePointer<CChar>?
            let code = dataDirectory().path.withCString { d in
                id.withCString { i in text.withCString { f in withUnsafeMutablePointer(to: &detail) { nori_ios_add_scrobbler(d, i, f, $0) } } }
            }
            let extra = taken(detail)
            DispatchQueue.main.async {
                self.busy(false)
                if code == NORI_LOGIN_OK {
                    self.load()
                } else {
                    self.add(kind, failed: Say.failure(code, extra))
                }
            }
        }
    }

    /// A spinner at the title's end while the network is asked.
    private func busy(_ on: Bool) {
        guard on else { return nameEnd = [] }
        let spinner = UIActivityIndicatorView(style: Theme.dark ? .white : .gray)
        spinner.startAnimating()
        nameEnd = [spinner]
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
