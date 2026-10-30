//! The lyrics services the settings switch and rank, and which ones a lookup asks ([`lyrics_lookup`]).
//! Querying them is nori-lyrics'.

use crate::codec::Choice;
use crate::settings::StoredPrefs;

/// Where lyrics came from, for the credit line: the server, a [`LyricsService`] or a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum LyricsOrigin {
    Server,
    Binilyrics,
    BetterLyrics,
    Paxsenix,
    LyricsPlus,
    PaxsenixMusixmatch,
    Unison,
    Netease,
    Kugou,
    Lrclib,
    PaxsenixSpotify,
    YoutubeMusic,
    /// A plugin's lyrics service, by the plugin's id.
    Plugin { id: String },
}

/// How finely lyrics are timed, worst to best.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Timing {
    Empty,
    Untimed,
    Lines,
    Words,
}

impl Timing {
    /// The words for it in the log.
    pub fn words(self) -> &'static str {
        match self {
            Timing::Words => "word-timed",
            Timing::Lines => "line-timed",
            Timing::Untimed => "not timed",
            Timing::Empty => "empty",
        }
    }
}

/// A third-party lyrics service, asked after the server's own lyrics. Declared in default rank order:
/// the first wave (asked at once), then the second (asked only when the first misses, scores low or
/// lacks word timing, nori-lyrics' race.rs), then those off out of the box: the keyed ones and the only
/// untimed one. Rankings and switches are stored by [`name`](Self::name), so this order can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, nori_settings_derive::Choice)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum LyricsService {
    LyricsPlus,
    BetterLyrics,
    Lrclib,
    Kugou,
    Netease,
    Binilyrics,
    Paxsenix,
    Unison,
    PaxsenixMusixmatch,
    PaxsenixSpotify,
    YoutubeMusic,
}

/// Per service, in declaration order: its name in log lines, the finest timing it can answer with,
/// whether a lookup's first wave asks it, and the prior trust in its answers, 0 to 1 (part of
/// nori-lyrics' trust.rs score).
const ABOUT: [(&str, Timing, bool, f64); 11] = [
    ("LyricsPlus", Timing::Words, true, 0.8),
    ("BetterLyrics", Timing::Words, true, 0.9),
    ("LRCLIB", Timing::Words, true, 0.85),
    ("KuGou", Timing::Words, false, 0.75),
    ("NetEase Cloud Music", Timing::Words, false, 0.75),
    ("BiniLyrics", Timing::Words, false, 0.9),
    ("PaxSenix", Timing::Words, false, 0.95),
    ("Unison", Timing::Words, false, 0.85),
    ("PaxSenix: Musixmatch", Timing::Words, false, 0.85),
    ("PaxSenix: Spotify", Timing::Words, false, 0.85),
    ("YouTube Music", Timing::Untimed, false, 0.6),
];

/// Every service's stored name in the order 0.6.2 shipped: a ranking or switch list stored as exactly
/// this was never changed by hand, so it follows the new default.
const DEFAULT_BEFORE_063: &str =
    "PAXSENIX,BINILYRICS,UNISON,BETTER_LYRICS,KUGOU,NETEASE,LYRICS_PLUS,SIMPMUSIC,PORTATO,PAXSENIX_MUSIXMATCH,LRCLIB,PAXSENIX_SPOTIFY,YOUTUBE_CAPTIONS,MEGALOBIZ,YOUTUBE_MUSIC,GENIUS";

impl LyricsService {
    /// Every service, in declaration order.
    pub const ALL: &'static [LyricsService] = <Self as Choice>::ALL;

