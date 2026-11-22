import UIKit

/// The first run, full screen over the app: the welcome and a profile's form while the install has none, then
/// the questions (`nori_ios_setup`) once that first profile opens the app beneath them.
final class FirstRun: NavController {
    private var run: SetupRun?

    init(_ run: SetupRun? = nil) {
        self.run = run
        super.init(nibName: nil, bundle: nil)
        modalPresentationStyle = .fullScreen
        modalTransitionStyle = .crossDissolve
        viewControllers = [run.map(SetupPage.init) ?? WelcomePage()]
    }

    required init?(coder: NSCoder) { fatalError() }

    /// The questions, in place of the welcome and the form.
    func ask() {
        let questions = SetupRun()
        run = questions
        setViewControllers([SetupPage(questions)], animated: true)
    }

    /// The same place again, in the colors `Theme` now has.
    func rebuilt() -> FirstRun { FirstRun(run) }
}

/// The welcome: where the music is, as many sources as one plays from picked, then Continue on to each one's
/// form; the core's `WelcomeFlow` keeps the picks and says which form shows. The iPod has no plugins and restores
/// no backups, so its own kinds are all it offers, and one picked goes straight to its form.
final class WelcomePage: CardsPage {
    private let flow = nori_ios_welcome_new()
    private let sources = GroupCard()
    private let onward = PillButton(Say.continueLabel, nil)

    deinit { nori_ios_welcome_free(flow) }

    override func viewDidLoad() {
        super.viewDidLoad()
        let name = label(Fonts.title(38), Theme.label, lines: 0)
        name.text = Say.welcomeTitle
        let head = UIStackView(arrangedSubviews: [name])
        head.isLayoutMarginsRelativeArrangement = true
        head.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 40, leading: Space.gutter, bottom: 2, trailing: Space.gutter)
        set([head, GroupLabel(Say.welcomeWhere), sources])
        onward.addAction { [weak self] in self?.goOn() }
        view.addSubview(onward)
        NSLayoutConstraint.activate([
            onward.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: Space.groupMargin),
            onward.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -Space.groupMargin),
            onward.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -16),
        ])
        lay()
    }

    /// Back from a form: on the sources again, while nothing was added.
    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        on(NORI_WELCOME_BACK)
        lay()
    }

    @discardableResult
    private func on(_ event: Int32, _ kind: String = "") -> Int32 {
        kind.withCString { nori_ios_welcome_on(flow, event, $0) }
    }

    /// The kinds picked, and the one whose form shows now.
    private var state: (picked: [String], one: String?) {
        let d = takenJSON(nori_ios_welcome(flow)) as? [String: Any] ?? [:]
        return (d["picked"] as? [String] ?? [], d["one"] as? String)
    }

    private func lay() {
        let picked = state.picked
        onward.isEnabled = !picked.isEmpty
        onward.alpha = picked.isEmpty ? 0.38 : 1
        sources.set(LoginPage.kinds.map { k in
            let words = Say.provider(k.id)
            let on = picked.contains(k.id)
            let row = GroupRow(words.name, subtitle: words.about, icon: Glyph.draw(Iconoir.server, 20), trailing: PickMark(on)) { [weak self] in
                self?.on(NORI_WELCOME_TOGGLED, k.id)
                self?.lay()
            }
            row.isAccessibilityElement = true
            row.accessibilityTraits = on ? [.button, .selected] : .button
            return row
        })
    }

    private func goOn() {
        on(NORI_WELCOME_CONTINUE)
        guard let kind = state.one else { return }
        let form = LoginPage(kind: kind)
        form.added = { [weak self] in
            guard let self, self.on(NORI_WELCOME_ADDED, kind) == NORI_WELCOME_QUESTIONS else { return }
            (self.navigationController as? FirstRun)?.ask()
        }
        navigationController?.pushViewController(form, animated: true)
    }
}

