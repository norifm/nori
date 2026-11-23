import UIKit

/// Adds a profile of one of the kinds the core offers (`nori_ios_providers`), with the fields its form
/// asks: the core checks it, saves it as the active one, and the page opens it.
final class LoginPage: UIViewController, UITextFieldDelegate {
    /// A kind offered, and its form's fields by the login JSON's keys, plain or advanced.
    struct Kind {
        let id: String
        let plain: Set<String>
        let advanced: Set<String>
    }

    static let kinds: [Kind] = (takenJSON(nori_ios_providers()) as? [[String: Any]] ?? []).map { p in
        let fields = p["fields"] as? [[String: Any]] ?? []
        let named = { (advanced: Bool) in Set(fields.filter { $0["advanced"] as? Bool == advanced }.compactMap { $0["what"] as? String }) }
        return Kind(id: p["kind"] as? String ?? "", plain: named(false), advanced: named(true))
    }
    private let kinds = LoginPage.kinds
    /// The kind chosen before the form (the welcome's), whose choice the form no longer offers.
    private let given: String?
    /// What follows the profile opening; otherwise the form closes.
    var added: (() -> Void)?
    private lazy var kind = UISegmentedControl(items: kinds.map { Say.provider($0.id).name })
    private let hint = UILabel()
    private let address = UITextField()
    private let user = UITextField()
    private let password = UITextField()
    private let name = UITextField()
    private let second = UITextField()
    private let key = UITextField()
    /// The proxy as its sheet last saved it: the address as the core shows it (empty for none), and its login.
    private var proxy = (address: "", user: "", password: "")
    private lazy var proxyRow = GroupRow(Say.proxy, value: Say.proxyNone, chevron: true) { [weak self] in self?.askProxy() }
    private lazy var proxyCard = GroupCard([proxyRow], margin: 0)
    private let more = UIButton(type: .system)
    private let scroll = UIScrollView()
    private let connect = UIButton(type: .system)
    private let status = UILabel()
    private let spinner = UIActivityIndicatorView()
    private let edge = UnderEdge(Theme.background)