    /// Its stored and cache name.
    pub fn name(self) -> &'static str {
        <Self as Choice>::NAMES[self as usize]
    }

    /// The service stored under `name`, in any case.
    pub fn named(name: &str) -> Option<LyricsService> {
        LyricsService::ALL.iter().copied().find(|s| s.name().eq_ignore_ascii_case(name.trim()))
    }

    pub fn title(self) -> &'static str {
        ABOUT[self as usize].0
    }

    pub fn best(self) -> Timing {
        ABOUT[self as usize].1
    }

    /// Needs the PaxSenix key; never on without it.
    pub(crate) fn needs_key(self) -> bool {
        matches!(self, LyricsService::PaxsenixMusixmatch | LyricsService::PaxsenixSpotify)
    }

    /// Takes a key, needed or not (BetterLyrics finds more with one).
    pub(crate) fn takes_key(self) -> bool {
        self.needs_key() || self == LyricsService::BetterLyrics
    }

    /// The key it takes, as stored.
    pub(crate) fn key(self, p: &StoredPrefs) -> &str {
        match self {
            LyricsService::BetterLyrics => p.better_lyrics_key.trim(),
            s if s.needs_key() => p.paxsenix_key.trim(),
            _ => "",
        }
    }

    pub fn first_wave(self) -> bool {
        ABOUT[self as usize].2
    }

    pub fn prior(self) -> f64 {
        ABOUT[self as usize].3
    }

    /// The credit line's name for it.
    pub fn origin(self) -> LyricsOrigin {
        match self {
            LyricsService::Binilyrics => LyricsOrigin::Binilyrics,
            LyricsService::BetterLyrics => LyricsOrigin::BetterLyrics,
            LyricsService::Paxsenix => LyricsOrigin::Paxsenix,
            LyricsService::LyricsPlus => LyricsOrigin::LyricsPlus,
            LyricsService::PaxsenixMusixmatch => LyricsOrigin::PaxsenixMusixmatch,
            LyricsService::Unison => LyricsOrigin::Unison,
            LyricsService::Netease => LyricsOrigin::Netease,
            LyricsService::Kugou => LyricsOrigin::Kugou,
            LyricsService::Lrclib => LyricsOrigin::Lrclib,
            LyricsService::PaxsenixSpotify => LyricsOrigin::PaxsenixSpotify,
            LyricsService::YoutubeMusic => LyricsOrigin::YoutubeMusic,
        }
    }
}

/// Every service in default rank order.
pub fn default_order() -> Vec<LyricsService> {
    LyricsService::ALL.to_vec()
}

/// The services switched on out of the box: all but the keyed ones and the untimed one.
pub fn default_on() -> Vec<LyricsService> {
    LyricsService::ALL.iter().copied().filter(|s| !s.needs_key() && s.best() != Timing::Untimed).collect()
}

/// A stored ranking: `default` when it is the default 0.6.2 shipped, else as stored, without the
/// services since removed.
pub(crate) fn stored_or(text: &str, default: Vec<LyricsService>) -> Vec<LyricsService> {
    if text.trim() == DEFAULT_BEFORE_063 { default } else { parse(text) }
}

/// A ranking completed: duplicates removed, and services missing from it inserted after their default
/// predecessor.
pub(crate) fn complete_order(stored: &[LyricsService]) -> Vec<LyricsService> {
    let mut order = distinct(stored.iter().copied());
    for (i, &s) in LyricsService::ALL.iter().enumerate() {
        if !order.contains(&s) {
            let above = LyricsService::ALL[..i].iter().rev().find_map(|a| order.iter().position(|o| o == a));
            order.insert(above.map_or(0, |at| at + 1), s);
        }
    }
    order
}

/// The services a stored list names (`LRCLIB,KUGOU`), each once, in order; unknown names dropped.
pub fn parse(names: &str) -> Vec<LyricsService> {
    distinct(names.split(',').filter_map(LyricsService::named))
}

/// The stored form of a list: names joined by commas.
pub(crate) fn to_names(services: &[LyricsService]) -> String {
    services.iter().map(|s| s.name()).collect::<Vec<_>>().join(",")
}