/// Whether a source is picked: an empty ring in the quiet ink, or the accent filled with a check.
private final class PickMark: UIView {
    init(_ picked: Bool) {
        super.init(frame: .zero)
        layer.cornerRadius = 11
        if picked {
            backgroundColor = Theme.accent
            let check = UIImageView(image: Glyph.draw(Iconoir.check, 14))
            check.tintColor = Theme.background
            check.translatesAutoresizingMaskIntoConstraints = false
            addSubview(check)
            NSLayoutConstraint.activate([check.centerXAnchor.constraint(equalTo: centerXAnchor), check.centerYAnchor.constraint(equalTo: centerYAnchor)])
        } else {
            layer.borderWidth = 1.5
            layer.borderColor = Theme.label.withAlphaComponent(0.3).cgColor
        }
        translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([widthAnchor.constraint(equalToConstant: 22), heightAnchor.constraint(equalToConstant: 22)])
        isUserInteractionEnabled = false
    }

    required init?(coder: NSCoder) { fatalError() }
}

/// The first run's questions (`nori_ios_setup`), the one shown and the answers given, by index, so going back
/// shows them.
final class SetupRun {
    struct Question {
        let step: String
        let answers: [String]
    }

    let questions: [Question] = (takenJSON(nori_ios_setup()) as? [[String: Any]] ?? []).map {
        Question(step: $0["step"] as? String ?? "", answers: $0["answers"] as? [String] ?? [])
    }
    var at = 0
    var answers: [Int: Int] = [:]
}

/// One question to a page under a bar of Back, the run as dots (the one shown in the accent) and Skip: an
/// answer picked sets what it stands for at once and its preview plays it, Continue goes on; the look is taken
/// as it is picked, and Start listening ends the run.
final class SetupPage: UIViewController {
    private let run: SetupRun
    private let back = GlyphButton(Glyph.draw(Iconoir.navArrowLeft, 24), Say.setupBack)
    private let dots = UIStackView()
    private let skip = UIButton(type: .system)
    private var shown: UIView?
    /// The question shown when it has answers: it, its preview, its answers' card and Continue.
    private var answering: (question: SetupRun.Question, preview: ListeningPreview, rows: GroupCard, next: PillButton)?

    init(_ run: SetupRun) {
        self.run = run
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Theme.background
        back.tapped = { [weak self] in self?.move(-1) }
        skip.setTitle(Say.setupSkip, for: .normal)
        skip.tintColor = Theme.label
        skip.titleLabel?.font = Fonts.font(15, .medium)
        skip.addAction { [weak self] in self?.move(1) }
        dots.spacing = 6
        dots.alignment = .center
        let bar = UIView()
        for v in [bar, back, dots, skip] as [UIView] { v.translatesAutoresizingMaskIntoConstraints = false }
        view.addSubview(bar)
        [back, dots, skip].forEach(bar.addSubview)
        NSLayoutConstraint.activate([
            bar.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            bar.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            bar.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            bar.heightAnchor.constraint(equalToConstant: 52),
            back.leadingAnchor.constraint(equalTo: bar.leadingAnchor, constant: 4),
            back.centerYAnchor.constraint(equalTo: bar.centerYAnchor),
            dots.centerXAnchor.constraint(equalTo: bar.centerXAnchor),
            dots.centerYAnchor.constraint(equalTo: bar.centerYAnchor),
            skip.trailingAnchor.constraint(equalTo: bar.trailingAnchor, constant: -Space.gutter),
            skip.centerYAnchor.constraint(equalTo: bar.centerYAnchor),
        ])
        show(0)
    }

    /// The question `way` pages on, or before; past the last, the run is over.
    private func move(_ way: Int) {
        let to = run.at + way
        guard to < run.questions.count else {
            presentingViewController?.dismiss(animated: !Motion.plain)
            return
        }
        run.at = max(0, to)
        show(way)
    }

