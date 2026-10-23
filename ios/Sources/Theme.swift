import UIKit

/// The season's colors (crates/look, through `nori_ios_look`): page, surface, ink, an accent for what is
/// "you, now", and the color of a switch that is on. The greys are shares of the ink over the page. iOS 12
/// has no system dark mode or dynamic colors, so each color is read when a view is built, and a change of
/// mode, season or date builds the views again.
enum Theme {
    /// The `theme` and `season` settings as last seen, kept for the launch before the settings are open.
    private static let keptMode = "nori.theme.mode"
    private static let keptSeason = "nori.theme.season"
    /// The `theme` setting's value: SYSTEM, LIGHT, DARK or MIDNIGHT.
    static private(set) var mode = UserDefaults.standard.string(forKey: keptMode) ?? "SYSTEM"
    /// The `season` setting's value: FOLLOW, SPRING, SUMMER, AUTUMN or WINTER.
    static private(set) var season = UserDefaults.standard.string(forKey: keptSeason) ?? "FOLLOW"

    /// The four modes, as the `theme` setting names them, in Android's order.
    static let modes = ["SYSTEM", "LIGHT", "DARK", "MIDNIGHT"]
    /// The seasons a tile picks, spring first, as the `season` setting names them.
    static let seasons = ["SPRING", "SUMMER", "AUTUMN", "WINTER"]

    /// Takes the settings' mode and season and today's date; true when the colors moved and the views
    /// want building again.
    static func keep(mode m: String, season s: String) -> Bool {
        mode = m
        season = s
        UserDefaults.standard.set(m, forKey: keptMode)
        UserDefaults.standard.set(s, forKey: keptSeason)
        let before = look
        look = wear(season)
        return !same(look, before)
    }

    /// iOS 12 has no scheme of its own: System is Night here, the iPod's default (the owner's choice).
    private static let systemDark: Int32 = 1

    /// The core's look for `season` in `mode`, on today's date and time zone.
    private static func wear(_ season: String, _ mode: String = Theme.mode) -> NoriLook {
        var out = NoriLook()
        let zone = TimeZone.current
        nori_ios_look(season, mode, systemDark, Int64(Date().timeIntervalSince1970), Int32(zone.secondsFromGMT() / 60), zone.identifier, &out)
        return out
    }

    private static func same(_ a: NoriLook, _ b: NoriLook) -> Bool {
        let p = { (x: NoriPalette) in [x.page, x.surface, x.ink, x.accent, x.on] }
        return p(a.worn) == p(b.worn) && a.dark_theme == b.dark_theme && a.covers == b.covers
    }

    private(set) static var look = wear(season)

    /// The palette each season's tile shows, spring first: its day or night one, as the app is (Night's in
    /// Midnight: on black every tile would be the same).
    static func tiles() -> [NoriPalette] { seasons.map { wear($0, dark ? "DARK" : "LIGHT").worn } }

    /// The season now and the next one with its start, for the settings page.
    static func calendar() -> NoriSeasonNow {
        var out = NoriSeasonNow()
        let zone = TimeZone.current
        nori_ios_season_now(Int64(Date().timeIntervalSince1970), Int32(zone.secondsFromGMT() / 60), zone.identifier, &out)
        return out
    }

    static var dark: Bool { look.dark_theme != 0 }
    /// Pages and the player wear their covers' colors (not in Midnight).
    static var coversTint: Bool { look.covers != 0 }

    /// The page the app wears.
    static var page: PageLook { PageLook(look.worn) }

    static var background: UIColor { UIColor(argb: look.worn.page) }
    /// A raised surface: a card, a sheet, a selected chip.
    static var row: UIColor { UIColor(argb: look.worn.surface) }
    static var label: UIColor { UIColor(argb: look.worn.ink) }
    static var secondary: UIColor { UIColor(argb: look.worn.quiet) }
    /// What is "you, now": the current tab, the song playing, a lit toggle, a liked seal, a focused field.
    static var accent: UIColor { UIColor(argb: look.worn.accent) }
    /// A switch's track when on.
    static var switchOn: UIColor { UIColor(argb: look.worn.on) }
    /// A plate under a missing cover, a slider's track.
    static var track: UIColor { page.veil(0.12) }
    static var hairline: UIColor { page.hairline }
    static var bar: UIBarStyle { dark ? .black : .default }
    static var keyboard: UIKeyboardAppearance { dark ? .dark : .light }
    static var statusBar: UIStatusBarStyle { dark ? .lightContent : .default }

    /// The player's page: the season's paper dyed with the song's cover (not in Midnight), which the
    /// player sets as the song changes; its queue, lyrics and devices open in it.
    static var player = page