    init(kind: String? = nil) {
        given = kind
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Theme.background
        title = Say.addServer

        field(address, Say.serverUrl, .URL, false)
        field(user, Say.user, .default, false)
        field(password, Say.password, .default, true)
        address.returnKeyType = .next
        user.returnKeyType = .next
        password.returnKeyType = .go

        connect.setTitle(Say.connect, for: .normal)
        connect.setTitleColor(Theme.background, for: .normal)
        connect.titleLabel?.font = Fonts.font(16, .semibold, .headline)
        connect.titleLabel?.adjustsFontForContentSizeCategory = true
        connect.backgroundColor = Theme.label
        connect.layer.cornerRadius = 10
        connect.addTarget(self, action: #selector(submit), for: .touchUpInside)

        status.textColor = UIColor(rgb: 0xFFB4AB)
        status.font = Fonts.font(13, .regular, .footnote)
        status.adjustsFontForContentSizeCategory = true
        status.numberOfLines = 0
        status.textAlignment = .center

        spinner.hidesWhenStopped = true
        spinner.color = Theme.label

        kind.selectedSegmentIndex = kinds.firstIndex { $0.id == given } ?? 0
        kind.isHidden = kinds.count < 2 || given != nil
        kind.addTarget(self, action: #selector(kindChanged), for: .valueChanged)
        hint.textColor = Theme.secondary
        hint.font = Fonts.font(13, .regular, .footnote)
        hint.adjustsFontForContentSizeCategory = true
        hint.numberOfLines = 0
        hint.textAlignment = .center

        field(name, Say.nameOptional, .default, false)
        name.autocapitalizationType = .words
        field(second, Say.secondAddress, .URL, false)
        field(key, Say.apiKey, .default, true)
        more.setTitle(Say.advanced, for: .normal)
        more.tintColor = Theme.secondary
        more.contentHorizontalAlignment = .leading
        more.addTarget(self, action: #selector(moreTapped), for: .touchUpInside)
        for f in [name, second, key] {
            f.heightAnchor.constraint(equalToConstant: 44).isActive = true
        }
        kindChanged()
        let stack = UIStackView(arrangedSubviews: [kind, address, user, password, more, name, second, key, proxyCard, connect, spinner, status, hint])
        stack.axis = .vertical
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false
        // Scrolls, so the advanced fields and the button stay in reach above the keyboard on 568 pt.
        scroll.keyboardDismissMode = .onDrag
        scroll.alwaysBounceVertical = true
        scroll.translatesAutoresizingMaskIntoConstraints = false
        let name = PageTitle(title ?? "")
        name.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(name)
        view.addSubview(scroll)
        scroll.addSubview(stack)
        edge.pin(in: view, under: name.bottomAnchor)
        edge.follow(scroll)
        NotificationCenter.default.addObserver(self, selector: #selector(keyboard(_:)), name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            name.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            name.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            name.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            scroll.topAnchor.constraint(equalTo: name.bottomAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: scroll.leadingAnchor, constant: Space.gutter),
            stack.trailingAnchor.constraint(equalTo: scroll.trailingAnchor, constant: -Space.gutter),
            stack.widthAnchor.constraint(equalTo: scroll.widthAnchor, constant: -Space.gutter * 2),
            stack.topAnchor.constraint(equalTo: scroll.topAnchor, constant: 16),
            stack.bottomAnchor.constraint(equalTo: scroll.bottomAnchor, constant: -16),
            address.heightAnchor.constraint(equalToConstant: 44),
            user.heightAnchor.constraint(equalToConstant: 44),
            password.heightAnchor.constraint(equalToConstant: 44),
            connect.heightAnchor.constraint(equalToConstant: 44),
        ])
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        if textField === address {
            user.becomeFirstResponder()
        } else if textField === user {
            password.becomeFirstResponder()
        } else if textField === name {
            second.becomeFirstResponder()
        } else if textField === second {
            key.becomeFirstResponder()
        } else {
            textField.resignFirstResponder()
            submit()
        }
        return true
    }

    /// Room under the form for the keyboard, as far as it covers the scroll view.
    @objc private func keyboard(_ n: Notification) {
        guard let frame = (n.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue else { return }
        let covered = max(0, scroll.convert(scroll.bounds, to: nil).maxY - frame.minY)
        scroll.contentInset.bottom = covered
        scroll.scrollIndicatorInsets.bottom = covered
    }

    /// The boxes by the login JSON's keys, and the proxy's entry.
    private var boxes: [String: UIView] {
        ["name": name, "url": address, "user": user, "password": password, "alt": second, "key": key, "proxy": proxyCard]
    }

    /// The proxy's sheet, on what the page has or on what was typed when the core said `failed`: Save keeps
    /// what the core reads there, Cancel keeps the page's, Remove (while one is set) keeps none.
    private func askProxy(_ typed: [String]? = nil, failed: String? = nil) {
        let ask = UIAlertController(title: Say.proxy, message: failed, preferredStyle: .alert)
        ask.view.tintColor = .black
        let shown = typed ?? [proxy.address, proxy.user, proxy.password]
        for (i, hint) in [Say.proxyAddress, Say.proxyUser, Say.password].enumerated() {
            ask.addTextField { field in
                field.placeholder = hint
                field.text = shown[i]
                field.keyboardType = i == 0 ? .URL : .default
                field.isSecureTextEntry = i == 2
                field.autocapitalizationType = .none
                field.autocorrectionType = .no
            }
        }
        let save = UIAlertAction(title: Say.save, style: .default) { [weak self] _ in
            guard let self = self else { return }
            let typed = ask.textFields?.map { $0.text ?? "" } ?? ["", "", ""]
            var error: Int32 = 0
            let read = typed[0].withCString { a in typed[1].withCString { u in typed[2].withCString { p in nori_ios_proxy(a, u, p, &error) } } }
            guard let address = taken(read) else { return self.askProxy(typed, failed: Say.proxyError(error)) }
            self.proxySaved(address, typed[1], typed[2])
        }
        ask.addAction(save)
        if !proxy.address.isEmpty {
            ask.addAction(UIAlertAction(title: Say.proxyRemove, style: .destructive) { [weak self] _ in self?.proxySaved("", "", "") })
        }
        ask.addAction(UIAlertAction(title: Say.cancel, style: .cancel))
        ask.preferredAction = save
        present(ask, animated: true)
    }

    /// The proxy the sheet saved, `address` empty for none, on its row.
    private func proxySaved(_ address: String, _ user: String, _ password: String) {
        proxy = address.isEmpty ? ("", "", "") : (address, user, password)
        proxyRow.value.text = address.isEmpty ? Say.proxyNone : address
        proxyRow.accessibilityLabel = "\(Say.proxy), \(proxyRow.value.text ?? "")"
    }

    private var chosen: Kind? { kinds.indices.contains(kind.selectedSegmentIndex) ? kinds[kind.selectedSegmentIndex] : nil }

    /// The chosen kind's advanced fields: shown on asking.
    @objc private func moreTapped() {
        guard let k = chosen else { return }
        let show = k.advanced.contains { boxes[$0]?.isHidden ?? false }
        UIView.animate(withDuration: UIAccessibility.isReduceMotionEnabled ? 0 : 0.25) {
            for f in k.advanced { self.boxes[f]?.isHidden = !show }
        }
        more.setTitle(show ? Say.hideAdvanced : Say.advanced, for: .normal)
    }

    /// The chosen kind's plain fields, its advanced ones folded away, and its words.
    @objc private func kindChanged() {
        let k = chosen
        for (what, box) in boxes { box.isHidden = !(k?.plain.contains(what) ?? false) }
        more.isHidden = k?.advanced.isEmpty ?? true
        more.setTitle(Say.advanced, for: .normal)
        let words = Say.provider(k?.id ?? "")
        hint.text = words.note
        connect.setTitle(words.add, for: .normal)
        status.text = ""
    }

    @objc private func submit() {
        view.endEditing(true)
        let fields: [String: String] = [
            "kind": chosen?.id ?? "",
            "url": address.text ?? "", "user": user.text ?? "", "password": password.text ?? "",
            "name": name.text ?? "", "alt": second.text ?? "", "key": key.text ?? "",
            "proxy": proxy.address, "proxyUser": proxy.user, "proxyPassword": proxy.password,
        ]
        let form = (try? JSONSerialization.data(withJSONObject: fields)).flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
        connect.isEnabled = false
        connect.setTitle(Say.connecting, for: .normal)
        status.text = ""
        spinner.startAnimating()
        let dir = dataDirectory().path
        DispatchQueue.global(qos: .userInitiated).async {
            var detail: UnsafeMutablePointer<CChar>?
            let code = dir.withCString { d in
                form.withCString { f in
                    withUnsafeMutablePointer(to: &detail) { nori_ios_login(d, f, $0) }
                }
            }
            let extra = taken(detail)
            let opened = code == NORI_LOGIN_OK ? Core.reopen() : nil
            DispatchQueue.main.async {
                self.spinner.stopAnimating()
                if code == NORI_LOGIN_OK {
                    Core.shared.opened()
                    if opened == nil {
                        if let added = self.added { added() } else { self.navigationController?.popViewController(animated: true) }
                        return
                    }
                }
                self.status.text = opened ?? Say.failure(code, extra)
                self.connect.setTitle(Say.provider(self.chosen?.id ?? "").add, for: .normal)
                self.connect.isEnabled = true
            }
        }
    }

    private func field(_ box: UITextField, _ placeholder: String, _ keyboard: UIKeyboardType, _ secure: Bool) {
        box.placeholder = placeholder
        box.keyboardType = keyboard
        box.isSecureTextEntry = secure
        box.autocapitalizationType = .none
        box.autocorrectionType = .no
        box.keyboardAppearance = Theme.keyboard
        box.textColor = Theme.label
        box.tintColor = Theme.accent
        box.backgroundColor = Theme.row
        box.layer.cornerRadius = 10
        box.font = Fonts.font(16)
        box.adjustsFontForContentSizeCategory = true
        box.delegate = self
        box.leftView = UIView(frame: CGRect(x: 0, y: 0, width: 12, height: 44))
        box.leftViewMode = .always
        box.attributedPlaceholder = NSAttributedString(
            string: placeholder,
            attributes: [.foregroundColor: Theme.secondary]
        )
    }
}
