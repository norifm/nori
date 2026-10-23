import UIKit

/// Home: the core's shelves, each title an arrow to the whole row, and at the top the queue another
/// device or app left on the server, offered to carry on here (the core's `queue_offer`).
final class HomePage: PageController {
    private var card: ContinueCard?

    init() { super.init(kind: NORI_PAGE_HOME, title: Say.home) }
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        NotificationCenter.default.addObserver(self, selector: #selector(lookForOffer), name: UIApplication.didBecomeActiveNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(stands), name: .noriNow, object: nil)
    }

    /// Each time the page comes into sight; a jam guest's Home is the host's, and offers nothing.
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        lookForOffer()
    }

    @objc private func lookForOffer() {
        guard Core.shared.isOpen, !Core.shared.now.jam, view.window != nil, card == nil else { return }
        DispatchQueue.global(qos: .utility).async {
            let offer = takenJSON(nori_ios_queue_offer()) as? [String: Any]
            DispatchQueue.main.async { [weak self] in
                guard let self, let offer, self.card == nil else { return }
                self.show(ContinueCard(offer))
            }
        }
    }

    private func show(_ c: ContinueCard) {
        card = c
        c.done = { [weak self] take in self?.close(take: take) }
        let width = UIScreen.main.bounds.width
        c.frame = CGRect(x: 0, y: 0, width: width, height: ContinueCard.height + 16)
        tableView.tableHeaderView = c
    }

    /// Taken or turned down: the card goes, and its room folds away; taken, the queue starts once it has.
    private func close(take: Bool) {
        guard let c = card else { return }
        card = nil
        UIView.animate(withDuration: Motion.plain ? 0 : 0.22, animations: {
            c.alpha = 0
            c.transform = CGAffineTransform(scaleX: 0.94, y: 0.94)
        }, completion: { _ in
            self.tableView.beginUpdates()
            self.tableView.tableHeaderView = nil
            self.tableView.endUpdates()
            nori_ios_queue_offer_done(take ? 1 : 0)
        })
    }

    /// Music started, or the player came to show the offered song: the offer goes.
    @objc private func stands() {
        guard card != nil, nori_ios_queue_offer_stands() == 0 else { return }
        card = nil
        tableView.tableHeaderView = nil
    }
}

/// The offered queue as a card on the paper dyed with its song's cover: the cover running into the card,
/// "Continue", the song and where it stands, Resume and a close button; flicked away it is turned down.
final class ContinueCard: UIView {
    static let height: CGFloat = 178
    /// Taken (true) or turned down.
    var done: ((Bool) -> Void)?
    private let plate = UIView()
    private let fade = CAGradientLayer()
    private let words: [UILabel]
    private let resume = PillButton(Say.resume, Glyph.playPill)
    private let close = UIButton(type: .custom)

    init(_ offer: [String: Any]) {
        let song = Item(offer["song"] as? [String: Any] ?? [:])
        let kicker = label(Fonts.font(11.5, .semibold, .caption1), Theme.secondary)
        kicker.attributedText = NSAttributedString(string: Say.continueLabel.uppercased(), attributes: [.kern: 0.7])
        let title = label(Fonts.title(21, .title2), Theme.label, lines: 2)
        title.text = song.title
        let place = label(Fonts.font(13, .regular, .footnote), Theme.secondary)
        place.text = Say.offerPlace(song.subtitle, (offer["index"] as? Int ?? 0) + 1, offer["count"] as? Int ?? 0)
        words = [kicker, title, place]
        super.init(frame: .zero)
        plate.backgroundColor = Theme.row
        plate.layer.cornerRadius = Radius.tile
        plate.clipsToBounds = true
        let cover = CoverView()
        cover.layer.cornerRadius = 0
        cover.show(song.cover, points: ContinueCard.height)
        fade.startPoint = CGPoint(x: 0, y: 0.5)
        fade.endPoint = CGPoint(x: 1, y: 0.5)
        fade.locations = [0.55, 1]
        cover.layer.addSublayer(fade)
        let text = UIStackView(arrangedSubviews: words)
        text.axis = .vertical
        text.spacing = 4
        text.setCustomSpacing(6, after: kicker)
        resume.heightAnchor.constraint(equalToConstant: 38).isActive = true
        resume.addTarget(self, action: #selector(take), for: .touchUpInside)
        close.setImage(Glyph.xmark, for: .normal)
        close.accessibilityLabel = Say.close
        close.addTarget(self, action: #selector(dismissed), for: .touchUpInside)
        addSubview(plate)
        for v in [cover, text, resume, close] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            plate.addSubview(v)
        }
        plate.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            plate.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Space.gutter),
            plate.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -Space.gutter),
            plate.topAnchor.constraint(equalTo: topAnchor, constant: 16),
            plate.heightAnchor.constraint(equalToConstant: ContinueCard.height),
            cover.leadingAnchor.constraint(equalTo: plate.leadingAnchor),
            cover.topAnchor.constraint(equalTo: plate.topAnchor),
            cover.bottomAnchor.constraint(equalTo: plate.bottomAnchor),
            cover.widthAnchor.constraint(equalTo: cover.heightAnchor),
            text.leadingAnchor.constraint(equalTo: cover.trailingAnchor, constant: 6),
            text.trailingAnchor.constraint(equalTo: plate.trailingAnchor, constant: -16),
            text.topAnchor.constraint(equalTo: plate.topAnchor, constant: 16),
            resume.leadingAnchor.constraint(equalTo: text.leadingAnchor),
            resume.bottomAnchor.constraint(equalTo: plate.bottomAnchor, constant: -14),
            close.topAnchor.constraint(equalTo: plate.topAnchor),
            close.trailingAnchor.constraint(equalTo: plate.trailingAnchor),
            close.widthAnchor.constraint(equalToConstant: 40),
            close.heightAnchor.constraint(equalToConstant: 40),
        ])
        for way in [UISwipeGestureRecognizer.Direction.left, .right] {
            let swipe = UISwipeGestureRecognizer(target: self, action: #selector(dismissed))
            swipe.direction = way
            addGestureRecognizer(swipe)
        }
        plate.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(take)))
        paint(Theme.page)
        if Theme.coversTint && !song.cover.isEmpty {
            Core.shared.coverLook(song.cover) { [weak self] look in
                if let look { self?.paint(look) }
            }
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    override func layoutSubviews() {
        super.layoutSubviews()
        fade.frame = CGRect(x: 0, y: 0, width: ContinueCard.height, height: ContinueCard.height)
    }

    /// The card in `look`'s colors: its page, and its ink for the words and the pill.
    private func paint(_ look: PageLook) {
        plate.backgroundColor = look.page
        fade.colors = [look.page.withAlphaComponent(0).cgColor, look.page.cgColor]
        words[0].textColor = look.ink.withAlphaComponent(0.65)
        words[1].textColor = look.ink
        words[2].textColor = look.ink.withAlphaComponent(0.7)
        resume.paint(look)
        close.tintColor = look.ink.withAlphaComponent(0.6)
    }

    @objc private func take() { done?(true) }
    @objc private func dismissed() { done?(false) }
}
