import UIKit

extension Notification.Name {
    /// Where sync stands, the pairing or the group's devices changed (`NORI_SYNC_CHANGED`), or the saved
    /// profiles did.
    static let noriSync = Notification.Name("noriSync")
}

/// Where sync stands, as `nori_ios_sync_state` says it.
struct SyncView {
    let d: [String: Any]

    static func read() -> SyncView { SyncView(d: takenJSON(nori_ios_sync_state()) as? [String: Any] ?? [:]) }

    var paired: Bool { d["paired"] as? Bool ?? false }
    var on: Bool { d["on"] as? Bool ?? true }
    var server: String { d["server"] as? String ?? "" }
    var gives: Bool { d["gives"] as? Bool ?? false }
    var opensShowing: Bool { (d["opens"] as? String ?? "Show") == "Show" }
    var keepsCopy: Bool { d["keepsCopy"] as? Bool ?? false }
    var synced: [String] { d["synced"] as? [String] ?? [] }
    var offers: [(a: String, b: String)] { (d["offers"] as? [[String: Any]] ?? []).map { ($0["a"] as? String ?? "", $0["b"] as? String ?? "") } }
    var devices: [[String: Any]] { d["devices"] as? [[String: Any]] ?? [] }
    var nearby: [[String: Any]] { d["nearby"] as? [[String: Any]] ?? [] }
    var pairing: Any? { d["pairing"] }
    var state: String {
        Say.syncState(d["state"] as? String ?? "", refused: d["refused"] as? String, retryMs: (d["retryMs"] as? NSNumber)?.int64Value)
    }
    /// The code shown, with its words.
    var code: (link: String, words: [String])? {
        guard let p = pairing as? [String: Any], let link = p["code"] as? String else { return nil }
        return (link, p["words"] as? [String] ?? [])
    }
    var confirm: [String: Any]? { (pairing as? [String: Any])?["confirm"] as? [String: Any] }
    /// A refused pairing: its `PairingRefused` by name, and the server's refusal with it.
    var refused: (reason: String, server: String?)? {
        guard let p = pairing as? [String: Any], let r = p["refused"] as? String else { return nil }
        return (r, p["server"] as? String)
    }
    var over: String? { (pairing as? String).flatMap { ["Paired", "Sent"].contains($0) ? $0 : nil } }
    var starting: Bool { pairing as? String == "Starting" }
}

/// Use a sync server and its address, asked for in a field; paired, the kept copy's switch. Your devices has
/// it, and the first run's last page.
func syncServerCard(_ v: SyncView, from host: UIViewController, changed: @escaping () -> Void) -> GroupCard {
    let on = UISwitch()
    on.isOn = v.on
    on.onTintColor = Theme.switchOn
    on.addAction {
        nori_ios_sync_on(on.isOn ? 1 : 0)
        changed()
    }
    var rows: [UIView] = [GroupRow(Say.syncOn, subtitle: v.paired && v.on ? v.state : nil, trailing: on)]
    let address = GroupRow(Say.syncServer, value: v.server.isEmpty ? Say.syncServerOwn : v.server, chevron: true) { [weak host] in
        let ask = UIAlertController(title: Say.syncServer, message: nil, preferredStyle: .alert)
        ask.view.tintColor = .black
        ask.addTextField { field in
            field.text = v.server
            field.placeholder = Say.syncServerOwn
            field.keyboardType = .URL
            field.autocapitalizationType = .none
            field.autocorrectionType = .no
        }
        ask.addAction(UIAlertAction(title: Say.cancel, style: .cancel))
        ask.addAction(UIAlertAction(title: Say.save, style: .default) { _ in
            (ask.textFields?.first?.text ?? "").withCString { nori_ios_sync_server($0) }
            changed()
        })
        host?.present(ask, animated: true)
    }
    address.isEnabled = v.on
    address.alpha = v.on ? 1 : 0.4
    rows.append(address)
    if v.paired && v.on {
        let keep = UISwitch()
        keep.isOn = v.keepsCopy
        keep.onTintColor = Theme.switchOn
        keep.addAction {
            nori_ios_sync_keep_copy(keep.isOn ? 1 : 0)
            changed()
        }
        rows.append(GroupRow(Say.syncKeepCopy, subtitle: Say.syncKeepCopyDetail, trailing: keep))
    }
    return GroupCard(rows)
}

