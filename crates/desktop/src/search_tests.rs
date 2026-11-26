//! The search page in the testing backend, filled with the local fixture server's answers
//! (tools/dev-server.sh); with `NORI_DESKTOP_SHOTS` set, each state is saved there as a PNG with the top
//! pill laid over it as the compositor lays it.

use super::*;

/// What the fixture server answers `query` with, as its search3 gives it.
pub(super) fn fixture(query: &str) -> nori_core::SearchResult {
    use nori_core::{Album, Artist, SearchResult};
    let artists = [("ar-alpha", "Alpha Waves"), ("ar-beta", "Beta Band"), ("ar-gamma", "Gamma Ray Trio"), ("ar-sessions", "Nori Sessions"), ("ar-e2e", "Nori E2E"), ("ar-lyrics", "Nori Lyrics")];
    let artist_id = |name: &str| artists.iter().find(|a| a.1 == name).map(|a| a.0.to_string());
    let album = |id: &str, name: &str, artist: &str, year: u32| Album {
        id: id.into(),
        name: name.into(),
        artist: artist.into(),
        artist_id: artist_id(artist),
        cover_art: Some(format!("al-{id}")),
        year,
        song_count: 4,
        ..Default::default()
    };
    let song = |id: &str, title: &str, artist: &str, album: &Album, duration: u32| Song {
        id: id.into(),
        title: title.into(),
        artist: artist.into(),
        artist_id: artist_id(artist),
        album: album.name.clone(),
        album_id: Some(album.id.clone()),
        cover_art: album.cover_art.clone(),
        duration,
        year: album.year,
        ..Default::default()
    };
    let first = |artist: &str, id: &str, year| album(id, "First Light", artist, year);
    let (alpha1, alpha2) = (first("Alpha Waves", "alpha-1", 2004), album("alpha-2", "Second Wind", "Alpha Waves", 2008));
    let (beta1, beta2) = (first("Beta Band", "beta-1", 2012), album("beta-2", "Second Wind", "Beta Band", 2016));
    let (gamma1, gamma2) = (first("Gamma Ray Trio", "gamma-1", 2020), album("gamma-2", "Second Wind", "Gamma Ray Trio", 2024));
    let sessions = first("Nori Sessions", "sessions", 2026);
    let long = album("long", "Long Album", "Nori E2E", 2024);
    let sung = album("sung", "Sung Album", "Nori Lyrics", 2024);
    let tracks = |a: &Album, artist: &str, from: u32| (1..=4).map(move |t| song(&format!("{}-{t}", a.id), &format!("Track {t} of {}", a.name), artist, a, from + t * 3)).collect::<Vec<_>>();
    let interleave = |a: Vec<Song>, b: Vec<Song>| a.into_iter().zip(b).flat_map(|(x, y)| [x, y]).collect::<Vec<_>>();
    match query {
        "alpha" => SearchResult {
            artists: vec![Artist { id: "ar-alpha".into(), name: "Alpha Waves".into(), album_count: Some(3), cover_art: Some("ar-alpha".into()), ..Default::default() }],
            albums: vec![alpha1.clone(), alpha2.clone()],
            songs: interleave(tracks(&alpha1, "Alpha Waves", 40), tracks(&alpha2, "Alpha Waves", 52)),
            playlists: Vec::new(),
        },
        "first" => SearchResult {
            artists: Vec::new(),
            albums: vec![beta1.clone(), alpha1.clone(), sessions.clone(), gamma1.clone()],
            songs: interleave(tracks(&beta1, "Beta Band", 64), tracks(&alpha1, "Alpha Waves", 40))
                .into_iter()
                .chain(tracks(&gamma1, "Gamma Ray Trio", 88))
                .chain(["Something New", "Hold the Line", "Morning Comes Again", "Somewhere a Light"].iter().enumerate().map(|(i, t)| song(&format!("sessions-{i}"), t, "Nori Sessions", &sessions, 120)))
                .collect(),
            playlists: Vec::new(),
        },
        "track" => SearchResult {
            songs: std::iter::once(song("sung-1", "Sung Track", "Nori Lyrics", &sung, 120))
                .chain((1..=12).map(|t| song(&format!("long-{t}"), &format!("Long Track {t:02}"), "Nori E2E", &long, 75 + t * 5)))
                .chain(interleave(tracks(&beta1, "Beta Band", 64), tracks(&alpha1, "Alpha Waves", 40)))
                .chain(interleave(tracks(&beta2, "Beta Band", 76), tracks(&alpha2, "Alpha Waves", 52)))
                .chain(interleave(tracks(&gamma1, "Gamma Ray Trio", 88), tracks(&gamma2, "Gamma Ray Trio", 100)))
                .collect(),
            ..Default::default()
        },
        _ => SearchResult::default(),
    }
}

