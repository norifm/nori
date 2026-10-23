import UIKit

/// The app's data directory: `Library/Application Support/nori`, excluded from backups (the stream cache
/// and downloads live under it).
func dataDirectory() -> URL {
    let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    var dir = support.appendingPathComponent("nori", isDirectory: true)
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    try? dir.setResourceValues(values)
    return dir
}

final class AppDelegate: UIResponder, UIApplicationDelegate {
    var window: UIWindow?

    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        let window = UIWindow(frame: UIScreen.main.bounds)
        window.rootViewController = ShellController()
        window.backgroundColor = Theme.background
        window.tintColor = Theme.accent
        window.makeKeyAndVisible()
        self.window = window
        Core.shared.start(in: window)
        NowPlaying.shared.start()
        Bonjour.shared.start()
        let dir = dataDirectory().path
        nori_ios_keep_log(dir, Int32(TimeZone.current.secondsFromGMT() / 60))
        DispatchQueue.global(qos: .userInitiated).async {
            let failed = dir.withCString { d in taken(nori_ios_open(d, nil)) }
            guard failed == nil else { return }
            let mode = SettingsPage.value("theme") ?? Theme.mode
            let season = SettingsPage.value("season") ?? Theme.season
            DispatchQueue.main.async {
                if Theme.keep(mode: mode, season: season) { self.restyle() }
                Core.shared.opened()
            }
        }
        return true
    }

    /// The date may have moved the season (nothing else ticks): the colours are read again.
    func applicationDidBecomeActive(_ application: UIApplication) {
        if Theme.keep(mode: Theme.mode, season: Theme.season) { restyle() }
    }

    /// Builds the interface again in the colours `Theme` now has, on the tab it was on.
    func restyle() {
        guard let window else { return }
        let tab = (window.rootViewController as? ShellController)?.selectedIndex ?? 0
        let shell = ShellController()
        shell.loadViewIfNeeded()
        shell.select(tab)
        window.backgroundColor = Theme.background
        window.tintColor = Theme.accent
        UIView.transition(with: window, duration: Motion.plain ? 0 : 0.25, options: .transitionCrossDissolve, animations: {
            window.rootViewController = shell
        })
    }

    /// A nori://jam invite opened from elsewhere (the relay's invite page): joins its jam.
    func application(_ app: UIApplication, open url: URL, options: [UIApplication.OpenURLOptionsKey: Any] = [:]) -> Bool {
        JamJoin.join(url.absoluteString)
        return true
    }

    func applicationDidEnterBackground(_ application: UIApplication) {
        nori_ios_background()
    }

    func applicationWillEnterForeground(_ application: UIApplication) {
        nori_ios_foreground()
        Core.shared.refresh()
    }

    func applicationDidReceiveMemoryWarning(_ application: UIApplication) {
        CoverCache.trim()
        nori_ios_memory_warning()
    }
}
