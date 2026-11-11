import AVKit
import MediaPlayer
import UIKit

/// The system's output picker, which has no call of its own: its button is pressed on the caller's behalf.
final class OutputPicker {
    private let picker = AVRoutePickerView()

    /// Puts the picker into `host`, where it must be for its button to answer, out of sight.
    func attach(to host: UIView) {
        picker.alpha = 0.011
        // Out of sight and out of the way: the host's own touches are its own.
        picker.isUserInteractionEnabled = false
        picker.frame = CGRect(x: 0, y: 0, width: 44, height: 44)
        host.addSubview(picker)
    }

    func show() {
        picker.subviews.compactMap { $0 as? UIButton }.first?.sendActions(for: .touchUpInside)
    }
}

/// The player's output button, as Android's sheet and in the song menu's look: this iPod and the account's
/// other devices with nori, each with what it plays, the one playing ticked; then the iPod's own outputs
/// (the system's picker), joining someone's jam, then the devices paired with this iPod (a tap picks one that
/// answers, its × forgets it after asking) and pairing another. A tap on a device moves the music there. Open, the other devices are
/// followed.
final class DevicesSheet: Sheet {
    private let outputs = OutputPicker()
    private var pairing: String?

    init() { super.init(Say.playOn) }
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        outputs.attach(to: view)
        NotificationCenter.default.addObserver(self, selector: #selector(read), name: .noriDevices, object: nil)
        read()
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        nori_ios_remote_watch(1)
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        nori_ios_remote_watch(0)
        (presentingViewController as? PlayerCard)?.changed()
    }

    @objc private func read() {
        let d = takenJSON(nori_ios_remote_devices()) as? [String: Any] ?? [:]
        let devices = d["devices"] as? [[String: Any]] ?? []
        pairing = taken(nori_ios_pairing())
        let paired = takenJSON(nori_ios_paired()) as? [[String: Any]] ?? []
        refill {
            add(Say.thisIPod, glyph: Glyph.device(0, 20), checked: d["here"] as? Bool ?? true) { "".withCString { nori_ios_remote_pick($0) } }
            for device in devices {
                let song = [device["title"] as? String, device["artist"] as? String].compactMap { $0 }.filter { !$0.isEmpty }
                let playing = device["playing"] as? Bool ?? false
                let line = Say.refused(device["refused"] as? Int ?? 0) ?? (playing && !song.isEmpty ? song.joined(separator: " · ") : Say.deviceIdle)
                let id = device["id"] as? String ?? ""
                add(device["name"] as? String ?? "", subtitle: line, glyph: Glyph.device(device["kind"] as? Int ?? 0, 20),
                    checked: device["active"] as? Bool == true) { id.withCString { nori_ios_remote_pick($0) } }
            }
            add(Say.output, glyph: Glyph.output(OutputLook.now().glyph, 20), more: true, stays: true) { [weak self] in self?.outputs.show() }
            add(Say.joinJam, glyph: Glyph.jam, more: true, stays: true) { [weak self] in
                if let self { JamJoin.ask(from: self) }
            }
            if pairing != nil {
                for device in paired {
                    let (id, name) = (device["id"] as? String ?? "", device["name"] as? String ?? "")
                    let reach = device["reach"] as? Int ?? 2
                    let seen = Say.seen(reach: reach, days: device["days"] as? Int ?? 0)
                    // Picked as a device above while it answers; the × forgets it.
                    add(name, subtitle: Say.paired(seen), glyph: Glyph.device(device["kind"] as? Int ?? 0, 20), checked: device["active"] as? Bool == true,
                        enabled: reach == 0, more: true, remove: (Say.forgetDevice(name), { [weak self] in self?.forget(id, name) })) {
                        id.withCString { nori_ios_remote_pick($0) }
                    }
                }
                add(Say.pairDevice, glyph: Glyph.draw(Iconoir.plus, 20), more: true, stays: true) { [weak self] in self?.pair() }
            }
        }
    }

