import UIKit

/// Adds a profile of one of the kinds the core offers (`nori_ios_providers`), with the fields its form
/// shows (`nori_ios_form_fields`): the core checks it, saves it as the active one, and the page opens it.
/// What few profiles set folds under Advanced as one card of rows in the core's order.
final class LoginPage: UIViewController, UITextFieldDelegate, UIDocumentPickerDelegate {
    static let kinds: [String] = (takenJSON(nori_ios_providers()) as? [[String: Any]] ?? []).compactMap { $0["kind"] as? String }
    private let kinds = LoginPage.kinds
    /// The kind chosen before the form (the welcome's), whose choice the form no longer offers.
    private let given: String?
    /// What follows the profile opening; otherwise the form closes.
    var added: (() -> Void)?
    /// The profile's id from the page's opening: a client certificate is kept by it.
    private let id = taken(nori_ios_new_id()) ?? ""
    private lazy var kind = UISegmentedControl(items: kinds.map { Say.provider($0).name })
    private let hint = UILabel()
    private let address = UITextField()
    private let user = UITextField()
    private let password = UITextField()
    private let name = UITextField()
    private let second = UITextField()
    private let key = UITextField()
    private let legacyAuth = UISwitch()
    private let selfSigned = UISwitch()
    /// The extra headers, one per line.
    private var headers = ""
    /// The client certificate kept: its file's name (empty for none) and its password.
    private var cert = (name: "", password: "")
    /// The proxy as its sheet last saved it: the address as the core shows it (empty for none), and its login.
    private var proxy = (address: "", user: "", password: "")
    private lazy var headersRow = GroupRow(Say.extraHeaders, subtitle: Say.extraHeadersHint, value: Say.headers(0), chevron: true) { [weak self] in self?.askHeaders() }
    private lazy var certRow = GroupRow(Say.clientCert, value: Say.proxyNone, chevron: true) { [weak self] in self?.certTapped() }
    private lazy var proxyRow = GroupRow(Say.proxy, value: Say.proxyNone, chevron: true) { [weak self] in self?.askProxy() }
    /// The Advanced card's rows by the login JSON's keys.
    private lazy var rows: [String: UIView] = [
        "name": GroupRow(Say.name, trailing: name),
        "alt": GroupRow(Say.secondAddress, subtitle: Say.secondAddressHint, trailing: second),
        "key": GroupRow(Say.apiKey, subtitle: Say.apiKeyHint, trailing: key),
        "headers": headersRow,
        "legacyAuth": GroupRow(Say.legacyAuth, subtitle: Say.legacyAuthHint, trailing: legacyAuth),
        "selfSigned": GroupRow(Say.selfSigned, subtitle: Say.selfSignedHint, trailing: selfSigned),
        "clientCert": certRow,
        "proxy": proxyRow,
    ]
    private let advancedCard = GroupCard([], margin: 0)
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