/// A cover as the fixture's are: one colour, picked from its id.
fn fixture_art(id: &str) -> Image {
    let h = id.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let [r, g, b, _] = h.to_le_bytes();
    let mut px = SharedPixelBuffer::<Rgba8Pixel>::new(8, 8);
    px.make_mut_slice().fill(Rgba8Pixel { r: r / 2 + 60, g: g / 2 + 60, b: b / 2 + 60, a: 255 });
    Image::from_rgba8(px)
}

pub(super) struct Rig {
    pub ui: AppWindow,
    pub top: crate::TopBar,
    pub width: f32,
}

/// Day or Night, autumn, as the app paints them.
fn look(night: bool) -> crate::look::Look {
    let prefs = nori_core::settings::StoredPrefs { theme: if night { nori_core::settings::ThemeMode::Dark } else { nori_core::settings::ThemeMode::Light }, ..Default::default() };
    crate::look::plain(&Default::default(), &prefs, night, crate::look::Today { month: 10, day: 19, southern: false })
}

/// A main window and a top pill `width` by `height`, laid out as the compositor lays them.
pub(super) fn rig(width: f32, height: f32, night: bool) -> Rig {
    let _ = slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
        i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
    )));
    let ui = AppWindow::new().unwrap();
    let top = crate::TopBar::new().unwrap();
    let look = look(night);
    look.paint(&ui);
    look.paint(&top);
    ui.window().set_size(slint::LogicalSize::new(width, height));
    let top_w = (width - 2.0 * 84.0).clamp(200.0, 1040.0);
    let chrome = ui.global::<crate::Chrome>();
    chrome.set_top_x((width - top_w) / 2.0);
    chrome.set_top_w(top_w);
    top.window().set_size(slint::LogicalSize::new(top_w, 48.0));
    top.set_room(top_w);
    top.set_view(View::Search);
    ui.on_art(|id, _, _| fixture_art(&id));
    ui.set_view(View::Search);
    ui.show().unwrap();
    top.show().unwrap();
    Rig { ui, top, width }
}

impl Rig {
    /// Lets the frame settle: what was set is laid out, animations ended.
    pub fn settle(&self) {
        for _ in 0..3 {
            self.ui.window().take_snapshot().unwrap();
            i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(500));
            slint::platform::update_timers_and_animations();
        }
    }

    /// The window now, the top pill over it, saved as `name`.png under `NORI_DESKTOP_SHOTS`.
    pub fn shot(&self, name: &str) {
        self.settle();
        let Some(dir) = std::env::var_os("NORI_DESKTOP_SHOTS") else { return };
        let page = self.ui.window().take_snapshot().unwrap();
        let pill = self.top.window().take_snapshot().unwrap();
        let (w, h) = (page.width() as usize, page.height() as usize);
        let mut px = page.as_bytes().to_vec();
        let x0 = ((self.width - self.top.window().size().width as f32) / 2.0) as usize;
        let (pw, ph) = (pill.width() as usize, pill.height() as usize);
        for y in 0..ph {
            for x in 0..pw {
                let s = &pill.as_bytes()[(y * pw + x) * 4..][..4];
                let d = &mut px[((y + 12) * w + x0 + x) * 4..][..4];
                let a = s[3] as u32;
                for c in 0..3 {
                    // Premultiplied, as Skia draws.
                    d[c] = (s[c] as u32 + d[c] as u32 * (255 - a) / 255).min(255) as u8;
                }
            }
        }
        let file = std::fs::File::create(std::path::Path::new(&dir).join(format!("{name}.png"))).unwrap();
        let mut png = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        png.set_color(png::ColorType::Rgba);
        png.write_header().unwrap().write_image_data(&px).unwrap();
    }
}