    /// The question `run.at` in place of the one shown, slid a little from the side it lies on.
    private func show(_ way: Int) {
        let last = run.questions.count - 1
        back.isHidden = run.at == 0
        skip.isHidden = run.at >= last
        dots.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for i in run.questions.indices {
            let dot = UIView()
            dot.backgroundColor = i == run.at ? Theme.accent : Theme.page.veil(0.22)
            dot.layer.cornerRadius = 3
            dot.translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([
                dot.widthAnchor.constraint(equalToConstant: i == run.at ? 18 : 6),
                dot.heightAnchor.constraint(equalToConstant: 6),
            ])
            dots.addArrangedSubview(dot)
        }
        let q = run.questions[run.at]
        answering = nil
        let page = q.answers.isEmpty ? look() : answers(q)
        page.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(page)
        NSLayoutConstraint.activate([
            page.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 52),
            page.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            page.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            page.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        let old = shown
        shown = page
        guard let old, way != 0 else {
            old?.removeFromSuperview()
            return
        }
        let shift = view.bounds.width / 5 * CGFloat(way)
        page.alpha = 0
        if !Motion.plain { page.transform = CGAffineTransform(translationX: shift, y: 0) }
        UIView.animate(withDuration: 0.25, delay: 0, options: [.curveEaseOut], animations: {
            page.alpha = 1
            page.transform = .identity
            old.alpha = 0
            if !Motion.plain { old.transform = CGAffineTransform(translationX: -shift, y: 0) }
        }, completion: { _ in old.removeFromSuperview() })
    }

    /// A question's title in the page's type.
    private func heading(_ step: String) -> UIView {
        let title = label(Fonts.title(31), Theme.label, lines: 0)
        title.text = Say.setupStep(step)
        let holder = UIStackView(arrangedSubviews: [title])
        holder.isLayoutMarginsRelativeArrangement = true
        holder.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 20, leading: Space.gutter, bottom: 22, trailing: Space.gutter)
        return holder
    }

    /// A question: a preview of what the answer picked does, the answers as rows, the one picked checked, and
    /// Continue once one is.
    private func answers(_ q: SetupRun.Question) -> UIView {
        let at = run.at
        let preview = ListeningPreview(run.answers[at].map { q.answers[$0] })
        let plate = UIView()
        plate.backgroundColor = Theme.row
        plate.layer.cornerRadius = Radius.tile
        plate.layer.borderWidth = hairlineWidth
        plate.layer.borderColor = Theme.hairline.cgColor
        preview.translatesAutoresizingMaskIntoConstraints = false
        plate.addSubview(preview)
        let holder = UIView()
        plate.translatesAutoresizingMaskIntoConstraints = false
        holder.addSubview(plate)
        NSLayoutConstraint.activate([
            plate.leadingAnchor.constraint(equalTo: holder.leadingAnchor, constant: Space.groupMargin),
            plate.trailingAnchor.constraint(equalTo: holder.trailingAnchor, constant: -Space.groupMargin),
            plate.topAnchor.constraint(equalTo: holder.topAnchor),
            plate.bottomAnchor.constraint(equalTo: holder.bottomAnchor, constant: -14),
            preview.leadingAnchor.constraint(equalTo: plate.leadingAnchor, constant: 18),
            preview.trailingAnchor.constraint(equalTo: plate.trailingAnchor, constant: -18),
            preview.topAnchor.constraint(equalTo: plate.topAnchor, constant: 22),
            preview.bottomAnchor.constraint(equalTo: plate.bottomAnchor, constant: -14),
        ])
        let rows = GroupCard()
        let next = PillButton(Say.continueLabel, nil)
        next.addAction { [weak self] in self?.move(1) }
        answering = (q, preview, rows, next)
        layAnswers()
        let page = scrolling([heading(q.step), holder, rows], spacing: 0, foot: 78)
        pin(next, in: page)
        return page
    }

    /// The answers' rows, the one picked checked, and Continue on once one is.
    private func layAnswers() {
        guard let shown = answering else { return }
        let chosen = run.answers[run.at]
        shown.next.isEnabled = chosen != nil
        shown.next.alpha = chosen == nil ? 0.38 : 1
        shown.rows.set(shown.question.answers.enumerated().map { i, a in
            let words = Say.setupAnswer(a)
            let check = UIImageView(image: Glyph.check)
            check.tintColor = Theme.accent
            check.isHidden = chosen != i
            let row = GroupRow(words.name, subtitle: words.line, trailing: check, minHeight: 64) { [weak self] in self?.pick(i) }
            row.isAccessibilityElement = true
            row.accessibilityTraits = chosen == i ? [.button, .selected] : .button
            return row
        })
    }