fn distinct(services: impl Iterator<Item = LyricsService>) -> Vec<LyricsService> {
    let mut out = Vec::new();
    for s in services {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Whether `service` may be switched on: one that needs a key only once the key is given.
pub(crate) fn can_be_on(p: &StoredPrefs, service: LyricsService) -> bool {
    !service.needs_key() || !p.paxsenix_key.trim().is_empty()
}

/// The services switched on, in rank order.
pub(crate) fn switched_on(p: &StoredPrefs) -> Vec<LyricsService> {
    p.lyrics_order.iter().copied().filter(|s| p.lyrics_on.contains(s)).collect()
}

/// The ranking with `service` moved `by` places (-1 up), past switched-off neighbours too.
pub fn moved(p: &StoredPrefs, service: LyricsService, by: i32) -> Vec<LyricsService> {
    let Some(at) = p.lyrics_order.iter().position(|s| *s == service) else { return p.lyrics_order.clone() };
    placed(p, service, (at as i64 + by as i64).max(0) as usize)
}

/// The ranking with `service` moved to place `to` (past the end is last).
pub fn placed(p: &StoredPrefs, service: LyricsService, to: usize) -> Vec<LyricsService> {
    let mut order = complete_order(&p.lyrics_order);
    order.retain(|s| *s != service);
    order.insert(to.min(order.len()), service);
    order
}

/// What one lookup asks: the services in rank order, whether plugins' services too, and the options
/// and keys.
#[derive(Debug, Clone, PartialEq)]
pub struct LyricsLookup {
    pub services: Vec<LyricsService>,
    /// The enabled plugins' services are asked after nori's (under the same switches).
    pub plugins: bool,
    pub prefer_words: bool,
    pub paxsenix_key: String,
    pub better_lyrics_key: String,
}

impl LyricsLookup {
    /// Only service `name` (a jam host's choice), when this lookup asks anyone and that service can be
    /// asked here; else this lookup as it is.
    pub fn only(mut self, name: &str) -> LyricsLookup {
        let usable = |s: &LyricsService| !s.needs_key() || !self.paxsenix_key.is_empty();
        if let Some(s) = LyricsService::named(name).filter(|s| !self.services.is_empty() && usable(s)) {
            self.services = vec![s];
            self.plugins = false;
        }
        self
    }
}

/// The lookup the settings allow: none unless both the lookups and online lyrics switches are on, and
/// no service whose key is missing.
pub fn lyrics_lookup(p: &StoredPrefs) -> LyricsLookup {
    let paxsenix_key = p.paxsenix_key.trim().to_string();
    let better_lyrics_key = p.better_lyrics_key.trim().to_string();
    let services = if p.third_party_lookups && p.lyrics_online { switched_on(p).into_iter().filter(|s| !s.needs_key() || !paxsenix_key.is_empty()).collect() } else { Vec::new() };
    LyricsLookup { services, plugins: p.third_party_lookups && p.lyrics_online, prefer_words: p.lyrics_prefer_words, paxsenix_key, better_lyrics_key }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(v: &[&str]) -> Vec<LyricsService> {
        v.iter().map(|s| LyricsService::named(s).unwrap()).collect()
    }

    #[test]
    fn defaults() {
        let on = StoredPrefs::default();
        assert!(lyrics_lookup(&StoredPrefs { third_party_lookups: false, ..on.clone() }).services.is_empty(), "off under the lookups switch");
        let first: Vec<LyricsService> = lyrics_lookup(&on).services.into_iter().filter(|s| s.first_wave()).collect();
        assert_eq!(first, [LyricsService::LyricsPlus, LyricsService::BetterLyrics, LyricsService::Lrclib], "the first wave");
        let asked = lyrics_lookup(&on).services;
        assert_eq!(asked[3..], named(&["KUGOU", "NETEASE", "BINILYRICS", "PAXSENIX", "UNISON"]), "then the second");
        assert!(!asked.iter().any(|s| s.needs_key() || *s == LyricsService::YoutubeMusic), "keyed and untimed services are off out of the box");
        let off = StoredPrefs { lyrics_online: false, ..on };
        assert!(lyrics_lookup(&off).services.is_empty());
        assert!(LyricsService::ALL.iter().all(|s| (0.0..=1.0).contains(&s.prior())));

        // Names round trip.
        for &s in LyricsService::ALL {
            assert_eq!(LyricsService::named(s.name()), Some(s));
            assert_eq!(LyricsService::named(&s.name().to_lowercase()), Some(s));
            assert!(!s.title().is_empty());
        }
        assert_eq!(LyricsService::named("MUSIXMATCH"), None, "a test build's service is not read");
    }

    #[test]
    fn a_jam_hosts_service_is_asked_alone_where_it_can_be() {
        let on = lyrics_lookup(&StoredPrefs::default());
        assert_eq!(on.clone().only("UNISON").services, [LyricsService::Unison]);
        assert!(on.plugins && !on.clone().only("UNISON").plugins, "a jam's choice alone, no plugin's");
        assert!(on.clone().only("plugin:fm.nori.test").plugins, "a plugin's choice: this member's plugins");
        assert_eq!(on.clone().only("PAXSENIX_SPOTIFY").services, on.services, "no key here for it");
        assert_eq!(on.clone().only("NOPE").services, on.services);
        let off = lyrics_lookup(&StoredPrefs { lyrics_online: false, ..Default::default() });
        assert!(off.only("UNISON").services.is_empty(), "lookups stay off");
    }

    #[test]
    fn removed_services_load_cleanly() {
        // A ranking and switch list stored by hand before 0.6.3: the removed services go, the rest keep
        // their order; the new services' places are their defaults'.
        let raw = |order: &str, on: &str| {
            let mut m = std::collections::HashMap::new();
            m.insert("lyricsOrder".to_string(), crate::settings::PrefValue::Text { v: order.into() });
            m.insert("lyricsOn".to_string(), crate::settings::PrefValue::Text { v: on.into() });
            crate::settings::load(&m)
        };
        let p = raw("GENIUS,LRCLIB,SIMPMUSIC,KUGOU,PORTATO,MEGALOBIZ,YOUTUBE_CAPTIONS,PAXSENIX", "GENIUS,LRCLIB,SIMPMUSIC");
        let at = |n: &str| p.lyrics_order.iter().position(|s| s.name() == n).unwrap();
        assert_eq!(p.lyrics_order.len(), LyricsService::ALL.len());
        assert!(at("LRCLIB") < at("KUGOU") && at("KUGOU") < at("PAXSENIX"), "the order kept: {:?}", p.lyrics_order);
        assert_eq!(p.lyrics_on, [LyricsService::Lrclib], "only the ones still there");
        // Stored as 0.6.2 shipped them: the new default.
        let old = raw(DEFAULT_BEFORE_063, DEFAULT_BEFORE_063);
        assert_eq!((old.lyrics_order, old.lyrics_on), (default_order(), default_on()));
    }

    #[test]
    fn a_key_is_given_where_its_service_is_switched_on() {
        use crate::settings::set_by_name;
        let p = StoredPrefs::default();
        assert!(set_by_name(&p, "lyricsService:PAXSENIX_SPOTIFY", "true").is_none(), "not on without its key");
        assert!(set_by_name(&p, "lyricsKey:LRCLIB", "k").is_none(), "LRCLIB takes no key");
        let keyed = set_by_name(&p, "lyricsKey:PAXSENIX_SPOTIFY", " k1 ").unwrap().prefs;
        assert_eq!(keyed.paxsenix_key, "k1");
        assert!(keyed.lyrics_on.contains(&LyricsService::PaxsenixSpotify) && !keyed.lyrics_on.contains(&LyricsService::PaxsenixMusixmatch), "the one it was given for, on");
        let both = set_by_name(&keyed, "lyricsService:PAXSENIX_MUSIXMATCH", "true").unwrap().prefs;
        assert!(both.lyrics_on.contains(&LyricsService::PaxsenixMusixmatch), "the same key serves the other");
        let changed = set_by_name(&both, "lyricsKey:PAXSENIX_MUSIXMATCH", "k2").unwrap().prefs;
        assert_eq!((changed.paxsenix_key.as_str(), changed.lyrics_on.len()), ("k2", both.lyrics_on.len()), "a new key, nothing else moved");
        let cleared = set_by_name(&changed, "lyricsKey:PAXSENIX_SPOTIFY", "").unwrap().prefs;
        assert!(cleared.paxsenix_key.is_empty() && !cleared.lyrics_on.iter().any(|s| s.needs_key()), "cleared: both off");
        assert!(set_by_name(&cleared, "lyricsService:PAXSENIX_SPOTIFY", "false").is_some(), "off is always taken");
        let state = crate::settings_model::state(&changed, crate::settings_model::Output::default(), &nori_automix::beat_model::ModelFile::default());
        let spotify = state.lyrics_sources.iter().find(|s| s.id == "PAXSENIX_SPOTIFY").unwrap();
        assert!(spotify.needs_key && spotify.key_set);
        assert!(!state.values.contains_key("paxSenixKey"), "the key is never a value shown or reported");
    }

    #[test]
    fn better_lyrics_takes_its_key_on_its_row_and_plays_without_one() {
        use crate::settings::set_by_name;
        let off = StoredPrefs { lyrics_on: Vec::new(), ..StoredPrefs::default() };
        let keyed = set_by_name(&off, "lyricsKey:BETTER_LYRICS", " b1 ").unwrap().prefs;
        assert_eq!((keyed.better_lyrics_key.as_str(), keyed.paxsenix_key.as_str()), ("b1", ""), "its own key, not PaxSenix's");
        assert_eq!(keyed.lyrics_on, [LyricsService::BetterLyrics], "on once given");
        let cleared = set_by_name(&keyed, "lyricsKey:BETTER_LYRICS", "").unwrap().prefs;
        assert!(cleared.better_lyrics_key.is_empty() && cleared.lyrics_on == [LyricsService::BetterLyrics], "no key, still on");
        let state = |p: &StoredPrefs| crate::settings_model::state(p, crate::settings_model::Output::default(), &nori_automix::beat_model::ModelFile::default());
        let row = |p: &StoredPrefs| state(p).lyrics_sources.into_iter().find(|s| s.id == "BETTER_LYRICS").unwrap();
        assert!(!row(&keyed).needs_key && row(&keyed).key_set);
        assert!(!row(&cleared).key_set);
        assert!(!state(&keyed).values.contains_key("betterLyricsKey"), "the key is never a value shown or reported");
    }

    #[test]
    fn keyed_service_waits_for_key() {
        let p = StoredPrefs { third_party_lookups: true, lyrics_on: named(&["PAXSENIX_SPOTIFY", "LRCLIB"]), ..StoredPrefs::default() };
        assert_eq!(lyrics_lookup(&p).services, [LyricsService::Lrclib]);
        let keyed = StoredPrefs { paxsenix_key: " abc ".into(), ..p };
        let l = lyrics_lookup(&keyed);
        assert_eq!(l.services, [LyricsService::Lrclib, LyricsService::PaxsenixSpotify]);
        assert_eq!(l.paxsenix_key, "abc");
    }

    #[test]
    fn ordering() {
        let n = LyricsService::ALL.len();
        let order = complete_order(&parse("UNISON, KUGOU,MUSIXMATCH,,UNISON"));
        assert_eq!(order.len(), n);
        assert_eq!(order[..3], named(&["LYRICS_PLUS", "BETTER_LYRICS", "LRCLIB"]), "the ones ranked above everything stored come first");
        let at = |n: &str| order.iter().position(|o| o.name() == n).unwrap();
        assert!(at("UNISON") < at("KUGOU"), "the stored order stands");
        assert_eq!(at("PAXSENIX_MUSIXMATCH"), at("UNISON") + 1, "put in after the one above it out of the box");
        assert_eq!(complete_order(&[]), default_order());

        // Move passes switched off services.
        let p = StoredPrefs { lyrics_on: named(&["NETEASE", "PAXSENIX", "YOUTUBE_MUSIC"]), ..StoredPrefs::default() };
        let at = |o: &[LyricsService], n: &str| o.iter().position(|x| x.name() == n).unwrap();
        let order = moved(&p, LyricsService::Paxsenix, -1);
        assert_eq!(at(&order, "PAXSENIX"), at(&p.lyrics_order, "PAXSENIX") - 1, "one place, past a service that is off");
        assert_eq!(at(&order, "BINILYRICS"), at(&p.lyrics_order, "PAXSENIX"));
        let first = StoredPrefs { lyrics_order: placed(&p, LyricsService::Paxsenix, 0), ..p.clone() };
        assert_eq!(first.lyrics_order[0], LyricsService::Paxsenix);
        assert_eq!(moved(&first, LyricsService::Paxsenix, -1), first.lyrics_order, "the first stays first");
        assert_eq!(switched_on(&first), [LyricsService::Paxsenix, LyricsService::Netease, LyricsService::YoutubeMusic]);
        let off = moved(&p, LyricsService::Kugou, 1);
        assert_eq!(at(&off, "KUGOU"), at(&p.lyrics_order, "KUGOU") + 1, "one switched off moves too");

        // Place moves one service.
        let p = StoredPrefs::default();
        let last = placed(&p, LyricsService::LyricsPlus, 99);
        assert_eq!(last.len(), n);
        assert_eq!(last[n - 1], LyricsService::LyricsPlus);
        assert_eq!(last[..n - 1], p.lyrics_order[1..]);
        let back = StoredPrefs { lyrics_order: last, ..p.clone() };
        assert_eq!(placed(&back, LyricsService::LyricsPlus, 0), p.lyrics_order);
        let mid = placed(&p, LyricsService::YoutubeMusic, 3);
        assert_eq!(mid[3], LyricsService::YoutubeMusic);
        assert_eq!(mid.iter().filter(|n| **n == LyricsService::YoutubeMusic).count(), 1);
    }
}