    /// This iPod's code to copy to the other device, or the other's pasted here; NORI_REPORT_PAIRED says
    /// how pairing went.
    private func pair() {
        guard let code = pairing else { return }
        let ask = UIAlertController(title: Say.pairDevice, message: Say.pairHow, preferredStyle: .alert)
        ask.addTextField { $0.placeholder = "nori://pair…"; $0.keyboardType = .URL; $0.autocapitalizationType = .none; $0.autocorrectionType = .no }
        ask.addAction(UIAlertAction(title: Say.pairCopy, style: .default) { _ in
            UIPasteboard.general.string = code
            Toast.show(Say.linkCopied)
        })
        ask.addAction(UIAlertAction(title: Say.pairAccept, style: .default) { [weak ask] _ in
            let link = ask?.textFields?.first?.text ?? ""
            switch link.withCString({ nori_ios_pair($0) }) {
            case NORI_PAIR_STARTED: Toast.show(Say.pairing)
            case NORI_PAIR_NOT_A_CODE: Toast.show(Say.pairFailed(NORI_PAIR_FAILED_NOT_A_CODE, name: ""))
            default: break
            }
        })
        ask.addAction(UIAlertAction(title: Say.cancel, style: .cancel))
        present(ask, animated: true)
    }

    /// Forgets paired device `id`, asked first.
    private func forget(_ id: String, _ name: String) {
        let ask = UIAlertController(title: Say.forgetDevice(name), message: Say.forgetHow, preferredStyle: .alert)
        ask.addAction(UIAlertAction(title: Say.cancel, style: .cancel))
        ask.addAction(UIAlertAction(title: Say.forget, style: .destructive) { [weak self] _ in
            id.withCString { nori_ios_forget($0) }
            self?.read()
        })
        present(ask, animated: true)
    }
}

/// Joining someone's jam with the invite link its host sent: the guest profile opens once joined
/// (`Core`'s report 20).
enum JamJoin {
    static func ask(from host: UIViewController) {
        let ask = UIAlertController(title: Say.joinJam, message: Say.joinJamHow, preferredStyle: .alert)
        ask.view.tintColor = .black
        ask.addTextField { field in
            field.placeholder = Say.inviteLink
            field.keyboardType = .URL
            field.autocapitalizationType = .none
            field.autocorrectionType = .no
            field.returnKeyType = .join
        }
        ask.addAction(UIAlertAction(title: Say.cancel, style: .cancel))
        ask.addAction(UIAlertAction(title: Say.join, style: .default) { _ in
            join(ask.textFields?.first?.text ?? "")
        })
        host.present(ask, animated: true)
    }

    /// Joins the jam `link` invites to (an https invite, or the app's own nori:// one).
    static func join(_ link: String) {
        switch link.withCString({ l in Say.jamGuest.withCString { nori_ios_jam_join(l, $0) } }) {
        case NORI_JOIN_STARTED: Toast.show(Say.joining)
        case NORI_JOIN_NOT_AN_INVITE: Toast.show(Say.notAnInvite)
        case NORI_JOIN_OWN: Toast.show(Say.ownJam)
        case NORI_JOIN_ENDED: Toast.show(Say.jamInviteEnded)
        case NORI_JOIN_CLOSED: Core.shared.inviteWaiting = link
        default: break
        }
    }
}

/// Remote control's mDNS through the system's Bonjour: this iPod's door announced while remote control
/// is on, the account's other doors looked for while "Play on" is open. The library asks from its own
/// threads; NetService runs on the main run loop.
final class Bonjour: NSObject, NetServiceDelegate, NetServiceBrowserDelegate {
    static let shared = Bonjour()
    private static let type = "_nori._tcp."
    private var announced: NetService?
    private var browser: NetServiceBrowser?
    /// Found and being resolved or resolved, by name: kept so their answers arrive.
    private var found: [String: NetService] = [:]