    /// Answer `i` of the question shown picked: its settings set, its row checked, its preview played.
    private func pick(_ i: Int) {
        guard let shown = answering, run.answers[run.at] != i else { return }
        nori_ios_setup_answer(UInt32(run.at), UInt32(i))
        run.answers[run.at] = i
        layAnswers()
        shown.preview.play(shown.question.answers[i])
    }

    /// The look, taken as it is picked, with Settings' own Appearance controls; Start listening at the foot.
    private func look() -> UIView {
        let picked = { [weak self] (mode: String, season: String) in
            guard AppearancePage.wear(mode: mode, season: season) else {
                self?.show(0)
                return
            }
            (UIApplication.shared.delegate as? AppDelegate)?.restyle()
        }
        let page = scrolling([heading(run.questions[run.at].step)] + AppearancePage.controls(picked), spacing: 0, foot: 78)
        let start = PillButton(Say.setupStart, nil)
        start.addAction { [weak self] in self?.move(1) }
        pin(start, in: page)
        return page
    }

    /// `pill` at the foot of `page`, over what scrolls.
    private func pin(_ pill: PillButton, in page: UIView) {
        page.addSubview(pill)
        NSLayoutConstraint.activate([
            pill.leadingAnchor.constraint(equalTo: page.leadingAnchor, constant: Space.groupMargin),
            pill.trailingAnchor.constraint(equalTo: page.trailingAnchor, constant: -Space.groupMargin),
            pill.bottomAnchor.constraint(equalTo: page.safeAreaLayoutGuide.bottomAnchor, constant: -16),
        ])
    }

    /// `views` in a scrolling column on the page's paper, `foot` points kept clear under them.
    private func scrolling(_ views: [UIView], spacing: CGFloat, foot: CGFloat = 16) -> UIView {
        let holder = UIView()
        holder.backgroundColor = Theme.background
        let scroll = UIScrollView()
        scroll.alwaysBounceVertical = true
        scroll.delaysContentTouches = false
        let column = UIStackView(arrangedSubviews: views)
        column.axis = .vertical
        column.spacing = spacing
        for v in [scroll, column] as [UIView] { v.translatesAutoresizingMaskIntoConstraints = false }
        holder.addSubview(scroll)
        scroll.addSubview(column)
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: holder.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: holder.bottomAnchor),
            scroll.leadingAnchor.constraint(equalTo: holder.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: holder.trailingAnchor),
            column.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor),
            column.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -foot),
            column.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor),
            column.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor),
            column.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor),
        ])
        return holder
    }
}

/// The first run's covers (`nori_ios_setup_covers`) in a row, played as a listening answer plays songs: apart
/// with a cut between them without mixing, overlapping and blended with AutoMix, drawn close and blended longer,
/// over the beat, with the DJ. Another answer glides them to their places, and a pick runs a playhead across
/// them. Covers not decoded yet are plates.
private final class ListeningPreview: UIView {
    /// Where each cover starts after the one before, as a share of its side, and how much of it fades in over that
    /// one, for `answer` (no mixing's with none picked yet).
    private static func spacing(_ answer: String?) -> (step: CGFloat, fade: CGFloat) {
        switch answer {
        case "AutoMix": return (0.72, 0.28)
        case "AutoMixDj": return (0.5, 0.5)
        default: return (1.12, 0)
        }
    }

    private let covers = (0..<Int(nori_ios_setup_covers())).map { BlendedCover(blends: $0 > 0) }
    private let ticks = (0...16).map { _ in UIView() }
    /// The covers' side as a share of the width: the widest arrangement fits.
    private let share: CGFloat
    private var step: CGFloat
    private var asked = false
    private var head: UIView?