    /// The player card, its queue, its lyrics and its devices: the player's colors.
    enum Card {
        static var background: UIColor { player.page }
        static var label: UIColor { player.ink }
        static var secondary: UIColor { player.quiet }
        static var accent: UIColor { player.accent }
        static var dim: UIColor { player.veil(0.36) }
        static var track: UIColor { player.veil(0.2) }
    }

    static func apply(nav: UINavigationBar) {
        nav.barStyle = bar
        nav.barTintColor = background
        nav.isTranslucent = false
        nav.tintColor = label
        nav.shadowImage = UIImage()
        nav.setBackgroundImage(UIImage.line(background), for: .default)
        nav.titleTextAttributes = [.foregroundColor: label, .font: Fonts.font(17, .semibold)]
        nav.largeTitleTextAttributes = [.foregroundColor: label, .font: Fonts.title(31)]
        nav.prefersLargeTitles = true
        nav.backIndicatorImage = Glyph.back
        nav.backIndicatorTransitionMaskImage = Glyph.back
    }
}

/// The colors one page wears: the theme's, or the season's paper dyed with a cover's (`nori_ios_cover_look`).
struct PageLook: Equatable {
    var page: UIColor
    var surface: UIColor
    var ink: UIColor
    var quiet: UIColor
    var accent: UIColor
    var hairline: UIColor
    /// The dissolve from a picture into the page: its colors and where they stand, top to bottom.
    var melt: [UIColor]
    var stops: [CGFloat]

    init(_ p: NoriPalette) {
        page = UIColor(argb: p.page)
        surface = UIColor(argb: p.surface)
        ink = UIColor(argb: p.ink)
        quiet = UIColor(argb: p.quiet)
        accent = UIColor(argb: p.accent)
        hairline = UIColor(argb: p.page).mixed(UIColor(argb: p.ink), 0.14)
        melt = [page.withAlphaComponent(0), page.withAlphaComponent(0.4), page.withAlphaComponent(0.86), page]
        stops = [0.60, 0.76, 0.88, 1.0]
    }

    /// A cover's page as the core measured it.
    init?(_ d: [String: Any]) {
        let c = { (k: String) in (d[k] as? NSNumber).map { UIColor(argb: $0.uint32Value) } }
        guard let page = c("page"), let surface = c("surface"), let ink = c("ink"), let quiet = c("quiet"),
              let accent = c("accent"), let hairline = c("hairline"), let edge = c("edge"), let mid = c("mid") else { return nil }
        self.page = page
        self.surface = surface
        self.ink = ink
        self.quiet = quiet
        self.accent = accent
        self.hairline = hairline
        melt = [edge.withAlphaComponent(0), edge, mid, page]
        stops = (d["stops"] as? [Double])?.map { CGFloat($0) } ?? [0.60, 0.76, 0.88, 1.0]
    }

    /// The ink over the page at `share`: the greys of the page.
    func veil(_ share: CGFloat) -> UIColor { page.mixed(ink, share) }
}

extension UIColor {
    /// An opaque RGB color.
    convenience init(rgb: UInt32) {
        self.init(
            red: CGFloat((rgb >> 16) & 0xFF) / 255,
            green: CGFloat((rgb >> 8) & 0xFF) / 255,
            blue: CGFloat(rgb & 0xFF) / 255,
            alpha: 1
        )
    }

    /// An ARGB color, as the core hands them out.
    convenience init(argb: UInt32) {
        self.init(
            red: CGFloat((argb >> 16) & 0xFF) / 255,
            green: CGFloat((argb >> 8) & 0xFF) / 255,
            blue: CGFloat(argb & 0xFF) / 255,
            alpha: CGFloat(argb >> 24) / 255
        )
    }

    /// This color moved `share` of the way to `other`.
    func mixed(_ other: UIColor, _ share: CGFloat) -> UIColor {
        var a: (CGFloat, CGFloat, CGFloat, CGFloat) = (0, 0, 0, 0), b = a
        getRed(&a.0, green: &a.1, blue: &a.2, alpha: &a.3)
        other.getRed(&b.0, green: &b.1, blue: &b.2, alpha: &b.3)
        return UIColor(red: a.0 + (b.0 - a.0) * share, green: a.1 + (b.1 - a.1) * share, blue: a.2 + (b.2 - a.2) * share, alpha: 1)
    }
}

extension UIImage {
    /// A one-point picture of `color`, for a bar's background and edge.
    static func line(_ color: UIColor) -> UIImage {
        UIGraphicsBeginImageContextWithOptions(CGSize(width: 1, height: 1), false, 0)
        defer { UIGraphicsEndImageContext() }
        color.setFill()
        UIRectFill(CGRect(x: 0, y: 0, width: 1, height: 1))
        return UIGraphicsGetImageFromCurrentImageContext() ?? UIImage()
    }
}
