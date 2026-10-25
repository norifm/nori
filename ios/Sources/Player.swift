import AVKit
import CoreText
import MediaPlayer
import UIKit

/// The player, as Android's: the artwork running edge to edge from the top and melting into the page, the
/// title row with the seal and more, the seek bar, then lyrics, previous, play, next and queue, then where
/// the sound goes, the device's glyph and name as one button. The page is the season's paper dyed with the
/// cover (not in Midnight). While another device plays, the card is that device's music and controls it;
/// a jam guest's is the jam's, its host's to control, the jam's glyph where the output is. Drag down to
/// close.
final class PlayerCard: UIViewController {
    /// The cover's size on the card, which the lock screen's artwork shares.
    static var coverPoints: CGFloat { UIScreen.main.bounds.width }
    static var coverPx: Int { Int(coverPoints * UIScreen.main.scale) }

    /// How card comes and goes; kept here because `transitioningDelegate` is weak.
    let transition = CardTransition()
    private var closer: DragToClose?
    private let cover = CoverView()
    private let melt = GradientView(colours: [])
    private let songTitle = ReadingLine()
    private let artist = label(Fonts.font(16, .medium, .headline), Theme.Card.label)
    private let album = label(Fonts.font(14, .regular, .subheadline), Theme.Card.label)
    private let seal = GlyphButton(Glyph.seal, Say.favorite)
    private let more = GlyphButton(Glyph.more, Say.more)
    private let seek = UISlider()
    private let elapsed = label(Fonts.digits(12), Theme.Card.secondary)
    private let remaining = label(Fonts.digits(12), Theme.Card.secondary)
    private let mixing = label(Fonts.font(11, .regular, .caption2), Theme.Card.secondary)
    private let lyrics = GlyphButton(Glyph.lyrics, Say.lyrics, quiet: true)
    private let previous = GlyphButton(Glyph.bigPrevious, Say.previous, side: 56)
    private let play = GlyphButton(Glyph.bigPlay, Say.play, side: 68)
    private let nextButton = GlyphButton(Glyph.bigNext, Say.next, side: 56)
    private let queue = GlyphButton(Glyph.queue, Say.queue, quiet: true)
    private let transport = UIStackView()
    /// Where the sound goes: the device's glyph and name, one button.
    private let output = UIButton(type: .custom)
    private let outputs = OutputPicker()
    private let grabber = UIButton(type: .custom)
    private var ticker: Timer?
    private var scrubbing = false
    private var paintedCover = ""

