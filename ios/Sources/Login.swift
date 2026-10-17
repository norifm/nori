import UIKit

/// Adds a profile of one of the kinds the core offers (`nori_ios_providers`), with the fields its form
/// asks: the core checks it, saves it as the active one, and the page opens it.
final class LoginPage: UIViewController, UITextFieldDelegate {
    /// A kind offered, and its form's fields by the login JSON's keys, plain or advanced.
    private struct Kind {
        let id: String
        let plain: Set<String>
        let advanced: Set<String>
    }

    private let kinds: [Kind] = (takenJSON(nori_ios_providers()) as? [[String: Any]] ?? []).map { p in
        let fields = p["fields"] as? [[String: Any]] ?? []
        let named = { (advanced: Bool) in Set(fields.filter { $0["advanced"] as? Bool == advanced }.compactMap { $0["what"] as? String }) }
        return Kind(id: p["kind"] as? String ?? "", plain: named(false), advanced: named(true))
    }
    private lazy var kind = UISegmentedControl(items: kinds.map { Say.provider($0.id).name })
    private let hint = UILabel()
    private let address = UITextField()
    private let user = UITextField()
    private let password = UITextField()
    private let name = UITextField()
    private let second = UITextField()
    private let key = UITextField()
    private let more = UIButton(type: .system)
    private let scroll = UIScrollView()
    private let connect = UIButton(type: .system)
    private let status = UILabel()
    private let spinner = UIActivityIndicatorView()

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Theme.background
        title = Say.addServer
        navigationItem.largeTitleDisplayMode = .never
        additionalSafeAreaInsets.bottom = MiniPlayer.height

        field(address, Say.serverUrl, .URL, false)
        field(user, Say.user, .default, false)
        field(password, Say.password, .default, true)
        address.returnKeyType = .next
        user.returnKeyType = .next
        password.returnKeyType = .go

        connect.setTitle(Say.connect, for: .normal)
        connect.setTitleColor(Theme.background, for: .normal)
        connect.titleLabel?.font = UIFont.preferredFont(forTextStyle: .headline)
        connect.titleLabel?.adjustsFontForContentSizeCategory = true
        connect.backgroundColor = Theme.accent
        connect.layer.cornerRadius = 10
        connect.addTarget(self, action: #selector(submit), for: .touchUpInside)

        status.textColor = UIColor(rgb: 0xFFB4AB)
        status.font = UIFont.preferredFont(forTextStyle: .footnote)
        status.adjustsFontForContentSizeCategory = true
        status.numberOfLines = 0
        status.textAlignment = .center

        spinner.hidesWhenStopped = true
        spinner.color = Theme.label

        kind.selectedSegmentIndex = 0
        kind.isHidden = kinds.count < 2
        kind.addTarget(self, action: #selector(kindChanged), for: .valueChanged)
        hint.textColor = Theme.secondary
        hint.font = UIFont.preferredFont(forTextStyle: .footnote)
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
        let stack = UIStackView(arrangedSubviews: [kind, address, user, password, more, name, second, key, connect, spinner, status, hint])
        stack.axis = .vertical
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false
        // Scrolls, so the advanced fields and the button stay in reach above the keyboard on 568 pt.
        scroll.keyboardDismissMode = .onDrag
        scroll.alwaysBounceVertical = true
        scroll.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(scroll)
        scroll.addSubview(stack)
        NotificationCenter.default.addObserver(self, selector: #selector(keyboard(_:)), name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: scroll.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: scroll.trailingAnchor, constant: -20),
            stack.widthAnchor.constraint(equalTo: scroll.widthAnchor, constant: -40),
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

    /// The boxes by the login JSON's keys.
    private var boxes: [String: UITextField] {
        ["name": name, "url": address, "user": user, "password": password, "alt": second, "key": key]
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
                        self.navigationController?.popViewController(animated: true)
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
        box.font = UIFont.preferredFont(forTextStyle: .body)
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
