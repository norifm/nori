import UIKit

/// What this iPod played, newest first, a page at a time, as Android's History: Listening stats and Clear
/// history under its name.
final class HistoryPage: PageController {
    init() { super.init(kind: NORI_PAGE_HISTORY, title: Say.history) }
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        let stats = TextAction(Say.listeningStats) { [weak self] in
            self?.navigationController?.pushViewController(StatsPage(), animated: true)
        }
        let clear = TextAction(Say.clearHistory) { [weak self] in
            DispatchQueue.global(qos: .userInitiated).async {
                let done = nori_ios_history_clear() != 0
                DispatchQueue.main.async { done ? self?.reload() : Toast.show(Say.failed) }
            }
        }
        let row = UIStackView(arrangedSubviews: [stats, clear, UIView()])
        row.spacing = 2
        row.isLayoutMarginsRelativeArrangement = true
        row.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 0, leading: Space.gutter - 10, bottom: 6, trailing: Space.gutter)
        underName = row
    }
}

/// The listening stats, as Android's (from History): whose plays and how far back as chips, the plays in large
/// type and the numbers under them on plates, the listening day as 24 bars and when it peaks, then the top
/// lists. The core counts it all (`nori_ios_read`, NORI_PAGE_STATS).
final class StatsPage: CardsPage {
    private var token: UInt64 = 0
    /// How the server account's plays stood when first asked for; they are asked for once a visit.
    private var server: String?

    override func viewDidLoad() {
        super.viewDidLoad()
        title = Say.listening
        set([])
        read(nil)
    }

    deinit {
        if token != 0 { Core.shared.forget(token) }
    }

    /// Reads the stats of `choice` (period and source, by the core's names); none: those the page opens on.
    private func read(_ choice: (period: String, source: String)?) {
        if token != 0 { Core.shared.forget(token) }
        let arg = choice.map { "\($0.period)\n\($0.source)\n\($0.source == "ServerAccount" && server == nil ? 1 : 0)" } ?? ""
        token = Core.shared.read(NORI_PAGE_STATS, arg) { [weak self] a in self?.show(a.raw) }
    }

    private func show(_ d: [String: Any]) {
        if let s = d["server"] as? String { server = s }
        let periods = d["periods"] as? [String] ?? []
        let sources = d["sources"] as? [String] ?? []
        let period = d["period"] as? String ?? ""
        let source = d["source"] as? String ?? ""
        let fromServer = source == "ServerAccount"
        var views: [UIView] = []

        let whose = ChipStrip()
        whose.set(sources.map(Say.statsChoice), chosen: sources.firstIndex(of: source) ?? 0)
        whose.picked = { [weak self] i in self?.read((period, sources[i])) }
        let when = ChipStrip()
        when.set(periods.map(Say.statsChoice), chosen: periods.firstIndex(of: period) ?? 0)
        when.picked = { [weak self] i in self?.read((periods[i], source)) }
        views += [padded(whose, top: 4, bottom: 4, sides: 0), padded(when, top: 4, bottom: 4, sides: 0)]

        // The headline: two numbers worth reading from across the room, the rest on plates.
        let plays = label(Fonts.digits(36, .bold), Theme.label)
        plays.text = "\(d["plays"] as? Int ?? 0)"
        let what = label(Fonts.font(16, .medium, .headline), Theme.secondary, lines: 0)
        what.text = Say.statsHeadline(server: fromServer, ms: d["ms"] as? Int ?? 0)
        let head = UIStackView(arrangedSubviews: [plays, what])
        head.axis = .vertical
        if fromServer {
            let note = label(Fonts.font(13, .regular, .footnote), Theme.secondary, lines: 0)
            note.text = Say.statsServerNote(server)
            head.addArrangedSubview(note)
            head.setCustomSpacing(6, after: what)
        }
        views.append(padded(head, top: 14, bottom: 14))
        let tiles = (d["tiles"] as? [[String: Any]] ?? []).map { StatPlate(value: $0["n"] as? Int ?? 0, words: Say.statKind($0["k"] as? String ?? "")) }
        for row in [Array(tiles.prefix(3)), Array(tiles.dropFirst(3))] where !row.isEmpty {
            let line = UIStackView(arrangedSubviews: row)
            line.spacing = 8
            line.distribution = .fillEqually
            views.append(padded(line, top: 0, bottom: 8))
        }

        if let hour = d["hour"] as? Int {
            views.append(SectionTitle(Say.whenYouListen))
            let hours = (d["hours"] as? [NSNumber] ?? []).map { CGFloat($0.doubleValue) }
            views.append(padded(HourChart(hours), top: 0, bottom: 0))
            let habit = label(Fonts.font(14, .regular, .subheadline), Theme.secondary, lines: 0)
            habit.text = Say.statsHabit(hour: hour, weekday: d["weekday"] as? Int)
            views.append(padded(habit, top: 6, bottom: 0))
        }

        let timed = { (e: [String: Any]) in fromServer ? "" : Fmt.clock(ms: e["ms"] as? Int ?? 0) }
        let lists: [(String, String, ([String: Any]) -> String)] = [
            (Say.topSongs, "songs", { $0["s"] as? String ?? "" }),
            (Say.topArtists, "artists", timed),
            (Say.topAlbums, "albums", { _ in "" }),
            (Say.topGenres, "genres", { _ in "" }),
        ]
        for (title, key, sub) in lists {
            let rows = d[key] as? [[String: Any]] ?? []
            guard !rows.isEmpty else { continue }
            views.append(SectionTitle(title))
            for (i, e) in rows.enumerated() {
                views.append(RankRow(rank: i + 1, title: e["t"] as? String ?? "", subtitle: sub(e), count: e["n"] as? Int ?? 0))
            }
        }
        set(views)
    }