/// The view of `query` answered with the fixture's answer, with `kind` picked.
fn answered(query: &str, kind: Option<nori_core::search::SearchKind>) -> SearchView {
    use nori_core::search::{search_split, Answer, Session};
    let mut s = Session { text: query.into(), answer: Answer::Server(search_split(fixture(query), query)), ..Default::default() };
    s.pick(kind);
    s.view()
}

/// Shows `v` as the app does, its songs as plain rows.
fn show(r: &Rig, v: SearchView) -> SearchPage {
    r.top.set_search_text(v.text.as_str().into());
    let page = search_shown(&r.ui, v);
    r.ui.set_search_songs(ModelRc::new(VecModel::from(page.songs.iter().enumerate().map(|(i, s)| row(s, i, false)).collect::<Vec<_>>())));
    page
}

fn titles(m: ModelRc<Card>) -> Vec<String> {
    m.iter().map(|c| c.title.to_string()).collect()
}

#[test]
fn the_page_draws_the_cores_layout_and_a_kind_picked_whole() {
    use nori_core::search::SearchKind;
    let r = rig(1280.0, 800.0, true);
    let ui = &r.ui;
    show(&r, answered("alpha", None));
    r.settle();
    assert_eq!(ui.get_search_chips().iter().map(|c| c.to_string()).collect::<Vec<_>>(), ["All", "Songs", "Albums", "Artists"], "the kinds found, in their order");
    assert_eq!((ui.get_search_chip(), ui.get_search_picked()), (0, -1));
    assert!(ui.get_search_has_top());
    assert_eq!((ui.get_search_top().title.to_string(), ui.get_search_top_kind().to_string()), ("Alpha Waves".to_string(), "ARTIST".to_string()));
    assert_eq!(ui.get_search_songs().row_count(), 4, "the best few songs beside it");
    assert!(ui.get_search_artists().row_count() == 0, "the artist is not listed again under itself");

    let page = show(&r, answered("first", Some(SearchKind::Albums)));
    r.settle();
    assert_eq!((ui.get_search_chip(), ui.get_search_picked(), ui.get_search_has_top()), (2, 1, false));
    assert_eq!(titles(ui.get_search_albums()).len(), 4, "every album, the top result's own among them");
    assert_eq!(page.songs.len(), 0);

    show(&r, answered("zzzz", None));
    r.settle();
    assert!(ui.get_search_nothing());
    assert_eq!(ui.get_search_chips().row_count(), 0, "nothing to narrow");
}

#[test]
fn the_search_pages_states_drawn() {
    use nori_core::search::SearchKind;
    for (w, h) in [(1600.0, 1000.0), (1280.0, 800.0)] {
        for night in [false, true] {
            let r = rig(w, h, night);
            let name = |what: &str| format!("desktop-{}x{}-{}-{what}", w as u32, h as u32, if night { "night" } else { "day" });
            for q in ["alpha", "first", "track", "zzzz"] {
                show(&r, answered(q, None));
                r.shot(&name(q));
                // The top result and the songs stand where they are when the server's answer replaces the
                // index's, one kind picked and All again.
                if q == "first" {
                    let before = r.ui.window().take_snapshot().unwrap();
                    show(&r, answered(q, Some(SearchKind::Albums)));
                    r.shot(&name("first-albums"));
                    show(&r, answered(q, Some(SearchKind::Songs)));
                    r.shot(&name("first-songs"));
                    show(&r, answered(q, None));
                    r.settle();
                    assert!(r.ui.window().take_snapshot().unwrap().as_bytes() == before.as_bytes(), "All again is drawn as it was");
                }
            }
        }
    }
}