    func start() {
        nori_ios_on_bonjour({ name, port, txt in
            let name = name.map { String(cString: $0) }
            let txt = txt.map { String(cString: $0) } ?? "{}"
            DispatchQueue.main.async { Bonjour.shared.announce(name, port, txt) }
        }, { on in
            DispatchQueue.main.async { Bonjour.shared.browse(on != 0) }
        })
    }

    private func announce(_ name: String?, _ port: UInt16, _ txt: String) {
        announced?.stop()
        announced = nil
        guard let name else { return }
        let service = NetService(domain: "local.", type: Bonjour.type, name: name, port: Int32(port))
        let fields = (try? JSONSerialization.jsonObject(with: Data(txt.utf8))) as? [String: String] ?? [:]
        service.setTXTRecord(NetService.data(fromTXTRecord: fields.mapValues { Data($0.utf8) }))
        service.publish()
        announced = service
    }

    private func browse(_ on: Bool) {
        browser?.stop()
        browser = nil
        found.values.forEach { $0.stop() }
        found = [:]
        guard on else { return }
        let b = NetServiceBrowser()
        b.delegate = self
        b.searchForServices(ofType: Bonjour.type, inDomain: "local.")
        browser = b
    }

    func netServiceBrowser(_ browser: NetServiceBrowser, didFind service: NetService, moreComing: Bool) {
        found[service.name] = service
        service.delegate = self
        service.resolve(withTimeout: 5)
    }

    func netServiceBrowser(_ browser: NetServiceBrowser, didRemove service: NetService, moreComing: Bool) {
        found[service.name] = nil
        service.name.withCString { nori_ios_lan_lost($0) }
    }

    /// Every IPv4 address it resolved to goes to the core, which tries those on this iPod's network first.
    func netServiceDidResolveAddress(_ sender: NetService) {
        let hosts = (sender.addresses ?? []).compactMap(Bonjour.ipv4)
        guard !hosts.isEmpty else { return }
        let text = { (object: Any) in (try? JSONSerialization.data(withJSONObject: object)).flatMap { String(data: $0, encoding: .utf8) } }
        let fields = NetService.dictionary(fromTXTRecord: sender.txtRecordData() ?? Data())
            .compactMapValues { String(data: $0, encoding: .utf8) }
        let (json, addresses) = (text(fields) ?? "{}", text(hosts) ?? "[]")
        sender.name.withCString { name in
            addresses.withCString { h in json.withCString { nori_ios_lan_found(name, h, UInt16(sender.port), $0) } }
        }
    }

    /// An IPv4 socket address as text.
    private static func ipv4(_ address: Data) -> String? {
        guard address.count >= MemoryLayout<sockaddr_in>.size else { return nil }
        var sin = sockaddr_in()
        _ = withUnsafeMutableBytes(of: &sin) { address.copyBytes(to: $0, count: MemoryLayout<sockaddr_in>.size) }
        guard sin.sin_family == sa_family_t(AF_INET) else { return nil }
        var text = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
        guard inet_ntop(AF_INET, &sin.sin_addr, &text, socklen_t(INET_ADDRSTRLEN)) != nil else { return nil }
        return String(cString: text)
    }
}

/// The system volume slider, kept in the window so it stays connected to the output route. Hidden, as one
/// that counts as shown would take the place of the system's volume popup.
final class SystemVolume {
    private let view = MPVolumeView(frame: CGRect(x: -200, y: -200, width: 100, height: 40))

    init(in window: UIWindow) {
        view.isHidden = true
        view.isUserInteractionEnabled = false
        window.addSubview(view)
        view.layoutIfNeeded()
    }

    func set(_ fraction: Float) {
        guard let slider = view.subviews.lazy.compactMap({ $0 as? UISlider }).first else { return }
        slider.setValue(min(1, max(0, fraction)), animated: false)
        slider.sendActions(for: .valueChanged)
    }
}