    init(_ answer: String?) {
        let (step, fade) = ListeningPreview.spacing(answer)
        self.step = step
        share = 1 / (1 + ListeningPreview.spacing(nil).step * CGFloat(nori_ios_setup_covers() - 1))
        super.init(frame: .zero)
        for cover in covers {
            cover.fade = fade
            addSubview(cover)
        }
        for tick in ticks {
            tick.backgroundColor = Theme.label.withAlphaComponent(0.32)
            tick.alpha = answer == "AutoMixDj" ? 1 : 0
            addSubview(tick)
        }
        heightAnchor.constraint(equalTo: widthAnchor, multiplier: share, constant: 24).isActive = true
    }

    required init?(coder: NSCoder) { fatalError() }

    private var side: CGFloat { bounds.width * share }
    private var span: CGFloat { side * (1 + step * CGFloat(covers.count - 1)) }
    private var left: CGFloat { (bounds.width - span) / 2 }

    override func layoutSubviews() {
        super.layoutSubviews()
        for (i, cover) in covers.enumerated() {
            cover.frame = CGRect(x: left + CGFloat(i) * step * side, y: 0, width: side, height: side)
        }
        for (i, tick) in ticks.enumerated() {
            tick.frame = CGRect(x: left + span * CGFloat(i) / CGFloat(ticks.count - 1) - 0.75, y: side + 12, width: 1.5, height: 8)
        }
        guard !asked, side > 0 else { return }
        asked = true
        let px = Int(side * UIScreen.main.scale)
        for (i, cover) in covers.enumerated() {
            Core.shared.setupCover(i, px: px) { [weak cover] image in cover?.image = image }
        }
    }

    /// `answer` picked: the covers glide to its places, and the playhead runs across them once.
    func play(_ answer: String) {
        let (step, fade) = ListeningPreview.spacing(answer)
        self.step = step
        let move = Motion.plain ? 0 : 0.3
        UIView.animate(withDuration: move, delay: 0, options: [.curveEaseInOut], animations: {
            self.setNeedsLayout()
            self.layoutIfNeeded()
            self.ticks.forEach { $0.alpha = answer == "AutoMixDj" ? 1 : 0 }
        })
        CATransaction.begin()
        CATransaction.setAnimationDuration(move)
        CATransaction.setAnimationTimingFunction(CAMediaTimingFunction(name: .easeInEaseOut))
        covers.forEach { $0.fade = fade }
        CATransaction.commit()
        head?.removeFromSuperview()
        guard !Motion.plain else { return }
        let line = UIView(frame: CGRect(x: left - 1, y: -6, width: 2, height: side + 12))
        line.backgroundColor = Theme.accent
        addSubview(line)
        head = line
        UIView.animate(withDuration: 0.9, delay: 0, options: [.curveLinear], animations: {
            line.frame.origin.x = self.left + self.span - 1
        }, completion: { _ in line.removeFromSuperview() })
    }
}

/// A cover on its plate; one that `blends` fades in from its left edge over `fade` of its width (a gradient
/// mask), so it cross-fades over the cover before it.
private final class BlendedCover: UIView {
    private let picture = UIImageView()
    private let ramp = CAGradientLayer()
    var image: UIImage? {
        get { picture.image }
        set { picture.image = newValue }
    }
    var fade: CGFloat = 0 {
        didSet { ramp.locations = [0, NSNumber(value: Double(fade))] }
    }

    init(blends: Bool) {
        super.init(frame: .zero)
        picture.backgroundColor = Theme.page.veil(0.1)
        picture.contentMode = .scaleAspectFill
        picture.layer.cornerRadius = Radius.cover
        picture.clipsToBounds = true
        addSubview(picture)
        ramp.colors = [UIColor.clear.cgColor, UIColor.black.cgColor]
        ramp.startPoint = CGPoint(x: 0, y: 0.5)
        ramp.endPoint = CGPoint(x: 1, y: 0.5)
        ramp.locations = [0, 0]
        if blends { layer.mask = ramp }
    }

    required init?(coder: NSCoder) { fatalError() }

    override func layoutSubviews() {
        super.layoutSubviews()
        picture.frame = bounds
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        ramp.frame = bounds
        CATransaction.commit()
    }
}