/// Your devices, as Android's: this iPod and its group's others with when each was last seen, Pair a device,
/// the sync server's switch and address, the kept copy, and Reset pairing, asked first. Not paired, one line
/// says so over Pair a device. The iPod runs no plugins, so it lists none that profiles need.
final class YourDevicesPage: CardsPage {
    override func viewDidLoad() {
        super.viewDidLoad()
        title = Say.yourDevices
        NotificationCenter.default.addObserver(self, selector: #selector(lay), name: .noriSync, object: nil)
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        lay()
    }

    @objc private func lay() {
        let v = SyncView.read()
        let pair = { [weak self] in
            guard let self, v.on else { return }
            self.navigationController?.pushViewController(PairPage(), animated: true)
        }
        var views: [UIView] = []
        let pairRow = GroupRow(Say.syncPair, icon: Glyph.draw(Iconoir.plus, 20), chevron: true, tapped: pair)
        pairRow.alpha = v.on ? 1 : 0.4
        if v.paired {
            let now = Int64(Date().timeIntervalSince1970 * 1000)
            var rows: [UIView] = [GroupRow(Say.thisIPod, subtitle: v.state, icon: Glyph.device(0, 20))]
            rows += v.devices.map { d in
                GroupRow(d["name"] as? String ?? "", subtitle: Say.syncSeen((d["lastMs"] as? NSNumber)?.int64Value ?? 0, online: d["online"] as? Bool ?? false, now: now),
                         icon: Glyph.device(1, 20))
            }
            rows.append(pairRow)
            views += [GroupLabel(Say.syncSectionDevices), GroupCard(rows)]
        } else {
            let alone = label(Fonts.font(15), Theme.secondary, lines: 0)
            alone.text = Say.syncNotPaired
            views += [inset(alone), GroupCard([pairRow])]
        }
        views += [GroupLabel(Say.syncSectionServer), syncServerCard(v, from: self) { [weak self] in self?.lay() }]
        if v.paired {
            views.append(GroupCard([GroupRow(Say.syncReset) { [weak self] in self?.reset() }]))
        }
        set(views)
    }

    /// Reset pairing, asked first, naming the profiles whose sign-ins the other devices hold.
    private func reset() {
        guard let notice = takenJSON(nori_ios_sync_reset_notice()) as? [String: Any] else { return }
        let sheet = Sheet(Say.syncResetTitle, Say.syncResetText(notice["signedIn"] as? [String] ?? []))
        sheet.add(Say.syncResetConfirm, destructive: true) { [weak self] in
            nori_ios_sync_reset()
            self?.lay()
        }
        sheet.show(from: self)
    }
}

/// `v` with the page's side margins.
private func inset(_ v: UIView) -> UIView {
    let s = UIStackView(arrangedSubviews: [v])
    s.isLayoutMarginsRelativeArrangement = true
    s.layoutMargins = UIEdgeInsets(top: 8, left: Space.gutter, bottom: 12, right: Space.gutter)
    return s
}

/// Pair a device, as Android's: in no group, this iPod's code shows at once (the QR code, its three words, how
/// long it works, Copy link, a new one once it ran out); with profiles to give, it takes a device waiting
/// nearby or a pasted link. The other device's yes comes here as a question: who and how many sign-ins, the
/// three words, the group it leaves and, at a first pairing, the encrypted copy. Leaving stops a pairing that
/// did not finish.
final class PairPage: CardsPage {
    private var showing = true
    private var keep = true
    /// The code shown last: it stays while the new device waits on after a key that did not fit.
    private var shown: (link: String, words: [String])?
    private var opened = false