    /// `v` inset by the gutter at its sides.
    private func padded(_ v: UIView, top: CGFloat, bottom: CGFloat, sides: CGFloat = Space.gutter) -> UIView {
        let box = UIView()
        v.translatesAutoresizingMaskIntoConstraints = false
        box.addSubview(v)
        NSLayoutConstraint.activate([
            v.leadingAnchor.constraint(equalTo: box.leadingAnchor, constant: sides),
            v.trailingAnchor.constraint(equalTo: box.trailingAnchor, constant: -sides),
            v.topAnchor.constraint(equalTo: box.topAnchor, constant: top),
            v.bottomAnchor.constraint(equalTo: box.bottomAnchor, constant: -bottom),
        ])
        return box
    }
}

/// One number of the listening stats and what it counts, on a plate of the page's ink.
private final class StatPlate: UIView {
    init(value: Int, words: String) {
        super.init(frame: .zero)
        backgroundColor = Theme.page.veil(0.06)
        layer.cornerRadius = Radius.card
        let number = label(Fonts.digits(22, .semibold), Theme.label)
        number.text = "\(value)"
        number.adjustsFontSizeToFitWidth = true
        number.minimumScaleFactor = 0.7
        let what = label(Fonts.font(12, .medium, .caption1), Theme.secondary)
        what.text = words
        let column = UIStackView(arrangedSubviews: [number, what])
        column.axis = .vertical
        column.spacing = 2
        column.translatesAutoresizingMaskIntoConstraints = false
        addSubview(column)
        NSLayoutConstraint.activate([
            column.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            column.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            column.topAnchor.constraint(equalTo: topAnchor, constant: 12),
            column.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -12),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }
}

/// Twenty-four bars, drawn: the shape of a listening day says more than its busiest hour alone. The hours
/// are marked under them.
private final class HourChart: UIView {
    private let hours: [CGFloat]
    private static let tall: CGFloat = 96
    private static let ticks = [0, 6, 12, 18, 23]

    init(_ hours: [CGFloat]) {
        self.hours = hours
        super.init(frame: .zero)
        backgroundColor = .clear
        isOpaque = false
        isAccessibilityElement = false
        heightAnchor.constraint(equalToConstant: HourChart.tall + 20).isActive = true
    }

    required init?(coder: NSCoder) { fatalError() }

    override func draw(_ rect: CGRect) {
        let step = bounds.width / 24
        let w = step * 0.72
        let track = Theme.page.veil(0.12)
        for (h, share) in hours.enumerated() {
            let x = CGFloat(h) * step
            track.setFill()
            UIBezierPath(roundedRect: CGRect(x: x, y: 0, width: w, height: HourChart.tall), cornerRadius: w / 2).fill()
            guard share > 0 else { continue }
            let tall = max(w, HourChart.tall * share)
            Theme.label.setFill()
            UIBezierPath(roundedRect: CGRect(x: x, y: HourChart.tall - tall, width: w, height: tall), cornerRadius: w / 2).fill()
        }
        let attributes: [NSAttributedString.Key: Any] = [.font: Fonts.font(11, .semibold, .caption2), .foregroundColor: Theme.secondary]
        for h in HourChart.ticks {
            let words = NSAttributedString(string: String(format: "%02d", h), attributes: attributes)
            let size = words.size()
            words.draw(at: CGPoint(x: min(bounds.width - size.width, max(0, CGFloat(h) * step + (w - size.width) / 2)), y: HourChart.tall + 4))
        }
    }
}

/// A place in a top list: its rank, what it is and the line under it, and how often it played.
private final class RankRow: UIView {
    init(rank: Int, title: String, subtitle: String, count: Int) {
        super.init(frame: .zero)
        let number = label(Fonts.digits(15, .semibold), Theme.secondary)
        number.text = "\(rank)"
        number.textAlignment = .center
        let name = label(Fonts.font(16), Theme.label)
        name.text = title
        let under = label(Fonts.font(13, .regular, .footnote), Theme.secondary)
        under.text = subtitle
        under.isHidden = subtitle.isEmpty
        let text = UIStackView(arrangedSubviews: [name, under])
        text.axis = .vertical
        text.spacing = 2
        let plays = label(Fonts.digits(14), Theme.secondary)
        plays.text = "\(count)"
        plays.setContentHuggingPriority(.required, for: .horizontal)
        plays.setContentCompressionResistancePriority(.required, for: .horizontal)
        let row = UIStackView(arrangedSubviews: [number, text, plays])
        row.alignment = .center
        row.spacing = Space.row
        row.translatesAutoresizingMaskIntoConstraints = false
        addSubview(row)
        NSLayoutConstraint.activate([
            number.widthAnchor.constraint(equalToConstant: 24),
            row.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Space.gutter - 4),
            row.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -Space.gutter),
            row.topAnchor.constraint(equalTo: topAnchor),
            row.bottomAnchor.constraint(equalTo: bottomAnchor),
            heightAnchor.constraint(greaterThanOrEqualToConstant: 52),
        ])
        isAccessibilityElement = true
        accessibilityLabel = [title, subtitle, "\(count)"].filter { !$0.isEmpty }.joined(separator: ", ")
    }

    required init?(coder: NSCoder) { fatalError() }
}