    init() {
        super.init(nibName: nil, bundle: nil)
        // Over the app, so a drag (up from the mini player, or down to close) shows what is under it.
        modalPresentationStyle = .overFullScreen
        modalPresentationCapturesStatusBarAppearance = true
        transitioningDelegate = transition
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        grabber.setImage(Glyph.close, for: .normal)
        grabber.tintColor = .white
        grabber.addTarget(self, action: #selector(close), for: .touchUpInside)
        grabber.accessibilityLabel = Say.close
        cover.layer.cornerRadius = 0
        let shade = GradientView(colours: [UIColor.black.withAlphaComponent(0.30), UIColor.black.withAlphaComponent(0)], at: [0, 0.16])

        songTitle.font = Fonts.title(26, .title1)
        // The artist and the album each on a line of their own, held back from the title rather than
        // colored, and each a way there.
        for (l, action) in [(artist, #selector(artistTapped)), (album, #selector(albumTapped))] {
            l.isUserInteractionEnabled = true
            l.addGestureRecognizer(UITapGestureRecognizer(target: self, action: action))
        }
        seal.addTarget(self, action: #selector(sealTapped), for: .touchUpInside)
        more.addTarget(self, action: #selector(moreTapped), for: .touchUpInside)
        let names = UIStackView(arrangedSubviews: [songTitle, artist, album])
        names.axis = .vertical
        names.spacing = 1
        let songRow = UIStackView(arrangedSubviews: [names, seal, more])
        songRow.spacing = 0
        songRow.alignment = .center
        songRow.setCustomSpacing(4, after: names)

        seek.addTarget(self, action: #selector(scrubBegan), for: .touchDown)
        seek.addTarget(self, action: #selector(scrubMoved), for: .valueChanged)
        seek.addTarget(self, action: #selector(scrubEnded), for: [.touchUpInside, .touchUpOutside, .touchCancel])
        remaining.textAlignment = .right
        for l in [elapsed, remaining] {
            l.setContentHuggingPriority(.required, for: .horizontal)
            l.setContentCompressionResistancePriority(.required, for: .horizontal)
        }
        mixing.text = Say.mixing
        mixing.textAlignment = .center
        mixing.isHidden = true
        let times = UIStackView(arrangedSubviews: [elapsed, mixing, remaining])
        times.distribution = .equalSpacing

        lyrics.addTarget(self, action: #selector(lyricsTapped), for: .touchUpInside)
        previous.addTarget(self, action: #selector(previousTapped), for: .touchUpInside)
        play.addTarget(self, action: #selector(playTapped), for: .touchUpInside)
        nextButton.addTarget(self, action: #selector(nextTapped), for: .touchUpInside)
        queue.addTarget(self, action: #selector(queueTapped), for: .touchUpInside)
        // One row of bare glyphs: Lyrics, rewind, play, fast-forward, the queue.
        [lyrics, previous, play, nextButton, queue].forEach(transport.addArrangedSubview)
        transport.distribution = .equalSpacing
        transport.alignment = .center

        output.titleLabel?.font = Fonts.font(15, .regular, .subheadline)
        output.titleLabel?.lineBreakMode = .byTruncatingTail
        output.imageEdgeInsets = UIEdgeInsets(top: 0, left: -5, bottom: 0, right: 5)
        output.titleEdgeInsets = UIEdgeInsets(top: 0, left: 5, bottom: 0, right: -5)
        output.contentEdgeInsets = UIEdgeInsets(top: 0, left: 19, bottom: 0, right: 19)
        output.heightAnchor.constraint(equalToConstant: 48).isActive = true
        output.addTarget(self, action: #selector(devicesTapped), for: .touchUpInside)
        outputs.attach(to: view)
        let outputRow = UIStackView(arrangedSubviews: [output])
        outputRow.alignment = .center
        outputRow.axis = .vertical

        let column = UIStackView(arrangedSubviews: [songRow, seek, times, transport, outputRow])
        column.axis = .vertical
        column.spacing = 4
        column.setCustomSpacing(10, after: songRow)
        column.setCustomSpacing(8, after: times)
        column.setCustomSpacing(6, after: transport)

        for v in [cover, column, grabber] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            view.addSubview(v)
        }
        for v in [melt, shade] {
            v.translatesAutoresizingMaskIntoConstraints = false
            cover.addSubview(v)
            NSLayoutConstraint.activate([
                v.leadingAnchor.constraint(equalTo: cover.leadingAnchor),
                v.trailingAnchor.constraint(equalTo: cover.trailingAnchor),
                v.topAnchor.constraint(equalTo: cover.topAnchor),
                v.bottomAnchor.constraint(equalTo: cover.bottomAnchor),
            ])
        }
        for l in [songTitle, artist, album] as [UIView] {
            l.setContentCompressionResistancePriority(.required, for: .vertical)
            l.setContentHuggingPriority(.required, for: .vertical)
        }
        // As tall as a page's hero (`HeroHeader.deeper` past square), held to the end of the title row.
        let deep = cover.heightAnchor.constraint(equalTo: cover.widthAnchor, constant: HeroHeader.deeper)
        deep.priority = .defaultHigh
        NSLayoutConstraint.activate([
            // The artwork from the top, as wide as the screen, reaching down behind the title.
            cover.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            cover.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            cover.topAnchor.constraint(equalTo: view.topAnchor),
            cover.bottomAnchor.constraint(lessThanOrEqualTo: songRow.bottomAnchor),
            deep,
            grabber.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            grabber.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            grabber.widthAnchor.constraint(equalToConstant: 60),
            grabber.heightAnchor.constraint(equalToConstant: 32),
            column.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: Space.gutter),
            column.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -Space.gutter + 8),
            column.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -6),
            seek.trailingAnchor.constraint(equalTo: column.trailingAnchor, constant: -8),
            times.trailingAnchor.constraint(equalTo: column.trailingAnchor, constant: -8),
        ])
        closer = DragToClose(self, transition)
        NotificationCenter.default.addObserver(self, selector: #selector(changed), name: .noriNow, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(paintSeal), name: .noriFavorites, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(changed), name: .noriJam, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(paintOutput), name: AVAudioSession.routeChangeNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(changed), name: UIApplication.didBecomeActiveNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(stopTicking), name: UIApplication.didEnterBackgroundNotification, object: nil)
        wear(Theme.player)
        changed()
    }

    override var preferredStatusBarStyle: UIStatusBarStyle { .lightContent }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        changed()
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        stopTicking()
    }

    @objc private func stopTicking() {
        ticker?.invalidate()
        ticker = nil
    }

    /// The card in `look`'s colors: the page, its ink and its quieter ink, the melt into it.
    private func wear(_ look: PageLook) {
        Theme.player = look
        view.backgroundColor = look.page
        (melt.layer as? CAGradientLayer)?.colors = look.melt.map { $0.cgColor }
        (melt.layer as? CAGradientLayer)?.locations = look.stops.map { NSNumber(value: Double($0)) }
        songTitle.textColor = look.ink
        songTitle.fadeColor = look.page
        artist.textColor = look.ink.withAlphaComponent(0.6)
        album.textColor = look.ink.withAlphaComponent(0.45)
        for l in [elapsed, remaining, mixing] { l.textColor = look.quiet }
        seek.minimumTrackTintColor = look.ink
        seek.maximumTrackTintColor = look.veil(0.22)
        seek.setThumbImage(PlayerCard.dot(8, look.accent), for: .normal)
        seek.setThumbImage(PlayerCard.dot(16, look.accent), for: .highlighted)
        for b in [seal, more, lyrics, previous, play, nextButton, queue] { b.look = look }
        paintSeal()
        paintOutput()
    }

    /// A full-screen sheet is up: the card is out of sight, though UIKit keeps it "appeared".
    private var covered: Bool {
        guard let sheet = presentedViewController else { return false }
        return sheet.modalPresentationStyle == .overFullScreen && !sheet.isBeingDismissed
    }

    /// The sheets call this as they close, so the clock starts again.
    @objc func changed() {
        let now = Core.shared.now
        // A jam guest's controls are those its role offers (the core's jam controls).
        let jam = now.jam ? Core.shared.jam : nil
        previous.isHidden = now.jam && (jam?.skip ?? 0) == 0
        nextButton.isHidden = previous.isHidden
        play.isHidden = now.jam && (jam?.play ?? 0) == 0
        seek.isUserInteractionEnabled = !now.jam || (jam?.seek ?? 0) != 0
        mixing.isHidden = !now.playing || !now.mixing
        paintOutput()
        guard let song = now.song else {
            songTitle.text = Say.nothingPlaying
            artist.text = ""
            album.text = ""
            seal.isHidden = true
            return
        }
        songTitle.text = song.title
        artist.text = song.subtitle
        album.text = song.album
        album.isHidden = song.album.isEmpty
        paintSeal()
        if paintedCover != song.cover {
            paintedCover = song.cover
            cover.show(song.cover, points: PlayerCard.coverPoints)
            if Theme.coversTint && !song.cover.isEmpty {
                let id = song.cover
                Core.shared.coverLook(id) { [weak self] look in
                    guard let self, let look, self.paintedCover == id else { return }
                    let swap = { self.wear(look) }
                    if self.view.window != nil && !Motion.plain {
                        UIView.transition(with: self.view, duration: 0.3, options: [.transitionCrossDissolve, .allowUserInteraction], animations: swap)
                    } else {
                        swap()
                    }
                }
            }
        }
        // A jam guest's says what its controls say: paused here while the jam plays on.
        let playing = jam?.playing ?? now.playing
        play.setImage(playing ? Glyph.bigPause : Glyph.bigPlay, for: .normal)
        play.accessibilityLabel = playing ? Say.pause : Say.play
        paintTime()
        if now.playing && view.window != nil && !covered && UIApplication.shared.applicationState == .active {
            if ticker == nil {
                ticker = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in self?.paintTime() }
                ticker?.tolerance = 0.1
            }
        } else {
            stopTicking()
        }
    }

    /// Where the sound goes: what plays (another device's glyph, lit), or the jam in its place.
    @objc private func paintOutput() {
        let now = Core.shared.now
        let look = Theme.player
        let glyph: UIImage, words: String, lit: Bool
        if now.jam {
            glyph = Glyph.jam
            words = Core.shared.jam?.strip ?? Say.jamGuest
            lit = (Core.shared.jam?.listeners.count ?? 0) > 0
        } else if let device = now.device {
            glyph = Glyph.device(now.deviceKind, 22)
            words = device
            lit = true
        } else {
            let o = OutputLook.now()
            glyph = Glyph.output(o.glyph, 22)
            words = Say.outputName(port: o.port, name: o.name)
            lit = false
            output.accessibilityLabel = Say.outputDescription(port: o.port, name: o.name)
        }
        let colour = lit ? (now.jam ? look.accent : look.ink) : look.quiet
        output.setImage(glyph, for: .normal)
        output.setTitle(words, for: .normal)
        output.tintColor = colour
        output.setTitleColor(colour, for: .normal)
    }

    private func paintTime() {
        guard !scrubbing else { return }
        let now = Core.shared.now
        let total = (now.song?.seconds ?? 0) * 1000
        let at = total > 0 ? min(now.position, total) : now.position
        seek.isEnabled = total > 0
        seek.maximumValue = Float(max(total, 1))
        seek.value = total > 0 ? Float(at) : 0
        elapsed.text = Fmt.clock(ms: at)
        remaining.text = "-" + Fmt.clock(ms: max(0, total - at))
    }

    private static func dot(_ d: CGFloat, _ colour: UIColor) -> UIImage {
        UIGraphicsBeginImageContextWithOptions(CGSize(width: d, height: d), false, 0)
        colour.setFill()
        UIBezierPath(ovalIn: CGRect(x: 0, y: 0, width: d, height: d)).fill()
        let image = UIGraphicsGetImageFromCurrentImageContext() ?? UIImage()
        UIGraphicsEndImageContext()
        return image
    }

    @objc private func close() { dismiss(animated: true) }

    @objc private func scrubBegan() { scrubbing = true }

    @objc private func scrubMoved() {
        let total = Int(seek.maximumValue)
        let at = Int(seek.value)
        elapsed.text = Fmt.clock(ms: at)
        remaining.text = "-" + Fmt.clock(ms: max(0, total - at))
    }

    @objc private func scrubEnded() {
        scrubbing = false
        nori_ios_seek(Int64(seek.value))
    }

    @objc private func playTapped() { nori_ios_toggle() }
    @objc private func nextTapped() { nori_ios_next() }
    @objc private func previousTapped() { nori_ios_previous() }

    /// The seal where songs carry favorites: in the accent when the song is one.
    @objc private func paintSeal() {
        guard let song = Core.shared.now.song else { return }
        let on = Core.shared.isFavorite(song)
        seal.setImage(on ? Glyph.sealed : Glyph.seal, for: .normal)
        seal.lit = on
        seal.isHidden = song.external || !Core.shared.rules.stars
    }

    @objc private func sealTapped() {
        guard let song = Core.shared.now.song else { return }
        Core.shared.favorite(song, !Core.shared.isFavorite(song))
        paintSeal()
    }

    @objc private func artistTapped() {
        guard let song = Core.shared.now.song, !song.artistId.isEmpty else { return }
        go(PageController(kind: NORI_PAGE_ARTIST, arg: song.artistId, title: song.subtitle))
    }

    @objc private func albumTapped() {
        guard let song = Core.shared.now.song, !song.albumId.isEmpty else { return }
        go(PageController(kind: NORI_PAGE_ALBUM, arg: song.albumId, title: song.album))
    }

    /// The song's menu, the core's; its format is the line under the title, as on Android.
    @objc private func moreTapped() {
        let now = Core.shared.now
        guard let song = now.song else { return }
        let token = Core.shared.token()
        guard nori_ios_keep_now(token) == 1 else { return }
        let format = Fmt.quality(suffix: now.suffix, kbps: now.kbps, hz: now.hz, bits: now.bits)
        SongMenu.show(song, token: token, index: 0, from: self, player: true, message: format.isEmpty ? nil : format) { [weak self] page in
            self?.go(page)
        }
    }

    /// Closes the card and opens `page` in the visible tab.
    private func go(_ page: UIViewController) {
        let shell = presentingViewController as? ShellController
        dismiss(animated: true) {
            shell?.current.pushViewController(page, animated: true)
        }
    }

    @objc private func lyricsTapped() {
        guard let song = Core.shared.now.song else { return }
        cover(with: LyricsSheet(song: song.id))
    }

    @objc private func queueTapped() {
        cover(with: QueueSheet())
    }

    private func cover(with sheet: UIViewController) {
        stopTicking()
        present(sheet, animated: true)
    }

    /// The output button: nori's devices with this iPod's outputs among them, or just the outputs with
    /// remote control off; the jam's queue where a jam is.
    @objc private func devicesTapped() {
        if Core.shared.now.jam {
            queueTapped()
        } else if Core.shared.now.remote {
            cover(with: DevicesSheet())
        } else {
            outputs.show()
        }
    }
}

/// The card sliding up from below and back down, by itself or under a finger (`interaction`). UIKit
/// sets the frames as it presents, so the move animates frames too.
final class CardTransition: NSObject, UIViewControllerTransitioningDelegate, UIViewControllerAnimatedTransitioning {
    /// Set while a finger drives the move; cleared when it ends.
    var interaction: UIPercentDrivenInteractiveTransition?

    func animationController(forPresented presented: UIViewController, presenting: UIViewController, source: UIViewController) -> UIViewControllerAnimatedTransitioning? { self }

    func animationController(forDismissed dismissed: UIViewController) -> UIViewControllerAnimatedTransitioning? { self }

    func interactionControllerForPresentation(using animator: UIViewControllerAnimatedTransitioning) -> UIViewControllerInteractiveTransitioning? { interaction }

    func interactionControllerForDismissal(using animator: UIViewControllerAnimatedTransitioning) -> UIViewControllerInteractiveTransitioning? { interaction }

    func transitionDuration(using context: UIViewControllerContextTransitioning?) -> TimeInterval { 0.35 }

    func animateTransition(using context: UIViewControllerContextTransitioning) {
        let opening = context.viewController(forKey: .to)?.isBeingPresented == true
        guard let card = context.view(forKey: opening ? .to : .from) else {
            return context.completeTransition(false)
        }
        let shown = context.containerView.bounds
        let below = shown.offsetBy(dx: 0, dy: shown.height)
        if opening {
            context.containerView.addSubview(card)
            card.frame = below
        }
        // Linear under a finger, so the card stays under it; eased when it runs by itself.
        let curve: UIView.AnimationOptions = context.isInteractive ? .curveLinear : .curveEaseOut
        UIView.animate(withDuration: transitionDuration(using: context), delay: 0, options: curve, animations: {
            card.frame = opening ? shown : below
        }, completion: { _ in
            let done = !context.transitionWasCancelled
            if opening && !done { card.removeFromSuperview() }
            self.interaction = nil
            context.completeTransition(done)
        })
    }
}

/// The lyrics over the card: the chevron pinned above the lines, and that strip dragged down closes it
/// under the finger wherever the lines are scrolled; pulling past the first line closes it too.
final class LyricsSheet: UIViewController {
    let transition = CardTransition()
    private let lines: LyricsCard
    private var closer: DragToClose?

    init(song: String) {
        lines = LyricsCard(song: song)
        super.init(nibName: nil, bundle: nil)
        modalPresentationStyle = .overFullScreen
        modalPresentationCapturesStatusBarAppearance = true
        transitioningDelegate = transition
    }

    required init?(coder: NSCoder) { fatalError() }

    override var preferredStatusBarStyle: UIStatusBarStyle { Theme.statusBar }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        (presentingViewController as? PlayerCard)?.changed()
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Theme.Card.background
        let grabber = UIButton(type: .system)
        grabber.setImage(Glyph.close, for: .normal)
        grabber.tintColor = Theme.Card.secondary
        grabber.accessibilityLabel = Say.close
        grabber.addTarget(self, action: #selector(close), for: .touchUpInside)
        addChild(lines)
        for v in [grabber, lines.view!] {
            v.translatesAutoresizingMaskIntoConstraints = false
            view.addSubview(v)
        }
        NSLayoutConstraint.activate([
            grabber.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            grabber.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            grabber.widthAnchor.constraint(equalToConstant: 120),
            grabber.heightAnchor.constraint(equalToConstant: 36),
            lines.view.topAnchor.constraint(equalTo: grabber.bottomAnchor),
            lines.view.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            lines.view.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            lines.view.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        lines.didMove(toParent: self)
        closer = DragToClose(self, transition) { [weak self] pan in
            guard let self else { return false }
            return pan.location(in: self.view).y < self.lines.view.frame.minY
        }
    }

    @objc private func close() { dismiss(animated: true) }
}

/// A drag down that closes a sheet under the finger through its `CardTransition`; let go early or
/// slowly and it comes back. `starts` says where on the sheet such a drag may begin.
final class DragToClose: NSObject, UIGestureRecognizerDelegate {
    /// How far past a list's top a pull closes its sheet; and how far a drag must go to close.
    static let pull: CGFloat = 70
    private static let far: CGFloat = 120
    private weak var sheet: UIViewController?
    private let transition: CardTransition
    private let starts: (UIPanGestureRecognizer) -> Bool

    init(_ sheet: UIViewController, _ transition: CardTransition,
         starts: @escaping (UIPanGestureRecognizer) -> Bool = { _ in true }) {
        self.sheet = sheet
        self.transition = transition
        self.starts = starts
        super.init()
        let pan = UIPanGestureRecognizer(target: self, action: #selector(dragged(_:)))
        pan.delegate = self
        sheet.view.addGestureRecognizer(pan)
    }

    func gestureRecognizerShouldBegin(_ g: UIGestureRecognizer) -> Bool {
        guard let pan = g as? UIPanGestureRecognizer, let view = sheet?.view else { return true }
        let v = pan.velocity(in: view)
        return v.y > abs(v.x) && starts(pan)
    }

    @objc private func dragged(_ g: UIPanGestureRecognizer) {
        guard let sheet else { return }
        // In the window's space: the sheet itself moves with the finger.
        let y = max(0, g.translation(in: sheet.view.window).y)
        switch g.state {
        case .began:
            guard sheet.presentedViewController == nil else { return }
            transition.interaction = UIPercentDrivenInteractiveTransition()
            sheet.dismiss(animated: true)
        case .changed:
            transition.interaction?.update(min(1, y / max(1, sheet.view.bounds.height)))
        case .ended, .cancelled:
            guard let drive = transition.interaction else { return }
            let closes = g.state == .ended && (y > DragToClose.far || g.velocity(in: sheet.view.window).y > 900)
            if closes { drive.finish() } else { drive.cancel() }
        default:
            break
        }
    }
}

/// One lyric line drawn with CoreText: the first `sung` UTF-16 units at `strength`, the rest at `rest`,
/// the edge moving smoothly inside a character.
final class LyricLineView: UIView {
    static let font = Fonts.font(24, .semibold, .title2)
    private var framesetter: CTFramesetter?
    private var drawn: CTFrame?
    var text = "" {
        didSet {
            guard text != oldValue else { return }
            framesetter = CTFramesetterCreateWithAttributedString(LyricLineView.attributed(text))
            drawn = nil
            accessibilityLabel = text
            setNeedsDisplay()
        }
    }
    var sung: CGFloat = .greatestFiniteMagnitude { didSet { if sung != oldValue { setNeedsDisplay() } } }
    var strength: CGFloat = 1 { didSet { if strength != oldValue { setNeedsDisplay() } } }
    var rest: CGFloat = 1 { didSet { if rest != oldValue { setNeedsDisplay() } } }

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = false
        backgroundColor = .clear
        // Drawn, not a label: VoiceOver reads the line from here.
        isAccessibilityElement = true
        accessibilityTraits = .staticText
    }

    required init?(coder: NSCoder) { fatalError() }

    /// The colour comes from the context, so one frame draws at either strength.
    private static func attributed(_ text: String) -> NSAttributedString {
        NSAttributedString(string: text, attributes: [
            .font: font,
            NSAttributedString.Key(kCTForegroundColorFromContextAttributeName as String): true,
        ])
    }

    static func height(_ text: String, width: CGFloat) -> CGFloat {
        let setter = CTFramesetterCreateWithAttributedString(attributed(text))
        let size = CTFramesetterSuggestFrameSizeWithConstraints(setter, CFRange(location: 0, length: 0), nil,
                                                                CGSize(width: width, height: .greatestFiniteMagnitude), nil)
        return ceil(size.height)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        drawn = nil
        setNeedsDisplay()
    }

    override func draw(_ rect: CGRect) {
        guard let setter = framesetter, let ctx = UIGraphicsGetCurrentContext() else { return }
        if drawn == nil {
            drawn = CTFramesetterCreateFrame(setter, CFRange(location: 0, length: 0), CGPath(rect: bounds, transform: nil), nil)
        }
        guard let frame = drawn, let lines = CTFrameGetLines(frame) as? [CTLine] else { return }
        var origins = [CGPoint](repeating: .zero, count: lines.count)
        CTFrameGetLineOrigins(frame, CFRange(location: 0, length: 0), &origins)
        ctx.textMatrix = .identity
        ctx.translateBy(x: 0, y: bounds.height)
        ctx.scaleBy(x: 1, y: -1)
        let label = Theme.Card.label
        for (line, origin) in zip(lines, origins) {
            let range = CTLineGetStringRange(line)
            let from = CGFloat(range.location), to = CGFloat(range.location + range.length)
            let edge: CGFloat
            if sung >= to {
                edge = bounds.width
            } else if sung <= from {
                edge = 0
            } else {
                let i = Int(sung)
                let x0 = CTLineGetOffsetForStringIndex(line, i, nil)
                let x1 = CTLineGetOffsetForStringIndex(line, min(i + 1, range.location + range.length), nil)
                edge = origin.x + x0 + (x1 - x0) * (sung - CGFloat(i))
            }
            var ascent: CGFloat = 0, descent: CGFloat = 0
            CTLineGetTypographicBounds(line, &ascent, &descent, nil)
            let band = CGRect(x: 0, y: origin.y - descent - 2, width: bounds.width, height: ascent + descent + 4)
            for (part, alpha) in [(CGRect(x: 0, y: band.minY, width: edge, height: band.height), strength),
                                  (CGRect(x: edge, y: band.minY, width: bounds.width - edge, height: band.height), rest)]
                where part.width > 0 {
                ctx.saveGState()
                ctx.clip(to: part)
                ctx.setFillColor(label.withAlphaComponent(alpha).cgColor)
                ctx.textPosition = origin
                CTLineDraw(line, ctx)
                ctx.restoreGState()
            }
        }
    }
}

final class LyricCell: UITableViewCell {
    static let id = "lyric"
    static let inset = UIEdgeInsets(top: 6, left: 20, bottom: 6, right: 20)
    let line = LyricLineView()

    override init(style: UITableViewCell.CellStyle, reuseIdentifier: String?) {
        super.init(style: style, reuseIdentifier: reuseIdentifier)
        backgroundColor = Theme.Card.background
        selectionStyle = .none
        line.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(line)
        let i = LyricCell.inset
        NSLayoutConstraint.activate([
            line.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: i.left),
            line.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -i.right),
            line.topAnchor.constraint(equalTo: contentView.topAnchor, constant: i.top),
            line.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -i.bottom),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }
}

/// The song's lyrics; the sung line lit and, with word times, filled word by word. Paced by the core's
/// clock: a display link only while a word fills, a one-shot timer between, nothing while paused,
/// waiting for bytes, out of sight or locked. A line tapped seeks there.
final class LyricsCard: UITableViewController {
    private let song: String
    private var lines: [String] = []
    private var synced = false
    private var clock: OpaquePointer?
    private var sweeps = false
    private var active = -1
    private var sung: CGFloat = 0
    private var pacer: Timer?
    private var link: CADisplayLink?
    private var heights: [CGFloat] = []
    private let unsung = CGFloat(nori_ios_lyric_unsung())

    init(song: String) {
        self.song = song
        super.init(style: .plain)
    }

    required init?(coder: NSCoder) { fatalError() }

    deinit { nori_ios_lyric_free(clock) }

    override func viewDidLoad() {
        super.viewDidLoad()
        tableView.backgroundColor = Theme.Card.background
        tableView.separatorStyle = .none
        tableView.contentInset = UIEdgeInsets(top: 8, left: 0, bottom: 200, right: 0)
        tableView.register(LyricCell.self, forCellReuseIdentifier: LyricCell.id)
        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(arrived(_:)), name: .noriLyrics, object: nil)
        center.addObserver(self, selector: #selector(moved), name: .noriNow, object: nil)
        center.addObserver(self, selector: #selector(moved), name: UIApplication.didBecomeActiveNotification, object: nil)
        center.addObserver(self, selector: #selector(stop), name: UIApplication.didEnterBackgroundNotification, object: nil)
        load()
        song.withCString { nori_ios_lyrics($0) }
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        tick(force: true)
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        stop()
    }

    /// Pulled down past the first line and let go: the lyrics close.
    override func scrollViewWillEndDragging(_ scrollView: UIScrollView, withVelocity velocity: CGPoint,
                                            targetContentOffset: UnsafeMutablePointer<CGPoint>) {
        if scrollView.contentOffset.y + scrollView.adjustedContentInset.top < -DragToClose.pull {
            parent?.dismiss(animated: true)
        }
    }

    @objc private func arrived(_ n: Notification) {
        guard n.object as? String == song else { return }
        load()
    }

    /// A seek, a pause or a resume: the clock is asked at once.
    @objc private func moved() { tick(force: true) }

    private func load() {
        nori_ios_lyric_free(clock)
        clock = nil
        guard let d = song.withCString({ takenJSON(nori_ios_lyrics_page($0)) }) as? [String: Any] else {
            if lines.isEmpty { lines = [Say.lookingForLyrics] }
            reloadLines()
            return
        }
        synced = d["synced"] as? Bool ?? false
        lines = (d["lines"] as? [[String: Any]] ?? []).map { $0["t"] as? String ?? "" }
        if lines.isEmpty {
            synced = false
            lines = [Say.noLyrics]
        }
        clock = song.withCString { nori_ios_lyric_clock($0, Int64(Core.shared.now.position)) }
        sweeps = clock.map { nori_ios_lyric_sweeps($0) == 1 } ?? false
        active = -1
        reloadLines()
        tick(force: true)
    }

    private func reloadLines() {
        let width = max(1, tableView.bounds.width) - LyricCell.inset.left - LyricCell.inset.right
        heights = lines.map { LyricLineView.height($0, width: width) + LyricCell.inset.top + LyricCell.inset.bottom }
        tableView.reloadData()
    }

    @objc private func stop() {
        pacer?.invalidate()
        pacer = nil
        link?.invalidate()
        link = nil
    }

    @objc private func frame() { tick(force: false) }

    private func tick(force: Bool) {
        guard let clock, view.window != nil, UIApplication.shared.applicationState == .active else { return stop() }
        let now = Core.shared.now
        var step = NoriLyricStep()
        nori_ios_lyric_advance(clock, Int64(now.position), 1, force ? 1 : 0, &step)
        if step.redraw != 0 { show(step) }
        // While bytes are on their way, or paused, the place stands still: nothing runs.
        guard now.playing, !now.buffering, step.wait > 0 else { return stop() }
        if sweeps && step.still == 0 {
            pacer?.invalidate()
            pacer = nil
            if link == nil {
                let l = CADisplayLink(target: self, selector: #selector(frame))
                l.add(to: .main, forMode: .common)
                link = l
            }
            link?.preferredFramesPerSecond = max(1, 60 / Int(step.wait))
        } else {
            link?.invalidate()
            link = nil
            pacer?.invalidate()
            let t = Timer(timeInterval: Double(step.wait) / 1000, repeats: false) { [weak self] _ in self?.tick(force: false) }
            t.tolerance = 0.01
            RunLoop.main.add(t, forMode: .common)
            pacer = t
        }
    }

    private func show(_ step: NoriLyricStep) {
        let was = active
        active = Int(step.active)
        sung = CGFloat(step.sung)
        guard active != was else {
            if active >= 0, active < lines.count, let cell = tableView.cellForRow(at: IndexPath(row: active, section: 0)) as? LyricCell {
                paint(cell.line, row: active)
            }
            return
        }
        let glide = UIAccessibility.isReduceMotionEnabled ? 0 : Double(step.glide_ms) / 1000
        for case let cell as LyricCell in tableView.visibleCells {
            guard let row = tableView.indexPath(for: cell)?.row else { continue }
            UIView.transition(with: cell.line, duration: glide, options: .transitionCrossDissolve, animations: {
                self.paint(cell.line, row: row)
            })
        }
        if active >= 0, active < lines.count {
            tableView.scrollToRow(at: IndexPath(row: active, section: 0), at: .middle, animated: glide > 0)
        }
    }

    private func paint(_ line: LyricLineView, row: Int) {
        let strength = CGFloat(nori_ios_lyric_strength(synced ? 1 : 0, Int32(row), Int32(active)))
        line.strength = strength
        if row == active && sweeps {
            line.sung = sung
            line.rest = min(unsung, strength)
        } else {
            line.sung = .greatestFiniteMagnitude
            line.rest = strength
        }
    }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { lines.count }

    override func tableView(_ tableView: UITableView, heightForRowAt indexPath: IndexPath) -> CGFloat {
        indexPath.row < heights.count ? heights[indexPath.row] : 44
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(withIdentifier: LyricCell.id, for: indexPath) as! LyricCell
        cell.line.text = lines[indexPath.row]
        paint(cell.line, row: indexPath.row)
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        guard synced, let clock else { return }
        // The line lights at once; the clock stands while the engine waits for bytes.
        nori_ios_seek(nori_ios_lyric_tap(clock, Int32(indexPath.row)))
        tick(force: true)
    }
}

/// The lock screen, Control Center, the EarPods clicker and Bluetooth controls; the system volume
/// fed to loudness compensation.
final class NowPlaying: NSObject {
    static let shared = NowPlaying()
    private var artworkFor = ""
    private var artwork: MPMediaItemArtwork?
    private var volumeWatch: NSKeyValueObservation?

    func start() {
        let c = MPRemoteCommandCenter.shared()
        c.playCommand.addTarget { _ in
            if !Core.shared.now.playing { nori_ios_toggle() }
            return .success
        }
        c.pauseCommand.addTarget { _ in
            if Core.shared.now.playing { nori_ios_toggle() }
            return .success
        }
        c.togglePlayPauseCommand.addTarget { _ in nori_ios_toggle(); return .success }
        c.nextTrackCommand.addTarget { _ in nori_ios_next(); return .success }
        c.previousTrackCommand.addTarget { _ in nori_ios_previous(); return .success }
        c.changePlaybackPositionCommand.addTarget { event in
            guard let e = event as? MPChangePlaybackPositionCommandEvent else { return .commandFailed }
            nori_ios_seek(Int64(e.positionTime * 1000))
            return .success
        }
        NotificationCenter.default.addObserver(self, selector: #selector(changed), name: .noriNow, object: nil)
        let session = AVAudioSession.sharedInstance()
        nori_ios_volume(session.outputVolume)
        volumeWatch = session.observe(\.outputVolume, options: [.new]) { s, _ in
            nori_ios_volume(s.outputVolume)
        }
    }

    @objc private func changed() {
        let now = Core.shared.now
        guard let song = now.song else {
            MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
            return
        }
        var info: [String: Any] = [
            MPMediaItemPropertyTitle: song.title,
            MPMediaItemPropertyArtist: song.subtitle,
            MPMediaItemPropertyAlbumTitle: song.album,
            MPMediaItemPropertyPlaybackDuration: Double(song.seconds),
            MPNowPlayingInfoPropertyElapsedPlaybackTime: Double(now.ms) / 1000,
            MPNowPlayingInfoPropertyPlaybackRate: now.playing ? now.pace : 0.0,
            MPNowPlayingInfoPropertyPlaybackQueueIndex: max(0, now.index),
            MPNowPlayingInfoPropertyPlaybackQueueCount: now.length,
        ]
        if artworkFor == song.cover {
            if artwork == nil, let image = CoverCache.any(song.cover) {
                artwork = MPMediaItemArtwork(boundsSize: image.size) { _ in image }
            }
            if let artwork { info[MPMediaItemPropertyArtwork] = artwork }
        } else if !song.cover.isEmpty {
            let id = song.cover
            artworkFor = id
            artwork = nil
            // At the player card's own size, kept in the cache: the card then opens on its picture.
            let px = PlayerCard.coverPx
            _ = Core.shared.cover(id, px: px) { [weak self] image in
                guard let image else { return }
                CoverCache.put(image, id, px)
                guard let self, self.artworkFor == id else { return }
                self.artwork = MPMediaItemArtwork(boundsSize: image.size) { _ in image }
                self.changed()
            }
        }
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
    }
}

/// The player's title as Android's `readable` line: one too long for its width waits, walks sideways round
/// to its start again, twice, then settles with a soft right edge. It reads out when its text changes and
/// when it comes into view; a line that fits stays still. Core Animation moves it, off the main thread.
final class ReadingLine: UIView {
    private let strip = UIView()
    private let first = UILabel()
    private let second = UILabel()
    private let fade = CAGradientLayer()
    private var fresh = true

    private static let delay: CFTimeInterval = 2.6
    private static let gap: CGFloat = 46
    private static let speed: CGFloat = 26
    private static let soft: CGFloat = 20
    private static let walks: Float = 2

    var text: String? {
        get { first.text }
        set {
            guard newValue != first.text else { return }
            first.text = newValue
            second.text = newValue
            accessibilityLabel = newValue
            strip.layer.removeAnimation(forKey: "walk")
            fresh = true
            invalidateIntrinsicContentSize()
            setNeedsLayout()
        }
    }
    var font: UIFont {
        get { first.font }
        set { first.font = newValue; second.font = newValue; invalidateIntrinsicContentSize() }
    }
    var textColor: UIColor {
        get { first.textColor }
        set { first.textColor = newValue; second.textColor = newValue }
    }
    /// The page the soft edge fades into.
    var fadeColor = Theme.Card.background {
        didSet { fade.colors = [fadeColor.cgColor, fadeColor.cgColor, UIColor.clear.cgColor] }
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        clipsToBounds = true
        isAccessibilityElement = true
        addSubview(strip)
        strip.addSubview(first)
        strip.addSubview(second)
        fade.startPoint = CGPoint(x: 0, y: 0.5)
        fade.endPoint = CGPoint(x: 1, y: 0.5)
        fade.colors = [Theme.Card.background.cgColor, Theme.Card.background.cgColor, UIColor.clear.cgColor]
    }
    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: CGSize {
        CGSize(width: UIView.noIntrinsicMetric, height: first.intrinsicContentSize.height)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil {
            strip.layer.removeAnimation(forKey: "walk")
        } else {
            fresh = true
            setNeedsLayout()
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let needs = first.intrinsicContentSize.width
        let over = needs > bounds.width + 1
        let h = bounds.height
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        first.frame = CGRect(x: 0, y: 0, width: over ? needs : bounds.width, height: h)
        second.frame = CGRect(x: needs + ReadingLine.gap, y: 0, width: needs, height: h)
        second.isHidden = !over
        strip.frame = CGRect(x: 0, y: 0, width: over ? second.frame.maxX : bounds.width, height: h)
        if over {
            fade.frame = bounds
            fade.locations = [0, NSNumber(value: Double(1 - ReadingLine.soft / max(bounds.width, ReadingLine.soft))), 1]
            layer.mask = fade
        } else {
            layer.mask = nil
        }
        CATransaction.commit()
        guard fresh, window != nil else { return }
        fresh = false
        strip.layer.removeAnimation(forKey: "walk")
        guard over, !UIAccessibility.isReduceMotionEnabled else { return }
        // Round to where the second copy stands, which looks the same as the start.
        let distance = needs + ReadingLine.gap
        let walk = CFTimeInterval(distance / ReadingLine.speed)
        let a = CAKeyframeAnimation(keyPath: "transform.translation.x")
        a.values = [0, 0, -distance]
        a.keyTimes = [0, NSNumber(value: ReadingLine.delay / (ReadingLine.delay + walk)), 1]
        a.duration = ReadingLine.delay + walk
        a.repeatCount = ReadingLine.walks
        strip.layer.add(a, forKey: "walk")
    }
}