    override func viewDidLoad() {
        super.viewDidLoad()
        title = Say.syncPair
        NotificationCenter.default.addObserver(self, selector: #selector(lay), name: .noriSync, object: nil)
        let v = SyncView.read()
        showing = v.opensShowing
        if v.code == nil && v.confirm == nil && !v.starting {
            if showing { nori_ios_sync_code() } else { nori_ios_sync_cancel() }
        }
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        nori_ios_sync_look(1)
        lay()
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        nori_ios_sync_look(0)
        if isMovingFromParent || isBeingDismissed { nori_ios_sync_cancel() }
    }

    private func side(_ show: Bool) {
        guard show != showing else { return }
        showing = show
        shown = nil
        if show { nori_ios_sync_code() } else { nori_ios_sync_cancel() }
        lay()
    }

    private func take(_ link: String, nearby: Bool) {
        keep = true
        if nearby {
            link.withCString { nori_ios_sync_take_nearby($0) }
        } else {
            _ = taken(link.withCString { nori_ios_sync_take($0) })
        }
        lay()
    }

    @objc private func lay() {
        let v = SyncView.read()
        if let over = v.over {
            if over == "Paired" { Toast.show(Say.syncPaired) }
            nori_ios_sync_cancel()
            navigationController?.popViewController(animated: true)
            return
        }
        if let c = v.code { shown = c }
        var views: [UIView] = []
        if let c = v.confirm {
            views += question(c)
        } else {
            if v.gives {
                let sides = ChipStrip()
                sides.set([Say.syncShowCode, Say.syncPaste], chosen: showing ? 0 : 1)
                sides.picked = { [weak self] i in self?.side(i == 0) }
                views.append(sides)
            }
            views += showing ? code(v) : taking(v)
        }
        set(views)
    }

    /// This iPod's code: the QR code, its words, how long it works and Copy link; ran out, a new one.
    private func code(_ v: SyncView) -> [UIView] {
        let waitsOn = v.refused.map { ["Tampered", "Spent"].contains($0.reason) } ?? false
        if let r = v.refused, !waitsOn {
            let new = PillButton(Say.syncNewCode, Glyph.draw(Iconoir.refresh, 18))
            new.addAction { [weak self] in
                self?.shown = nil
                nori_ios_sync_code()
                self?.lay()
            }
            return [inset(line(Say.pairingRefused(r.reason, showing: true, server: r.server), Theme.label)), inset(new)]
        }
        guard let c = shown, v.code != nil || waitsOn else { return [inset(line(Say.syncStarting, Theme.secondary))] }
        var views: [UIView] = []
        if let qr = c.link.withCString({ takenJSON(nori_ios_qr($0)) }) as? [String: Any] {
            let holder = UIStackView(arrangedSubviews: [QRCode(qr)])
            holder.axis = .vertical
            holder.alignment = .center
            views.append(holder)
        }
        let words = line(c.words.joined(separator: "  "), Theme.label, Fonts.font(20, .semibold))
        words.textAlignment = .center
        views.append(inset(words))
        views.append(inset(line(Say.syncCodeWords, Theme.secondary)))
        if let r = v.refused, waitsOn { views.append(inset(line(Say.pairingRefused(r.reason, showing: true, server: r.server), Theme.label))) }
        views.append(inset(line(Say.syncCodeLasts, Theme.secondary)))
        let copy = PillButton(Say.syncCopyLink, nil)
        copy.addAction {
            UIPasteboard.general.string = c.link
            Toast.show(Say.linkCopied)
        }
        views.append(inset(copy))
        return views
    }

    /// Another device's code: those waiting nearby, then a pasted link; why the last one was refused.
    private func taking(_ v: SyncView) -> [UIView] {
        var views: [UIView] = []
        if !v.nearby.isEmpty {
            views.append(GroupLabel(Say.syncSectionNearby))
            views.append(GroupCard(v.nearby.map { d in
                let link = d["link"] as? String ?? ""
                return GroupRow(d["name"] as? String ?? "", icon: Glyph.draw(Iconoir.antennaSignal, 20), chevron: true) { [weak self] in self?.take(link, nearby: true) }
            }))
        }
        views.append(GroupCard([GroupRow(Say.syncPaste, chevron: true) { [weak self] in self?.paste() }]))
        if v.starting { views.append(inset(line(Say.syncStarting, Theme.secondary))) }
        if let r = v.refused { views.append(inset(line(Say.pairingRefused(r.reason, showing: false, server: r.server), Theme.label))) }
        return views
    }

    private func paste() {
        let ask = UIAlertController(title: Say.syncPaste, message: nil, preferredStyle: .alert)
        ask.view.tintColor = .black
        ask.addTextField { field in
            field.text = UIPasteboard.general.string
            field.placeholder = Say.syncLink
            field.keyboardType = .URL
            field.autocapitalizationType = .none
            field.autocorrectionType = .no
        }
        ask.addAction(UIAlertAction(title: Say.cancel, style: .cancel))
        ask.addAction(UIAlertAction(title: Say.syncPairAction, style: .default) { [weak self] _ in
            let link = (ask.textFields?.first?.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            if !link.isEmpty { self?.take(link, nearby: false) }
        })
        present(ask, animated: true)
    }

    /// The question on the device with the profiles.
    private func question(_ c: [String: Any]) -> [UIView] {
        let name = c["name"] as? String ?? ""
        let title = line(Say.syncConfirmTitle(name, c["profiles"] as? Int ?? 0), Theme.label, Fonts.font(17, .semibold))
        var views: [UIView] = [inset(title)]
        if let words = c["words"] as? [String] {
            let w = line(words.joined(separator: "  "), Theme.label, Fonts.font(20, .semibold))
            w.textAlignment = .center
            views += [inset(w), inset(line(Say.syncConfirmWords, Theme.secondary))]
        }
        if let leaves = c["leaves"] as? Int { views.append(inset(line(Say.syncConfirmLeaves(name, leaves), Theme.label))) }
        var rows: [UIView] = []
        if c["asksKeepCopy"] as? Bool == true {
            let keep = UISwitch()
            keep.isOn = self.keep
            keep.onTintColor = Theme.switchOn
            keep.addAction { [weak self] in self?.keep = keep.isOn }
            let row = GroupRow(Say.syncKeepCopyAsk, subtitle: Say.syncKeepCopyAskDetail, trailing: keep)
            row.title.numberOfLines = 2
            rows.append(row)
        }
        if !rows.isEmpty { views.append(GroupCard(rows)) }
        let pair = PillButton(Say.syncPairAction, nil)
        pair.addAction { [weak self] in
            nori_ios_sync_confirm((self?.keep ?? true) ? 1 : 0)
            self?.lay()
        }
        let cancel = PillButton(Say.cancel, nil, prominent: false)
        cancel.addAction { [weak self] in
            nori_ios_sync_cancel()
            self?.navigationController?.popViewController(animated: true)
        }
        views += [inset(pair), inset(cancel)]
        return views
    }

    private func line(_ text: String, _ color: UIColor, _ font: UIFont = Fonts.font(14)) -> UILabel {
        let l = label(font, color, lines: 0)
        l.text = text
        return l
    }
}

/// Sync this profile for saved profile `id` (called `name`), now `on`: switched off on a profile that syncs,
/// the other devices keep it or remove it too.
func syncProfileSheet(id: String, name: String, on: Bool, from host: UIViewController, changed: @escaping () -> Void) {
    let set = { (to: Bool, elsewhere: Bool) in
        dataDirectory().path.withCString { d in id.withCString { nori_ios_profile_sync(d, $0, to ? 1 : 0, elsewhere ? 1 : 0) } }
        changed()
    }
    let sheet = Sheet(Say.syncProfile)
    sheet.add(Say.on, checked: on) { if !on { set(true, false) } }
    sheet.add(Say.off, checked: !on, stays: true) { [weak host] in
        guard on, let host else { return }
        let v = SyncView.read()
        guard v.paired && v.synced.contains(id) else {
            sheet.dismiss(animated: false)
            return set(false, false)
        }
        sheet.dismiss(animated: false) {
            let ask = Sheet(Say.syncProfileOffTitle(name), Say.syncProfileOffText)
            ask.add(Say.syncProfileOffRemove, destructive: true) { set(false, true) }
            ask.add(Say.syncProfileOffKeep) { set(false, false) }
            ask.show(from: host)
        }
    }
    sheet.show(from: host)
}
