import UIKit

/// The season's colours (crates/look, through `nori_ios_palette`): page, surface, ink, an accent for what is
/// "you, now", and the colour of a switch that is on. The greys are the ink over the page. iOS 12 has no
/// system dark mode or dynamic colours, so each colour is read from the mode when a view is built, and a
/// change of mode, season or date builds the views again.
enum Theme {
    /// The `theme` and `season` settings as last seen, kept for the launch before the settings are open.
    private static let kept = "nori.theme.light"
    private static let keptSeason = "nori.theme.season"
    static var light = UserDefaults.standard.bool(forKey: kept)
    /// The `season` setting's value: FOLLOW, SPRING, SUMMER, AUTUMN or WINTER.
    static private(set) var season = UserDefaults.standard.string(forKey: keptSeason) ?? "FOLLOW"

    /// The `theme` setting's value: the system choice is dark, iOS 12 having no system mode.
    static func isLight(_ setting: String) -> Bool { setting == "LIGHT" }

    /// Takes the settings' mode and season and today's date; true when the colours moved and the views
    /// want building again.
    static func keep(light on: Bool, season value: String) -> Bool {
        light = on
        season = value
        UserDefaults.standard.set(on, forKey: kept)
        UserDefaults.standard.set(value, forKey: keptSeason)
        let before = colors
        colors = palette()
        return colors != before
    }

    private struct Colors: Equatable {
        var page, surface, ink, accent, on, quiet: UIColor
    }

    /// The core's palette for the mode and season on today's date and time zone.
    private static func palette() -> Colors {
        var out = NoriPalette()
        let zone = TimeZone.current
        nori_ios_palette(season, light ? 0 : 1, Int64(Date().timeIntervalSince1970), Int32(zone.secondsFromGMT() / 60), zone.identifier, &out)
        return Colors(page: UIColor(rgb: out.page), surface: UIColor(rgb: out.surface), ink: UIColor(rgb: out.ink),
                      accent: UIColor(rgb: out.accent), on: UIColor(rgb: out.on), quiet: UIColor(rgb: out.quiet))
    }

    private static var colors = palette()

    /// The season now and the next one with its start, for the settings page.
    static func calendar() -> NoriSeasonNow {
        var out = NoriSeasonNow()
        let zone = TimeZone.current
        nori_ios_season_now(Int64(Date().timeIntervalSince1970), Int32(zone.secondsFromGMT() / 60), zone.identifier, &out)
        return out
    }

    /// The ink over the page at `alpha`: the greys of the theme.
    private static func veil(_ alpha: CGFloat) -> UIColor {
        var ink: (CGFloat, CGFloat, CGFloat, CGFloat) = (0, 0, 0, 0), page = ink
        colors.ink.getRed(&ink.0, green: &ink.1, blue: &ink.2, alpha: &ink.3)
        colors.page.getRed(&page.0, green: &page.1, blue: &page.2, alpha: &page.3)
        return UIColor(red: page.0 + (ink.0 - page.0) * alpha, green: page.1 + (ink.1 - page.1) * alpha,
                       blue: page.2 + (ink.2 - page.2) * alpha, alpha: 1)
    }

    static var background: UIColor { colors.page }
    static var row: UIColor { colors.surface }
    static var label: UIColor { colors.ink }
    static var secondary: UIColor { colors.quiet }
    /// What is "you, now": the current tab, the song playing, a lit toggle, a focused field.
    static var accent: UIColor { colors.accent }
    /// A control that is off (shuffle, repeat) beside one that is on.
    static var dim: UIColor { veil(0.36) }
    /// A switch's track when on.
    static var switchOn: UIColor { colors.on }
    static var track: UIColor { veil(0.2) }
    static var hairline: UIColor { veil(0.14) }
    static var bar: UIBarStyle { light ? .default : .black }
    static var keyboard: UIKeyboardAppearance { light ? .light : .dark }
    static var statusBar: UIStatusBarStyle { light ? .default : .lightContent }

    /// The player card, its queue and its lyrics: the page's colours, whatever the cover.
    enum Card {
        static var background: UIColor { Theme.background }
        static var label: UIColor { Theme.label }
        static var secondary: UIColor { Theme.secondary }
        static var accent: UIColor { Theme.accent }
        static var dim: UIColor { Theme.dim }
        static var track: UIColor { Theme.track }
    }

    static func apply(tab: UITabBar) {
        tab.barStyle = bar
        tab.barTintColor = background
        tab.isTranslucent = false
        tab.tintColor = accent
        tab.unselectedItemTintColor = secondary
    }

    static func apply(nav: UINavigationBar) {
        nav.barStyle = bar
        nav.barTintColor = background
        nav.isTranslucent = false
        nav.tintColor = accent
        nav.titleTextAttributes = [.foregroundColor: label]
        nav.largeTitleTextAttributes = [.foregroundColor: label]
        nav.prefersLargeTitles = true
    }
}

extension UIAlertController {
    /// iOS 12's sheet is always light; the window's accent tint reads poorly on it, so its text is black.
    static func sheet(_ title: String?, _ message: String? = nil) -> UIAlertController {
        let sheet = UIAlertController(title: title, message: message, preferredStyle: .actionSheet)
        sheet.view.tintColor = .black
        if let title {
            let bold = UIFont.systemFont(ofSize: 13, weight: .semibold)
            sheet.setValue(NSAttributedString(string: title, attributes: [.foregroundColor: UIColor.black, .font: bold]), forKey: "attributedTitle")
        }
        if let message {
            let plain = UIFont.systemFont(ofSize: 13)
            sheet.setValue(NSAttributedString(string: message, attributes: [.foregroundColor: UIColor.black, .font: plain]), forKey: "attributedMessage")
        }
        return sheet
    }

    func add(_ title: String, checked: Bool = false, destructive: Bool = false, enabled: Bool = true, run: @escaping () -> Void) {
        let action = UIAlertAction(title: title, style: destructive ? .destructive : .default) { _ in run() }
        if checked { action.setValue(true, forKey: "checked") }
        action.isEnabled = enabled
        addAction(action)
    }

    func show(from host: UIViewController) {
        addAction(UIAlertAction(title: Say.cancel, style: .cancel))
        host.present(self, animated: true)
    }
}

extension UIColor {
    convenience init(rgb: UInt32) {
        self.init(
            red: CGFloat((rgb >> 16) & 0xFF) / 255,
            green: CGFloat((rgb >> 8) & 0xFF) / 255,
            blue: CGFloat(rgb & 0xFF) / 255,
            alpha: 1
        )
    }
}