        kind.selectedSegmentIndex = kinds.firstIndex { $0 == given } ?? 0
        kind.isHidden = kinds.count < 2 || given != nil
        kind.addTarget(self, action: #selector(kindChanged), for: .valueChanged)
        hint.textColor = Theme.secondary
        hint.font = Fonts.font(13, .regular, .footnote)
        hint.adjustsFontForContentSizeCategory = true
        hint.numberOfLines = 0
        hint.textAlignment = .center

        inRow(name, Say.optional, .default)
        name.autocapitalizationType = .words
        inRow(second, Say.secondAddressPlaceholder, .URL)
        inRow(key, "", .default)
        key.addTarget(self, action: #selector(keyEdited), for: .editingChanged)
        for toggle in [legacyAuth, selfSigned] {
            toggle.onTintColor = Theme.switchOn
        }
        more.setTitle(Say.advanced, for: .normal)
        more.tintColor = Theme.secondary
        more.contentHorizontalAlignment = .leading
        more.addTarget(self, action: #selector(moreTapped), for: .touchUpInside)
        kindChanged()
        let stack = UIStackView(arrangedSubviews: [kind, address, user, password, more, advancedCard, connect, spinner, status, hint])
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
        } else if textField === name && !advancedCard.isHidden {
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

    /// The plain boxes by the login JSON's keys.
    private var boxes: [String: UIView] {
        ["url": address, "user": user, "password": password]
    }

    /// The fields the core shows for the form as typed: the plain boxes, and the Advanced card's rows in
    /// its order.
    private func shown() {
        let fields = formJSON().withCString { takenJSON(nori_ios_form_fields($0)) } as? [[String: Any]] ?? []
        let named = { (advanced: Bool) in fields.filter { $0["advanced"] as? Bool == advanced }.compactMap { $0["what"] as? String } }
        let (plain, folded) = (named(false), named(true))
        for (what, box) in boxes { box.isHidden = !plain.contains(what) }
        advancedCard.set(folded.compactMap { rows[$0] })
        more.isHidden = folded.isEmpty
    }

    /// An API key typed or cleared takes the user and password away or brings them back.
    @objc private func keyEdited() {
        shown()
    }

    /// The headers' sheet: a line each, and one more to add; Save keeps those filled in.
    private func askHeaders() {
        let ask = UIAlertController(title: Say.extraHeaders, message: nil, preferredStyle: .alert)
        ask.view.tintColor = .black
        let lines = headers.split(separator: "\n").map(String.init) + [""]
        for line in lines {
            ask.addTextField { field in
                field.placeholder = Say.headerPlaceholder
                field.text = line
                field.autocapitalizationType = .none
                field.autocorrectionType = .no
            }
        }
        let save = UIAlertAction(title: Say.save, style: .default) { [weak self] _ in
            let kept = (ask.textFields ?? []).compactMap { $0.text }.filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
            self?.headers = kept.joined(separator: "\n")
            self?.headersRow.value.text = Say.headers(kept.count)
        }
        ask.addAction(save)
        ask.addAction(UIAlertAction(title: Say.cancel, style: .cancel))
        ask.preferredAction = save
        present(ask, animated: true)
    }

    /// The certificate's row: a .p12 picked from Files, or, with one kept, replaced or removed.
    private func certTapped() {
        guard !cert.name.isEmpty else { return pickCert() }
        let ask = UIAlertController(title: Say.clientCert, message: nil, preferredStyle: .actionSheet)
        ask.addAction(UIAlertAction(title: Say.clientCertReplace, style: .default) { [weak self] _ in self?.pickCert() })
        ask.addAction(UIAlertAction(title: Say.clientCertRemove, style: .destructive) { [weak self] _ in self?.certKept("", "") })
        ask.addAction(UIAlertAction(title: Say.cancel, style: .cancel))
        present(ask, animated: true)
    }

    private func pickCert() {
        let picker = UIDocumentPickerViewController(documentTypes: ["com.rsa.pkcs-12", "public.data"], in: .import)
        picker.delegate = self
        present(picker, animated: true)
    }

    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        if let url = urls.first { askCertPassword(url) }
    }

    /// The picked file's password, saying why the last one didn't open it: the core keeps the file once it does.
    private func askCertPassword(_ file: URL, failed: String? = nil) {
        let ask = UIAlertController(title: Say.clientCert, message: failed ?? file.lastPathComponent, preferredStyle: .alert)
        ask.view.tintColor = .black
        ask.addTextField { field in
            field.placeholder = Say.clientCertPassword
            field.isSecureTextEntry = true
        }
        let save = UIAlertAction(title: Say.save, style: .default) { [weak self] _ in
            guard let self = self else { return }
            let typed = ask.textFields?.first?.text ?? ""
            var error: Int32 = 0
            let kept = dataDirectory().path.withCString { d in
                self.id.withCString { i in file.path.withCString { f in typed.withCString { p in nori_ios_import_cert(d, i, f, p, &error) } } }
            }
            guard let name = taken(kept) else { return self.askCertPassword(file, failed: Say.certError(error)) }
            self.certKept(name, typed)
        }
        ask.addAction(save)
        ask.addAction(UIAlertAction(title: Say.cancel, style: .cancel))
        ask.preferredAction = save
        present(ask, animated: true)
    }

    /// The certificate kept, `name` empty for none, on its row.
    private func certKept(_ name: String, _ password: String) {
        cert = (name, password)
        certRow.value.text = name.isEmpty ? Say.proxyNone : Say.clientCertImported
        certRow.accessibilityLabel = "\(Say.clientCert), \(certRow.value.text ?? "")"
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

    private var chosen: String? { kinds.indices.contains(kind.selectedSegmentIndex) ? kinds[kind.selectedSegmentIndex] : nil }

    /// The Advanced card: shown on asking.
    @objc private func moreTapped() {
        let show = advancedCard.isHidden
        UIView.animate(withDuration: UIAccessibility.isReduceMotionEnabled ? 0 : 0.25) {
            self.advancedCard.isHidden = !show
        }
        more.setTitle(show ? Say.hideAdvanced : Say.advanced, for: .normal)
    }

    /// The chosen kind's plain fields, its advanced ones folded away, and its words.
    @objc private func kindChanged() {
        shown()
        advancedCard.isHidden = true
        more.setTitle(Say.advanced, for: .normal)
        let words = Say.provider(chosen ?? "")
        hint.text = words.note
        connect.setTitle(words.add, for: .normal)
        status.text = ""
    }

    /// The form as the login JSON has it (`nori_ios_login`).
    private func formJSON() -> String {
        let fields: [String: Any] = [
            "id": id, "kind": chosen ?? "",
            "url": address.text ?? "", "user": user.text ?? "", "password": password.text ?? "",
            "name": name.text ?? "", "alt": second.text ?? "", "key": key.text ?? "", "headers": headers,
            "legacyAuth": legacyAuth.isOn, "selfSigned": selfSigned.isOn,
            "clientCert": cert.name, "clientCertPassword": cert.password,
            "proxy": proxy.address, "proxyUser": proxy.user, "proxyPassword": proxy.password,
        ]
        return (try? JSONSerialization.data(withJSONObject: fields)).flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
    }

    @objc private func submit() {
        view.endEditing(true)
        let form = formJSON()
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
                self.connect.setTitle(Say.provider(self.chosen ?? "").add, for: .normal)
                self.connect.isEnabled = true
            }
        }
    }

    /// A text typed at the end of its row, in the row's ink.
    private func inRow(_ box: UITextField, _ placeholder: String, _ keyboard: UIKeyboardType) {
        box.keyboardType = keyboard
        box.autocapitalizationType = .none
        box.autocorrectionType = .no
        box.keyboardAppearance = Theme.keyboard
        box.textColor = Theme.label
        box.tintColor = Theme.accent
        box.textAlignment = .right
        box.font = Fonts.font(16)
        box.adjustsFontForContentSizeCategory = true
        box.delegate = self
        box.attributedPlaceholder = NSAttributedString(string: placeholder, attributes: [.foregroundColor: Theme.secondary])
        box.widthAnchor.constraint(equalToConstant: 130).isActive = true
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
