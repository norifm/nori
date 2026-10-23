//! Window state and UI callbacks, on Slint's event loop thread. Worker results arrive as [`Msg`]s through
//! the inbox ([`session::Tx`]). The only repeating timer is the seek bar's, and only while playing.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use nori_core::browse::AlbumSort;
use nori_core::menus::PageKind;
use nori_core::search::SearchView;
use nori_core::settings::SavedServer;
use nori_core::Song;
use nori_covers::loader::Ticket;
use nori_covers::memory::Image as Picture;
use nori_engine::{Event, State};
use nori_host::remote::Elsewhere;
use nori_http::Http;
use nori_look::cover::CoverColours;
use skia_safe::Typeface;
use slint::{Color, ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode, VecModel};

use crate::compositor::{Compositor, Focus};
use crate::session::{self, CoverKey, CoverSize, Data, Fetch, Msg, Req, Session, Tx};
use nori_host::pages::{HomePart, PageAsk, PageGot, ShelfItems};
use nori_host::player::{Playback, QueueShown};
use crate::settings::{Act, Target};
use crate::words;
use crate::{AppWindow, ArtistLink, Card, Credits, Detail, LoginKind, LyricPiece, Motion, Pick, PlayerBar, Shelf, SongGo, SongRow, Theme, TopBar};

/// Cover fetch sizes, px square.
const SMALL_PX: u32 = 256;
const LARGE_PX: u32 = 800;
const HERO_PX: u32 = 1600;
/// Bytes of decoded covers kept per class.
const SMALL_BYTES: usize = 32 * 1024 * 1024;
const LARGE_BYTES: usize = 24 * 1024 * 1024;
/// Pending cover requests kept; the oldest beyond this are cancelled.
const PENDING_KEPT: usize = 1000;
/// A pending cover not looked up for this long is off screen, and its request is cancelled.
const GONE: Duration = Duration::from_millis(1500);
/// How long a row leaving the queue takes to fold away (app.slint's QueueView).
const QUEUE_FOLD_MS: u64 = 180;
/// How long the undo of a song taken out of the queue is offered.
const UNDO_MS: u64 = 5_000;

// View indices, as app.slint numbers them.
const HOME: i32 = 0;
const SEARCH: i32 = 1;
const ALBUMS: i32 = 2;
const ARTISTS: i32 = 3;
const PLAYLISTS: i32 = 4;
const SONGS: i32 = 5;
const PAGE: i32 = 6;
const LOGIN: i32 = 7;
const SETTINGS: i32 = 8;
const EQUALIZER: i32 = 9;
const FAVORITES: i32 = 12;

/// The way back from the page shown: the pages opened from one another before it, and the tab the first
/// of them was opened from.
#[derive(Default)]
struct PageTrail {
    /// The page shown, and those before it, oldest first.
    shown: Option<PageAsk>,
    before: Vec<PageAsk>,
    tab: i32,
}

/// Where Back leads.
#[derive(Debug, PartialEq)]
enum Back {
    Page(PageAsk),
    Tab(i32),
}

impl PageTrail {
    /// `ask` is shown, opened from tab `from` (or from the page shown, when `from` is the page view).
    fn opened(&mut self, ask: PageAsk, from: i32) {
        if from == PAGE {
            self.before.extend(self.shown.take());
        } else {
            self.before.clear();
            self.tab = from;
        }
        self.shown = Some(ask);
    }

    /// Back from the page shown: the page before it (now shown), else the tab it was opened from.
    fn back(&mut self) -> Back {
        match self.before.pop() {
            Some(page) => {
                self.shown = Some(page.clone());
                Back::Page(page)
            }
            None => {
                self.shown = None;
                Back::Tab(self.tab)
            }
        }
    }
}

/// The view Back returns to from a page opened on `view`. Search keeps its field and answer while a page
/// is open, so Back finds them as they were.
fn opened_from(view: i32) -> i32 {
    if view == LOGIN { HOME } else { view }
}

/// Weak handle to the app for callbacks and timers.
#[derive(Clone)]
struct AppHandle(Weak<RefCell<App>>);

impl AppHandle {
    /// Runs `f` on the app; dropped if the app is gone or already borrowed (a re-entrant callback, named
    /// by where it is wired).
    #[track_caller]
    fn with(&self, f: impl FnOnce(&mut App)) {
        let Some(app) = self.0.upgrade() else { return };
        match app.try_borrow_mut() {
            Ok(mut a) => f(&mut a),
            Err(_) => eprintln!("nori: a callback came while the window's state was busy ({}); dropped", std::panic::Location::caller()),
        };
    }
}

/// Wires a Slint callback to an [`App`] method: `on!(ui.on_go, h, |a, v| a.go(v))`.
macro_rules! on {
    ($ui:ident.$setter:ident, $h:expr, |$a:ident $(, $arg:ident)*| $body:expr) => {{
        let h = $h.clone();
        $ui.$setter(move |$($arg),*| h.with(|$a| $body));
    }};
}

/// Library lists as loaded, before the find filter.
#[derive(Default)]
struct Lists {
    albums: Vec<nori_core::Album>,
    artists: Vec<Card>,
    playlists: Vec<Card>,
}

/// Decoded covers, looked up by the `art` callback while drawing. A missing cover is requested once;
/// its arrival bumps `covers-rev` so pictures look again. Least recently drawn are evicted first.
#[derive(Default)]
struct Art {
    images: HashMap<CoverKey, (Image, Instant)>,
    mosaics: HashMap<Vec<String>, Mosaic>,
    /// Page colours of large covers, by cover id.
    colours: HashMap<String, Rc<CoverColours>>,
    asked: HashSet<CoverKey>,
    wanted: Vec<CoverKey>,
    /// When a pending cover was last looked up; stale ones are off screen.
    missing: HashMap<CoverKey, Instant>,
}

enum Mosaic {
    Ready(Image),
    Updating(Image),
}

impl Mosaic {
    fn image(&self) -> &Image {
        match self { Self::Ready(image) | Self::Updating(image) => image }
    }
}

impl Art {
    fn insert(&mut self, key: CoverKey, image: Image, colours: Option<Box<CoverColours>>, now: Instant) {
        let id = key.id.clone();
        let large = key.size != CoverSize::Card;
        self.missing.remove(&key);
        if !large {
            for (covers, mosaic) in &mut self.mosaics {
                if covers.contains(&id) {
                    *mosaic = Mosaic::Updating(mosaic.image().clone());
                }
            }
        }
        self.images.insert(key.clone(), (image, now));
        if let Some(c) = colours {
            self.colours.insert(id.clone(), Rc::from(c));
        }
        let limit = if large { LARGE_BYTES } else { SMALL_BYTES };
        let class = |k: &CoverKey| (k.size != CoverSize::Card) == large;
        let bytes = |image: &Image| { let size = image.size(); size.width as usize * size.height as usize * 4 };
        let mut kept: usize = self.images.iter().filter(|(k, _)| class(k)).map(|(_, (image, _))| bytes(image)).sum();
        while kept > limit {
            let oldest = self
                .images
                .iter()
                .filter(|(k, _)| class(k))
                .min_by_key(|(_, (_, drawn))| *drawn)
                .map(|(k, _)| k.clone());
            let Some(old) = oldest else { break };
            let (image, _) = self.images.remove(&old).expect("the oldest cover");
            kept -= bytes(&image);
            self.asked.remove(&old);
            if large && !self.images.keys().any(|k| k.id == old.id && k.size != CoverSize::Card) {
                self.colours.remove(&old.id);
            }
        }
    }
}

/// app.slint's size code: 0 card, 1 large, 2 hero.
fn cover_size(size: i32) -> CoverSize {
    match size.clamp(0, 2) {
        0 => CoverSize::Card,
        1 => CoverSize::Large,
        _ => CoverSize::Hero,
    }
}

fn cover_px(size: CoverSize) -> u32 {
    match size {
        CoverSize::Card => SMALL_PX,
        CoverSize::Large => LARGE_PX,
        CoverSize::Hero => HERO_PX,
    }
}

/// The `art` callback: the cover if decoded, else an empty image and a queued request.
fn cover_image(art: &RefCell<Art>, app: &AppHandle, id: SharedString, size: i32) -> Image {
    if id.is_empty() {
        return Image::default();
    }
    let k = CoverKey { id: id.to_string(), size: cover_size(size) };
    let mut a = art.borrow_mut();
    if let Some((i, drawn)) = a.images.get_mut(&k) {
        *drawn = Instant::now();
        return i.clone();
    }
    a.missing.insert(k.clone(), Instant::now());
    if a.asked.insert(k.clone()) {
        if a.wanted.is_empty() {
            // Requested after this frame is drawn, not from inside it.
            let app = app.clone();
            Timer::single_shot(Duration::ZERO, move || app.with(App::ask_covers));
        }
        a.wanted.push(k);
    }
    Image::default()
}

fn pick_image(art: &RefCell<Art>, app: &AppHandle, covers: ModelRc<SharedString>, compose: impl FnOnce(&[Image]) -> Option<Image>) -> Image {
    let ids: Vec<String> = covers.iter().take(4).map(|id| id.to_string()).collect();
    if ids.len() < 4 {
        return ids.first().map_or_else(Image::default, |id| cover_image(art, app, id.as_str().into(), 0));
    }
    {
        let mut cache = art.borrow_mut();
        if let Some(Mosaic::Ready(image)) = cache.mosaics.get(&ids) {
            let image = image.clone();
            let now = Instant::now();
            for id in &ids {
                if let Some((_, drawn)) = cache.images.get_mut(&CoverKey { id: id.clone(), size: CoverSize::Card }) {
                    *drawn = now;
                }
            }
            return image;
        }
    }
    let images: Vec<_> = ids.iter().map(|id| cover_image(art, app, id.as_str().into(), 0)).collect();
    let Some(image) = compose(&images) else {
        return art.borrow().mosaics.get(&ids).map_or_else(Image::default, |mosaic| mosaic.image().clone());
    };
    art.borrow_mut().mosaics.insert(ids, Mosaic::Ready(image.clone()));
    image
}

#[cfg(any(test, target_os = "linux"))]
pub(crate) fn mosaic(images: &[Image]) -> Option<Image> {
    use skia_safe::{AlphaType, ColorType, Data, FilterMode, ImageInfo, Paint, Rect, SamplingOptions};
    let side = SMALL_PX;
    let width = side * 2;
    let mut surface = skia_safe::surfaces::raster_n32_premul((width as i32, width as i32))?;
    let paint = Paint::default();
    for (i, image) in images.iter().enumerate() {
        let image = if let Some(image) = i_slint_renderer_skia::image_to_skia(image) {
            image
        } else {
            let pixels = image.to_rgba8()?;
            let (w, h) = (pixels.width(), pixels.height());
            let info = ImageInfo::new((w as i32, h as i32), ColorType::RGBA8888, AlphaType::Unpremul, None);
            skia_safe::images::raster_from_data(&info, Data::new_copy(pixels.as_bytes()), w as usize * 4)?
        };
        let (w, h) = (image.width() as u32, image.height() as u32);
        let crop = w.min(h) as f32;
        let source = Rect::from_xywh((w as f32 - crop) / 2.0, (h as f32 - crop) / 2.0, crop, crop);
        let target = Rect::from_xywh((i % 2) as f32 * side as f32, (i / 2) as f32 * side as f32, side as f32, side as f32);
        surface.canvas().draw_image_rect_with_sampling_options(&image, Some((&source, skia_safe::canvas::SrcRectConstraint::Strict)), target, SamplingOptions::new(FilterMode::Linear, skia_safe::MipmapMode::None), &paint);
    }
    Some(i_slint_renderer_skia::image_from_skia(surface.image_snapshot()))
}

/// The cover's blurred wash (square ARGB) as an image, dyed as its page is where `dye` says.
fn wash(c: &CoverColours, dye: Option<&nori_look::theme::Dye>) -> Image {
    let Some(w) = &c.wash else { return Image::default() };
    let side = (w.len() as f64).sqrt() as u32;
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(side, side);
    for (px, argb) in buf.make_mut_slice().iter_mut().zip(w) {
        let [a, r, g, b] = dye.map_or(*argb, |d| d.wash(*argb)).to_be_bytes();
        *px = Rgba8Pixel { r, g, b, a };
    }
    Image::from_rgba8(buf)
}

fn colour(argb: u32) -> Color {
    Color::from_argb_encoded(argb)
}

/// The cross-fade of the look, 340 ms as Android's chrome has it: run by a timer only while it lasts.
#[derive(Default)]
struct Fade {
    from: Option<crate::look::Look>,
    /// What was painted last.
    shown: Option<crate::look::Look>,
    began: Option<Instant>,
    timer: Timer,
}

const FADE: Duration = Duration::from_millis(340);

impl Fade {
    fn start(&mut self, from: crate::look::Look) {
        self.from = Some(from);
        self.began = Some(Instant::now());
    }

    /// The look on the way to `to` now, and whether it has arrived.
    fn at(&self, to: &crate::look::Look) -> (crate::look::Look, bool) {
        let (Some(from), Some(began)) = (&self.from, self.began) else { return (to.clone(), true) };
        let t = (began.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.0);
        (from.towards(to, t * t * (3.0 - 2.0 * t)), t >= 1.0)
    }
}

pub struct App {
    me: AppHandle,
    ui: slint::Weak<AppWindow>,
    compositor: Compositor,
    tx: Tx,
    inbox: mpsc::Receiver<Msg>,
    /// Media controls for the process; each session drives them while open.
    mpris: Option<Arc<nori_mpris::Mpris>>,
    data: PathBuf,
    http: Arc<Http>,
    session: Option<Session>,
    art: Rc<RefCell<Art>>,
    lyric_face: Option<Typeface>,
    /// Id and record of the song being heard.
    heard: Option<String>,
    song: Option<Song>,
    queue: Option<QueueShown>,
    /// Songs behind the clickable lists: the page's, the Songs view's, the search's.
    page_songs: Vec<Song>,
    /// The artist page's Show all pressed: every song listed, not the first few.
    page_songs_all: bool,
    page_fetch: Option<Fetch>,
    /// The mix shown, its id and its name: it can be drawn again and kept as a playlist.
    page_mix: Option<(String, String)>,
    /// The queue another device or app left on the server, offered at the top of Home (the core's `queue_offer`).
    offer: Option<nori_core::library::QueueOffer>,
    /// Home's mixes, whose cards take their first cover's colors once known.
    mixes: Vec<nori_core::mixes::board::MixTile>,
    songs: Vec<Song>,
    search_songs: Vec<Song>,
    /// Search's Show all pressed, for the query in `search_query`.
    search_songs_all: bool,
    search_query: String,
    /// The equalizer page's AutoEQ search and what it found, and the saved sound profiles shown.
    autoeq_query: String,
    autoeq_hits: Vec<nori_core::AutoEqEntry>,
    /// The output playing now, as the engine names it.
    output: String,
    /// Home's feed shelves' songs, by shelf (a source's own Home).
    feed_songs: Vec<Vec<Song>>,
    profiles: Vec<nori_core::SoundProfile>,
    /// The sign-in window's process while it is open (nori-sign-in).
    sign_in_window: Option<nori_sign_in_window::SignInWindow>,
    /// The read the shown page waits for.
    want: Option<Req>,
    shelves: Rc<VecModel<Shelf>>,
    /// The rows of the shelves shown.
    home_rows: Vec<nori_core::settings::HomeRow>,
    /// The saved profile the sign-in form edits; None while it adds one.
    editing: Option<SavedServer>,
    /// The album list's pages asked for, to this offset, and its order.
    albums_asked: u32,
    album_sort: AlbumSort,
    /// The player pill and the top pill: windows of their own, laid over the blurred page.
    player: Option<PlayerBar>,
    top: Option<TopBar>,
    /// Cover id whose wash is the window backdrop.
    backdrop: Option<String>,
    /// The covers whose colors the open page and Now Playing wear.
    page_cover: Option<CoverColours>,
    now_cover: Option<CoverColours>,
    /// The view a page was opened from, which back returns to.
    trail: PageTrail,
    /// The look worn (or fading in), and the cross-fade to it.
    look: Option<crate::look::Look>,
    fade: Fade,
    tickets: VecDeque<(CoverKey, Ticket)>,
    tick: Timer,
    tick_ms: u64,
    /// The window is off screen (minimized, covered): the seek bar, lyrics and playing bars stand still.
    hidden: bool,
    /// Motion is E-ink: the playing bars stand still.
    still: bool,
    /// Lyrics are on screen (Now Playing's lyrics or the side sheet): only then are they fetched.
    lyrics_viewed: bool,
    lists: Lists,
    find: String,
    facts: crate::settings::Facts,
    /// The engine is in its shallow equalizer-tuning buffer.
    tuning: bool,
    lyrics: Option<crate::lyrics::SongLyrics>,
    lyrics_timer: Timer,
    /// Active lyric line pieces in the side panel and Now Playing, updated in place each frame.
    pieces: [Rc<VecModel<LyricPiece>>; 2],
    /// Queue rows, edited in place; leaving rows fold first, then the list settles to `queue_next`.
    queue_rows: Rc<VecModel<SongRow>>,
    queue_next: Option<Vec<SongRow>>,
    queue_timer: Timer,
    /// The song last taken out of the queue, while its undo is offered.
    removed: Option<String>,
    undo_timer: Timer,
    note: Timer,
    search: Timer,
    /// The hosted jam's invite link its QR code was drawn for, and whether a jam is hosted.
    jam_link: String,
    jam_hosting: bool,
    /// The song whose menu opened the playlist picker.
    menu_song: Option<Song>,
    /// The sleep timer's choices as offered, its time, and whether one runs.
    sleep_choices: Vec<nori_core::menus::SleepChoice>,
    sleep_timer: Timer,
    sleeping: bool,
    /// Whose playback the player shows and controls, as last read.
    playback: Playback,
    /// A jam guest's controls by its role: what the player offers, and whether it paused here.
    jam_controls: Option<nori_core::remote::JamControls>,
    /// The songs this guest asked for that wait for the host.
    asked: HashSet<String>,
}

/// Wraps `f` to run on the main window while it is open.
fn to_main(ui: &AppWindow, f: impl Fn(&AppWindow) + 'static) -> impl Fn() + 'static {
    let main = ui.as_weak();
    move || {
        if let Some(m) = main.upgrade() {
            f(&m)
        }
    }
}

fn to_main_with<A>(ui: &AppWindow, f: impl Fn(&AppWindow, A) + 'static) -> impl Fn(A) + 'static {
    let main = ui.as_weak();
    move |v| {
        if let Some(m) = main.upgrade() {
            f(&m, v)
        }
    }
}

/// The player pill's window; its buttons forward to the main window.
fn player(ui: &AppWindow, art: impl Fn(SharedString, i32) -> Image + 'static) -> Option<PlayerBar> {
    let bar = PlayerBar::new().map_err(|e| eprintln!("nori: no player window: {e}")).ok()?;
    bar.on_art(move |id, size, _rev| art(id, size));
    bar.on_toggle(to_main(ui, |m| m.invoke_toggle()));
    bar.on_star(to_main(ui, |m| m.invoke_star()));
    bar.on_next(to_main(ui, |m| m.invoke_next()));
    bar.on_previous(to_main(ui, |m| m.invoke_previous()));
    bar.on_toggle_shuffle(to_main(ui, |m| m.invoke_toggle_shuffle()));
    bar.on_cycle_repeat(to_main(ui, |m| m.invoke_cycle_repeat()));
    bar.on_open_full(to_main(ui, |m| {
        if m.get_has_song() {
            m.set_full_player(true);
            m.invoke_player_changed();
        }
    }));
    bar.on_seek(to_main_with(ui, |m, v| m.invoke_seek(v)));
    bar.on_set_volume(to_main_with(ui, |m, v| m.invoke_set_volume(v)));
    bar.on_open_song_menu(to_main(ui, |m| m.invoke_show_player_menu()));
    let go = bar.global::<crate::Go>();
    go.on_artist(to_main_with(ui, |m, id| m.global::<crate::Go>().invoke_artist(id)));
    go.on_album(to_main_with(ui, |m, id| m.global::<crate::Go>().invoke_album(id)));
    bar.on_set_inspector(to_main_with(ui, |m, i| {
        m.set_inspector(i);
        m.invoke_player_changed();
    }));
    Some(bar)
}

/// The top pill's window; its tabs and search field forward to the main window.
fn top(ui: &AppWindow) -> Option<TopBar> {
    let bar = TopBar::new().map_err(|e| eprintln!("nori: no top pill: {e}")).ok()?;
    bar.on_go(to_main_with(ui, |m, v| m.invoke_go(v)));
    bar.on_open_menu(to_main(ui, |m| m.invoke_show_menu()));
    bar.on_search_edited(to_main_with(ui, |m, text| {
        if m.get_view() != SEARCH {
            m.invoke_go(SEARCH);
        }
        m.invoke_search_edited(text);
    }));
    Some(bar)
}

/// The album list once its page from `offset` came (`page`), and the page to ask for next: while pages
/// come full, each once (`asked`: the pages asked for, to this offset).
fn albums_paged(list: &mut Vec<nori_core::Album>, sort: AlbumSort, offset: u32, page: &[nori_core::Album], asked: &mut u32) -> Option<u32> {
    list.truncate(offset as usize);
    list.extend_from_slice(page);
    // Every page is read at once here.
    let next = nori_core::browse::next_album_page(sort, offset, nori_host::pages::ALBUM_PAGE, page.len() as u32).filter(|&n| *asked < n)?;
    *asked = next;
    Some(next)
}

/// The profile the sign-in form saves from its fields (name, address, user, password) for provider `info`:
/// a new one, or `editing` changed, which keeps its id, and its password when none is typed.
fn login_draft(info: &nori_core::providers::ProviderInfo, editing: Option<SavedServer>, [name, url, user, password]: [String; 4]) -> SavedServer {
    use nori_core::providers::FormField;
    let asks = |what| info.fields.iter().any(|f| f.what == what && !f.advanced);
    let base = editing.unwrap_or_else(|| SavedServer { id: nori_core::settings::new_server_id(), ..Default::default() });
    // What the form does not ask stays as saved (nothing, for a new profile).
    let text = |what, typed: String, saved: &str| if asks(what) { typed.trim().to_string() } else { saved.to_string() };
    SavedServer {
        kind: info.kind,
        name: text(FormField::Name, name, &base.name),
        url: text(FormField::Address, url, &base.url).trim_end_matches('/').to_string(),
        user: text(FormField::User, user, &base.user),
        password: if asks(FormField::Password) && !password.is_empty() { password } else { base.password.clone() },
        ..base
    }
}

/// The library's Home shelves of `rows`, empty until read.
fn home_shelves(rows: &[nori_core::settings::HomeRow]) -> Vec<Shelf> {
    rows.iter().map(|&r| Shelf { title: words::home_row(r).into(), cards: ModelRc::default(), loaded: false }).collect()
}

/// Builds the app state and wires the window. Keep the returned app alive while the window runs.
pub fn start(ui: &AppWindow, data: PathBuf, compositor: Compositor) -> Rc<RefCell<App>> {
    let shelves = Rc::new(VecModel::from(Vec::new()));
    ui.set_shelves(ModelRc::from(shelves.clone()));
    ui.set_greeting(words::LISTEN_NOW.into());
    ui.set_show_all_text(words::SHOW_ALL.into());
    ui.set_mixing_label(words::MIXING.into());
    ui.set_login_kinds(ModelRc::new(VecModel::from(nori_core::providers::providers().iter().map(login_kind).collect::<Vec<_>>())));
    let (tx, inbox) = Tx::new(ui.as_weak());
    let art = Rc::new(RefCell::new(Art::default()));
    let app = Rc::new_cyclic(|me: &Weak<RefCell<App>>| {
        let me = AppHandle(me.clone());
        let art_cb = |art: &Rc<RefCell<Art>>| {
            let (art, me) = (art.clone(), me.clone());
            move |id, size| cover_image(&art, &me, id, size)
        };
        let main_art = art_cb(&art);
        ui.on_art(move |id, size, _rev| main_art(id, size));
        let (pick_art, pick_app) = (art.clone(), me.clone());
        let pick_compositor = compositor.clone();
        ui.on_pick_art(move |covers, _rev| pick_image(&pick_art, &pick_app, covers, |images| pick_compositor.mosaic(images)));
        RefCell::new(App {
            ui: ui.as_weak(),
            top: top(ui),
            player: player(ui, art_cb(&art)),
            me,
            compositor,
            tx,
            inbox,
            mpris: nori_mpris::Mpris::for_app(&format!("nori.desktop{}", std::process::id())).ok().map(Arc::new),
            data,
            http: Http::new(),
            session: None,
            art,
            lyric_face: crate::sung::bold_face(),
            heard: None,
            song: None,
            queue: None,
            page_songs: Vec::new(),
            page_songs_all: false,
            page_fetch: None,
            page_mix: None,
            offer: None,
            mixes: Vec::new(),
            songs: Vec::new(),
            search_songs: Vec::new(),
            search_songs_all: false,
            search_query: String::new(),
            autoeq_query: String::new(),
            autoeq_hits: Vec::new(),
            output: String::new(),
            feed_songs: Vec::new(),
            profiles: Vec::new(),
            sign_in_window: None,
            want: None,
            shelves,
            home_rows: Vec::new(),
            editing: None,
            albums_asked: 0,
            album_sort: AlbumSort::ByName,
            backdrop: None,
            page_cover: None,
            now_cover: None,
            trail: PageTrail { tab: HOME, ..Default::default() },
            look: None,
            fade: Fade::default(),
            tickets: VecDeque::new(),
            tick: Timer::default(),
            tick_ms: 0,
            hidden: false,
            still: false,
            lyrics_viewed: false,
            facts: crate::settings::Facts::default(),
            lists: Lists::default(),
            find: String::new(),
            tuning: false,
            lyrics: None,
            lyrics_timer: Timer::default(),
            pieces: [Rc::new(VecModel::default()), Rc::new(VecModel::default())],
            queue_rows: Rc::new(VecModel::default()),
            queue_next: None,
            queue_timer: Timer::default(),
            removed: None,
            undo_timer: Timer::default(),
            note: Timer::default(),
            search: Timer::default(),
            jam_link: String::new(),
            jam_hosting: false,
            menu_song: None,
            sleep_choices: Vec::new(),
            sleep_timer: Timer::default(),
            sleeping: false,
            playback: Playback::Here,
            jam_controls: None,
            asked: HashSet::new(),
        })
    });
    let h = app.borrow().me.clone();
    wire(ui, &h);
    app.borrow().output_shown();
    let me = Rc::downgrade(&app);
    app.borrow().compositor.on_exit(move || {
        if let Some(app) = me.upgrade() {
            stop(&app);
        }
        nori_core::background::flush();
    });
    ui.set_display_position_ms(ui.get_position_ms());
    h.with(|a| {
        ui.set_queue(ModelRc::from(a.queue_rows.clone()));
        ui.set_lyric_pieces_side(ModelRc::from(a.pieces[0].clone()));
        ui.set_lyric_pieces_full(ModelRc::from(a.pieces[1].clone()));
        if let (Some(top), Some(player)) = (&a.top, &a.player) {
            a.compositor.roles(ui.window(), top.window(), player.window());
            let compositor = a.compositor.clone();
            top.on_typing(move |on| compositor.set_typing(on));
            let weak = top.as_weak();
            a.compositor.on_press_away(move || {
                if let Some(top) = weak.upgrade() { top.invoke_end_search(); }
            });
        }
        let me = a.me.clone();
        a.compositor.on_shown(move |shown| me.with(|a| {
            a.hidden = !shown;
            if let Some(s) = &a.session {
                s.in_sight(shown);
            }
            a.follow();
        }));
        #[cfg(target_os = "linux")]
        {
            let weak = ui.as_weak();
            a.compositor.on_mosaics_ready(move || {
                if let Some(ui) = weak.upgrade() { ui.set_covers_rev(ui.get_covers_rev().wrapping_add(1)); }
            });
        }
        ui.set_traffic_lights(cfg!(target_os = "macos"));
        menu_actions(ui, &a.compositor);
        // Now Playing's lyrics: the active line sharp, the rest blurred by the compositor.
        let weak = ui.as_weak();
        a.compositor.set_focus_source(move || {
            let ui = weak.upgrade()?;
            // Only while a line is lit: none before the first, and none once the last is over.
            let lit = usize::try_from(ui.get_lyrics_active()).is_ok_and(|i| i < ui.get_lyrics_lines().row_count());
            if !ui.get_full_player() || ui.get_full_panel() != 2 || !lit || !ui.get_lyrics_synced() || ui.get_full_lyrics_browsed() {
                return None;
            }
            let size = ui.window().size().to_logical(ui.window().scale_factor());
            let x = size.width / 2.0;
            // Clear of the volume pill above and the lyrics/queue pill below.
            // The band where the line sung is now, moving with it as the lyrics scroll; a step a line of 44 px
            // text and its 36 px gap.
            Some(Focus { region: [x, 48.0, size.width - x - 100.0, size.height - 48.0 - 56.0], band_top: ui.get_full_lyric_top() - 8.0, band_h: ui.get_full_lyric_h() + 16.0, pitch: 90.0 })
        });
        a.settings_shown();
        let wake = a.me.clone();
        a.compositor.on_wake(move || wake.with(|a| a.repaint()));
        let prefs = crate::session::app().settings.current().unwrap_or_default();
        match prefs.active_server().cloned() {
            Some(p) => a.open(p),
            None => ui.set_view(LOGIN),
        }
    });
    app
}

/// Menu bar items do what the window's buttons and keys do.
fn menu_actions(ui: &AppWindow, compositor: &Compositor) {
    let on = |id: &str, f: fn(&AppWindow)| compositor.menu().action(id, to_main(ui, f));
    on("toggle", |m| m.invoke_toggle());
    on("next", |m| m.invoke_next());
    on("previous", |m| m.invoke_previous());
    on("shuffle", |m| m.invoke_toggle_shuffle());
    on("repeat", |m| m.invoke_cycle_repeat());
    on("home", |m| m.invoke_go(HOME));
    on("albums", |m| m.invoke_go(ALBUMS));
    on("artists", |m| m.invoke_go(ARTISTS));
    on("songs", |m| m.invoke_go(SONGS));
    on("search", |m| m.invoke_go(SEARCH));
    on("settings", |m| m.invoke_go(SETTINGS));
    on("join-jam", |m| m.set_join_open(true));
    on("queue", |m| m.set_inspector(if m.get_inspector() == 1 { 0 } else { 1 }));
    on("lyrics", |m| m.set_inspector(if m.get_inspector() == 2 { 0 } else { 2 }));
    on("full", |m| {
        if m.get_has_song() {
            m.set_full_player(!m.get_full_player());
        }
    });
}

/// The app is quitting: saves the volume and queue, stops the engine.
fn stop(app: &RefCell<App>) {
    if let Some(s) = app.borrow_mut().session.take() {
        session::own::keep(session::own::VOLUME, s.volume().to_string());
        s.close();
    }
}

/// Opens `url` in the system's browser.
fn open_in_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(opener).arg(url).spawn();
}

/// A kind of profile as the login page offers it.
fn login_kind(info: &nori_core::providers::ProviderInfo) -> LoginKind {
    use nori_core::providers::FormField;
    let w = words::provider(info.kind);
    let asks = |what| info.fields.iter().any(|f| f.what == what && !f.advanced);
    LoginKind {
        name: w.name.into(),
        note: w.note.into(),
        add: w.add.into(),
        asks_name: asks(FormField::Name),
        asks_address: asks(FormField::Address),
        asks_user: asks(FormField::User),
        asks_password: asks(FormField::Password),
    }
}

fn wire(ui: &AppWindow, h: &AppHandle) {
    on!(ui.on_messages_arrived, h, |a| a.drain_inbox());
    on!(ui.on_go, h, |a, v| a.go(v));
    on!(ui.on_shelf_song, h, |a, shelf, id| {
        let songs = a.feed_songs.get(shelf.max(0) as usize).cloned().unwrap_or_default();
        if let Some(at) = songs.iter().position(|s| s.id == id.as_str()) {
            a.on_session(|s| s.play(songs, at, false, None));
        }
    });
    on!(ui.on_back, h, |a| a.back());
    on!(ui.on_note_act, h, |a| a.note_act());
    on!(ui.on_open_album, h, |a, id| a.open_page(PageAsk::Album(id.into())));
    on!(ui.on_open_artist, h, |a, id| a.open_page(PageAsk::Artist(id.into())));
    on!(ui.on_open_playlist, h, |a, id| a.open_page(PageAsk::Playlist(id.into())));
    on!(ui.on_open_mix, h, |a, id| a.open_page(PageAsk::Mix(id.into())));
    on!(ui.on_play_album, h, |a, id| a.play_fetch(Fetch::Album(id.into())));
    on!(ui.on_play_playlist, h, |a, id| a.play_fetch(Fetch::Playlist(id.into())));
    on!(ui.on_play_mix, h, |a, id| a.play_fetch(Fetch::Mix(id.into())));
    on!(ui.on_queue_card, h, |a, kind, id, next| a.queue_card(kind, id.into(), next));
    on!(ui.on_song, h, |a, list, i, how| a.song(list, i as usize, how));
    let menu = h.clone();
    ui.on_song_menu(move |list, i| {
        let mut lines = ModelRc::default();
        menu.with(|a| lines = a.song_menu(list, i as usize));
        lines
    });
    let menu = h.clone();
    ui.on_player_menu(move || {
        let mut lines = ModelRc::default();
        menu.with(|a| lines = a.player_menu());
        lines
    });
    on!(ui.on_player_act, h, |a, kind, _id| a.player_act(kind));
    on!(ui.on_sleep, h, |a, i| a.sleep(i as usize));
    on!(ui.on_lyrics_nudge, h, |a, dir| a.lyrics_nudge(dir));
    on!(ui.on_add_to_playlist, h, |a, id| a.add_to_playlist(Ok(id.into())));
    on!(ui.on_add_to_new_playlist, h, |a, name| a.add_to_playlist(Err(name.into())));
    on!(ui.on_play_page, h, |a, shuffle| a.play_page(shuffle));
    on!(ui.on_page_star, h, |a| a.star_page());
    on!(ui.on_page_act, h, |a, act| a.page_act(act));
    on!(ui.on_page_save, h, |a| a.save_mix());
    on!(ui.on_take_offer, h, |a| a.close_offer(true));
    on!(ui.on_dismiss_offer, h, |a| a.close_offer(false));
    on!(ui.on_more, h, |a| a.more_songs());
    on!(ui.on_search_edited, h, |a, t| a.search_edited(&t));
    on!(ui.on_search_again, h, |a, t| a.search_again(&t));
    on!(ui.on_forget_searches, h, |a| a.forget_searches());
    on!(ui.on_toggle, h, |a| a.on_session(|s| s.toggle()));
    on!(ui.on_star, h, |a| {
        if let Some(song) = a.song.clone() {
            a.star(&song);
        }
    });
    on!(ui.on_next, h, |a| a.on_session(|s| s.next()));
    on!(ui.on_previous, h, |a| a.on_session(|s| s.previous()));
    on!(ui.on_seek, h, |a, f| a.seek(f));
    on!(ui.on_set_volume, h, |a, v| {
        a.on_session(|s| s.set_volume(v));
        a.ui().set_volume(v);
        a.mirror();
    });
    on!(ui.on_toggle_shuffle, h, |a| {
        let on = !a.ui().get_shuffle();
        a.on_session(|s| s.shuffle(on));
        a.say(if on { "Shuffle on" } else { "Shuffle off" }, false);
        a.follow();
    });
    on!(ui.on_cycle_repeat, h, |a| {
        a.on_session(|s| s.cycle_repeat());
        let shown = a.session.as_ref().map_or(0, |s| s.now().repeat);
        a.say(["Repeat off", "Repeat one", "Repeat all"][shown.min(2) as usize], false);
        a.follow();
    });
    on!(ui.on_lyric_tapped, h, |a, line| a.lyric_tapped(line));
    on!(ui.on_jump, h, |a, i| a.on_session(|s| s.jump(i.max(0) as usize)));
    on!(ui.on_seek_by, h, |a, ms| {
        if a.session.is_some() {
            a.seek_to((a.position_now() + ms as i64).max(0));
        }
    });
    on!(ui.on_drag_window, h, |a| a.compositor.drag_window());
    on!(ui.on_all_songs, h, |a, list| {
        if list == 0 { a.page_songs_all = true; a.show_page_songs(); } else { a.search_songs_all = true; a.show_search_songs(); }
    });
    on!(ui.on_zoom_window, h, |a| a.compositor.zoom_window());
    on!(ui.on_page_moved, h, |a| a.place_player());
    on!(ui.on_player_changed, h, |a| {
        a.repaint();
        position_shown(&a.ui(), a.player.as_ref(), a.position_now() as i32);
        a.lyrics_seen();
        a.lyrics_step(true);
        a.mirror();
        a.place_player();
        a.devices_watched();
    });
    on!(ui.on_pick_device, h, |a, id| a.pick_device(id.to_string()));
    let go = ui.global::<crate::Go>();
    on!(go.on_artist, h, |a, id| a.go_to(PageAsk::Artist(id.into())));
    on!(go.on_album, h, |a, id| a.go_to(PageAsk::Album(id.into())));
    on!(ui.on_jam_play, h, |a, kind, id| {
        let what = if kind == 0 { Fetch::Album(id.into()) } else { Fetch::Playlist(id.into()) };
        a.on_session(|s| {
            s.play_later(what, false);
            s.jam_open();
        });
    });
    use nori_core::remote::wire::Op;
    let jam = ui.global::<crate::Jam>();
    on!(jam.on_start, h, |a| a.on_session(|s| s.jam_open()));
    on!(jam.on_end, h, |a| {
        if let Some(r) = a.session.as_ref().and_then(|s| s.remote()) {
            r.jam_close();
        }
    });
    on!(jam.on_copy_link, h, |a| a.jam_copy_link());
    on!(ui.on_copy_pairing, h, |a| a.copy(a.ui().get_pairing_link().into()));
    on!(jam.on_leave, h, |a| a.leave_jam());
    on!(ui.on_forget_devices, h, |a| {
        if let Some(remote) = a.session.as_ref().and_then(|s| s.remote()) { remote.reset_pairing(); }
        a.devices_shown();
        a.jam_shown();
    });
    on!(ui.on_join_jam, h, |a, link| a.join_jam(link.trim().to_string()));
    on!(ui.on_sign_in, h, |a, answer| a.sign_in(answer.to_string()));
    on!(ui.on_sign_in_window, h, |a| a.sign_in_window());
    on!(ui.on_sign_out, h, |a| a.sign_out());
    on!(ui.on_sign_in_quiet, h, |a| a.sign_in_quiet());
    ui.on_open_sign_in_page(|| open_in_browser(words::YOUTUBE_MUSIC_PAGE));
    on!(jam.on_listen, h, |a, on| {
        if let Some(r) = a.session.as_ref().and_then(|s| s.remote()) {
            r.listen(on);
        }
    });
    on!(jam.on_set_remote_songs, h, |a, on| {
        if let Some(r) = a.session.as_ref().and_then(|s| s.remote()) {
            r.jam_remote_songs(on);
        }
    });
    on!(jam.on_set_along, h, |a, on| {
        if let Some(r) = a.session.as_ref().and_then(|s| s.remote()) {
            r.jam_along(on);
        }
    });
    on!(jam.on_decide, h, |a, request, accept| {
        if let Ok(request) = request.parse() {
            a.jam_act(Op::Decide { request, accept });
        }
    });
    on!(jam.on_promote, h, |a, member, admin| a.jam_act(Op::Promote { member: member.into(), admin }));
    on!(jam.on_send_out, h, |a, member| a.jam_act(Op::Kick { member: member.into() }));
    on!(ui.on_login, h, |a| a.login());
    on!(ui.on_cancel_login, h, |a| {
        a.editing = None;
        a.go(HOME);
    });
    on!(ui.on_find_edited, h, |a, t| {
        a.find = t.to_string();
        a.narrowed();
    });
    on!(ui.on_choose_artist, h, |a, id| a.choose_artist(id.to_string()));
    on!(ui.on_album_sort_chosen, h, |a, at| a.sort_albums(at as usize));
    on!(ui.on_open_accounts, h, |a| {
        let accounts = crate::settings::tabs(&a.facts.rules.settings).iter().position(|p| *p == nori_core::settings_pages::SettingsPage::Servers);
        a.ui().set_settings_tab(accounts.unwrap_or(0) as i32);
        a.go(SETTINGS);
    });
    on!(ui.on_clear_queue, h, |a| {
        a.on_session(|s| s.clear_upcoming());
        a.follow();
    });
    on!(ui.on_remove_queued, h, |a, i| a.remove_queued(i.max(0) as usize));
    on!(ui.on_move_queued, h, |a, from, to| {
        a.on_session(|s| s.move_song(from.max(0) as usize, to.max(0) as usize));
        a.follow();
    });
    on!(ui.on_undo_removed, h, |a| {
        if let Some(id) = a.removed.take() {
            a.on_session(|s| s.put_back(&id));
        }
        a.undo_timer.stop();
        a.ui().set_queue_undo("".into());
        a.follow();
    });
    on!(ui.on_settings_tab_chosen, h, |a, t| {
        a.ui().set_settings_tab(t);
        a.untune(SETTINGS);
        a.settings_shown();
    });
    on!(ui.on_setting_toggled, h, |a, name, on| {
        let (name, value) = crate::settings::toggled(&name, on);
        a.setting(name, &value);
    });
    on!(ui.on_setting_chosen, h, |a, name, i| a.setting_chosen(&name, i.max(0) as usize));
    on!(ui.on_setting_action, h, |a, name| a.setting_action(&name));
    on!(ui.on_setting_slid, h, |a, name, v, last| a.slid(&name, v, last));
    on!(ui.on_setting_typed, h, |a, name, text| a.setting(&name, &text));
    on!(ui.on_source_moved, h, |a, id, by| a.source_moved(&id, by));
    on!(ui.on_eq_set, h, |a, name, value| {
        a.setting(&name, &value);
        a.tune();
    });
    on!(ui.on_eq_gain, h, |a, i, v, last| a.eq_gain(i.max(0) as usize, v, last));
    on!(ui.on_eq_tool, h, |a, name, i| a.eq_tool(&name, i.max(0) as usize));
    on!(ui.on_eq_autoeq_search, h, |a, q| a.autoeq_search(&q));
    on!(ui.on_eq_autoeq_pick, h, |a, i| a.autoeq_pick(i.max(0) as usize));
    on!(ui.on_eq_autoeq_download, h, |a| a.autoeq_download());
    on!(ui.on_eq_profile_apply, h, |a, i| a.profile_apply(i.max(0) as usize));
    on!(ui.on_eq_profile_delete, h, |a, i| a.profile_delete(i.max(0) as usize));
    on!(ui.on_eq_profile_save, h, |a, name| a.profile_save(&name));
    on!(ui.on_eq_sized, h, |a, w, hh| {
        let ui = a.ui();
        ui.set_eq_curve_w(w);
        ui.set_eq_curve_h(hh);
        if let Some(p) = crate::session::app().settings.current() {
            crate::eq::curve_only(&ui, &p);
        }
    });
}

impl App {
    fn ui(&self) -> AppWindow {
        self.ui.upgrade().expect("the window is open while its state is")
    }

    fn on_session(&self, f: impl FnOnce(&Session)) {
        if let Some(s) = &self.session {
            f(s);
        }
    }

    fn drain_inbox(&mut self) {
        while let Ok(m) = self.inbox.try_recv() {
            match m {
                Msg::From(id, m) if self.session.as_ref().is_some_and(|s| s.id == id) => self.take(*m),
                // Left in flight by an earlier session.
                Msg::From(..) => {}
                m => self.take(m),
            }
        }
    }

    /// Adds the whole page to the queue.
    fn enqueue_page(&self) {
        if !self.page_songs.is_empty() {
            self.on_session(|s| s.enqueue(self.page_songs.clone(), false));
        }
    }

    fn setting_chosen(&mut self, name: &str, i: usize) {
        if name == crate::settings::APPEARANCE {
            self.setting("theme", crate::settings::appearance_theme(i));
            return;
        }
        // The season tiles are the seasons alone; the core's options put Follow before them.
        let i = if name == "season" { i + 1 } else { i };
        let target = Target::of(name);
        let Some(v) = crate::settings::option_value(target, i, &self.facts) else { return };
        match target {
            Target::Device => {
                // The output opens with the engine, so a new device applies from the next start.
                session::own::keep(session::own::DEVICE, v.clone());
                self.facts.device = v;
                self.say("The new output is used from the next start", false);
                self.settings_shown();
            }
            Target::Setting(name) => self.setting(name, &v),
        }
    }

    fn say(&self, text: &str, error: bool) {
        self.note_for(text, error, "", Duration::from_millis(if error { 5000 } else { 2500 }));
    }

    /// A note with a button, up long enough to be answered.
    fn ask(&self, text: &str, action: &str) {
        self.note_for(text, false, action, Duration::from_secs(8));
    }

    fn note_for(&self, text: &str, error: bool, action: &str, up: Duration) {
        let ui = self.ui();
        ui.set_note(text.into());
        ui.set_note_error(error);
        ui.set_note_action(action.into());
        let weak = self.ui.clone();
        self.note.start(TimerMode::SingleShot, up, move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_note("".into());
                ui.set_note_action("".into());
            }
        });
    }

    /// The note's button: the session's last AutoEQ notice answered.
    fn note_act(&self) {
        self.on_session(|s| s.curve_answer());
        let ui = self.ui();
        ui.set_note("".into());
        ui.set_note_action("".into());
    }

    fn open(&mut self, profile: SavedServer) {
        if let Some(s) = self.session.take() {
            s.close();
        }
        let ui = self.ui();
        let (who, where_) = words::profile_lines(&profile);
        ui.set_account_initial(who.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default().into());
        ui.set_account(who.into());
        ui.set_server(where_.into());
        match Session::open(&self.data, self.http.clone(), profile, self.tx.clone(), self.mpris.clone()) {
            Ok(s) => {
                s.check();
                ui.set_volume(s.volume());
                ui.set_downloads(s.rules().downloads);
                // Albums and artists carry the account's seals, as on Android.
                ui.set_page_can_star(s.rules().account && s.rules().stars);
                self.session = Some(s);
                // The last profile's song leaves the screen; the new one's, if it has one, comes with follow.
                self.song_shown(None);
                // The new profile's core has no lyrics view open yet.
                self.lyrics_viewed = false;
                self.queue = None;
                self.home_rows.clear();
                self.shelves.set_vec(Vec::new());
                ui.set_picks(ModelRc::default());
                ui.set_picks_loaded(false);
                self.playback = Playback::Here;
                self.follow();
                self.jam_shown();
                self.go(HOME);
                if self.session.as_ref().is_some_and(|s| s.client.reminds_to_sign_in()) {
                    self.offer_sign_in();
                }
                if !self.account() {
                    // A jam guest's: the host's library, and the jam in the queue panel.
                    ui.set_inspector(1);
                }
            }
            Err(e) => {
                ui.set_login_error(e.into());
                ui.set_view(LOGIN);
            }
        }
    }

    fn login(&mut self) {
        let ui = self.ui();
        let Some(info) = nori_core::providers::providers().into_iter().nth(ui.get_login_kind().max(0) as usize) else { return };
        let form = [ui.get_login_name(), ui.get_login_url(), ui.get_login_user(), ui.get_login_password()].map(|t| t.to_string());
        let draft = login_draft(&info, self.editing.clone(), form);
        if !nori_core::providers::server_ready(draft.clone()) {
            ui.set_login_error("The server's address and a user name, please.".into());
            return;
        }
        ui.set_login_busy(true);
        ui.set_login_error("".into());
        let (http, tx) = (self.http.clone(), self.tx.clone());
        std::thread::spawn(move || tx.send(Msg::LoggedIn(session::check_login(http, draft))));
    }

    fn go(&mut self, view: i32) {
        // A new page starts unfiltered.
        if !self.find.is_empty() {
            self.find.clear();
            self.ui().set_find_text("".into());
            self.narrowed();
        }
        self.untune(view);
        let ui = self.ui();
        ui.set_failed("".into());
        ui.set_loading(false);
        if view == LOGIN {
            ui.set_can_cancel_login(self.session.is_some());
            ui.set_login_error("".into());
            ui.set_view(LOGIN);
            return;
        }
        ui.set_view(view);
        // The search page's field is the top pill's.
        if let Some(top) = &self.top {
            if view == SEARCH { top.invoke_focus_search() } else { top.invoke_end_search() }
        }
        if view == SEARCH && self.top.as_ref().is_none_or(|t| t.get_search_text().trim().is_empty()) {
            self.search_edited("");
        }
        let req = match view {
            HOME => {
                self.look_for_offer();
                self.home_req()
            }
            ALBUMS => {
                self.albums_asked = 0;
                self.album_sort = nori_core::browse::album_sort_saved(crate::session::app().settings.current().unwrap_or_default().list_prefs);
                self.show_album_sorts();
                Some(Req::Page(PageAsk::Albums { sort: self.album_sort, offset: 0 }))
            }
            ARTISTS => Some(Req::Page(PageAsk::Artists)),
            PLAYLISTS => Some(Req::Page(PageAsk::Playlists)),
            FAVORITES => {
                self.songs.clear();
                Some(Req::Page(PageAsk::Favorites))
            }
            SONGS => {
                self.songs.clear();
                Some(Req::Page(PageAsk::Songs { offset: 0 }))
            }
            SETTINGS => {
                self.settings_shown();
                self.on_session(|s| s.facts());
                None
            }
            EQUALIZER => {
                self.settings_shown();
                None
            }
            _ => None,
        };
        if let Some(r) = req {
            self.load(r);
        }
    }

    fn load(&mut self, req: Req) {
        let Some(s) = &self.session else { return };
        self.ui().set_loading(true);
        self.want = Some(req.clone());
        s.load(req);
    }

    /// Home's read: the shelves of the rows the profile shows, drawn anew when those changed.
    fn home_req(&mut self) -> Option<Req> {
        let rows = self.session.as_ref()?.home_album_rows();
        if rows != self.home_rows {
            self.shelves.set_vec(home_shelves(&rows));
            self.home_rows = rows.clone();
        }
        Some(Req::Home(rows))
    }

    fn open_page(&mut self, ask: PageAsk) {
        self.trail.opened(ask.clone(), opened_from(self.ui().get_view()));
        self.show_page(ask);
    }

    /// Shows page `ask`, the way back already kept ([`PageTrail`]).
    fn show_page(&mut self, ask: PageAsk) {
        if !self.find.is_empty() {
            self.find.clear();
            self.ui().set_find_text("".into());
            self.narrowed();
        }
        let ui = self.ui();
        self.page_songs.clear();
        self.page_songs_all = false;
        self.page_fetch = Some(match &ask {
            PageAsk::Album(id) => Fetch::Album(id.clone()),
            PageAsk::Artist(id) => Fetch::Artist(id.clone()),
            PageAsk::Playlist(id) => Fetch::Playlist(id.clone()),
            PageAsk::Mix(id) => Fetch::Mix(id.clone()),
            _ => return,
        });
        ui.set_page_id(match &ask {
            PageAsk::Album(id) | PageAsk::Artist(id) | PageAsk::Playlist(id) | PageAsk::Mix(id) => id.as_str().into(),
            _ => "".into(),
        });
        ui.set_page_kind(match &ask {
            PageAsk::Album(_) => 0,
            PageAsk::Artist(_) => 1,
            _ => 2,
        });
        ui.set_page_mix(matches!(ask, PageAsk::Mix(_)));
        ui.set_page_title("".into());
        ui.set_page_sub("".into());
        ui.set_page_sub_artist("".into());
        ui.set_page_caption("".into());
        ui.set_page_art("".into());
        ui.set_page_covers(ModelRc::default());
        ui.set_page_acts(ModelRc::default());
        ui.set_page_can_save(false);
        self.page_mix = None;
        ui.set_page_songs(ModelRc::default());
        ui.set_page_shelves(ModelRc::default());
        self.page_colours(None);
        ui.set_view(PAGE);
        ui.set_failed("".into());
        self.mirror();
        self.load(Req::Page(ask));
    }

    /// Opens a page from a name that links to it, leaving Now Playing for it.
    fn go_to(&mut self, ask: PageAsk) {
        self.ui().set_full_player(false);
        self.open_page(ask);
    }

    /// The page's cover, whose colors the page wears, or none. Its header is the cover's own wash, undyed, as
    /// Android's page shows the cover itself and lets it melt into the dyed page; none where pages wear no cover.
    fn page_colours(&mut self, c: Option<&CoverColours>) {
        let tinted = crate::session::app().settings.prefs(|p| p.theme.covers_tint_pages());
        self.ui().set_page_wash(c.filter(|_| tinted).map(|c| wash(c, None)).unwrap_or_default());
        self.page_cover = c.cloned();
        self.repaint();
    }

    /// Cross-fades the window backdrop to this cover's wash (into the hidden layer; app.slint animates), and
    /// dresses Now Playing in its colors.
    fn now_colours(&mut self, art: &str, c: Option<&CoverColours>) {
        if self.backdrop.as_deref() == Some(art) && c.is_some() {
            return;
        }
        let Some(c) = c else {
            self.backdrop = None;
            self.now_cover = None;
            self.repaint();
            return;
        };
        let ui = self.ui();
        self.backdrop = Some(art.to_string());
        let on_b = !ui.get_wash_on_b();
        let dyed = self.dyed_wash(c).unwrap_or_default();
        if on_b {
            ui.set_wash_b(dyed);
        } else {
            ui.set_wash_a(dyed);
        }
        ui.set_wash_on_b(on_b);
        self.now_cover = Some(c.clone());
        self.repaint();
    }

    /// A cover's wash on the season's paper, as its page is dyed; none where pages wear no cover.
    fn dyed_wash(&self, c: &CoverColours) -> Option<Image> {
        let prefs = crate::session::app().settings.current().unwrap_or_default();
        crate::look::dye(&prefs, self.compositor.system_dark(), crate::look::Today::now(), c).map(|d| wash(c, Some(&d)))
    }

    /// The look of the page the app is on: the season's, or a cover's.
    fn page_look(&self) -> crate::look::Look {
        let ui = self.ui();
        let prefs = crate::session::app().settings.current().unwrap_or_default();
        let (dark, today) = (self.compositor.system_dark(), crate::look::Today::now());
        let cover = if ui.get_full_player() {
            self.now_cover.as_ref()
        } else if ui.get_view() == PAGE {
            self.page_cover.as_ref()
        } else {
            None
        };
        match cover {
            Some(c) => crate::look::cover(&prefs, dark, today, c),
            None => crate::look::plain(&prefs, dark, today),
        }
    }

    /// Dresses every window in the page's look, cross-fading from the one worn unless motion is reduced.
    fn repaint(&mut self) {
        let want = self.page_look();
        let prefs = crate::session::app().settings.current().unwrap_or_default();
        self.ui().set_covers_tint(prefs.theme.covers_tint_pages());
        // nori's Motion setting, as on Android: animations run unless it asks for less.
        let motion = prefs.motion;
        let reduced = motion != nori_core::settings::Motion::Full;
        self.still = motion == nori_core::settings::Motion::Eink;
        self.advancing();
        self.ui().global::<Motion>().set_reduced(reduced);
        if let Some(p) = &self.player {
            p.global::<Motion>().set_reduced(reduced);
        }
        if let Some(t) = &self.top {
            t.global::<Motion>().set_reduced(reduced);
        }
        match self.look.clone() {
            Some(now) if now != want && !reduced => {
                let from = self.fade.shown.clone().unwrap_or(now);
                self.fade.start(from);
                let me = self.me.clone();
                self.fade.timer.start(TimerMode::Repeated, Duration::from_millis(16), move || me.with(|a| a.fade_step()));
            }
            Some(now) if now == want => return,
            _ => {
                self.fade.timer.stop();
                self.paint(&want);
                self.fade.shown = Some(want.clone());
            }
        }
        self.look = Some(want);
    }

    fn fade_step(&mut self) {
        let Some(want) = self.look.clone() else { return };
        let (look, done) = self.fade.at(&want);
        self.paint(&look);
        self.fade.shown = Some(look);
        if done {
            self.fade.timer.stop();
        }
    }

    fn paint(&self, look: &crate::look::Look) {
        look.paint(&self.ui().global::<Theme>());
        if let Some(p) = &self.player {
            look.paint(&p.global::<Theme>());
        }
        if let Some(t) = &self.top {
            look.paint(&t.global::<Theme>());
        }
        self.compositor.set_page_colour(look.page());
    }

    fn data(&mut self, req: Req, r: Result<Data, String>) {
        let ui = self.ui();
        let shown = self.want.as_ref() == Some(&req);
        if shown {
            ui.set_loading(false);
        }
        let d = match r {
            Ok(d) => d,
            Err(e) => {
                if shown {
                    ui.set_failed(e.into());
                }
                return;
            }
        };
        match d {
            Data::Home(HomePart::Mixes(tiles)) => {
                self.art.borrow_mut().mosaics.clear();
                self.mixes = tiles;
                self.show_picks();
            }
            Data::Home(HomePart::Feed(feed)) => {
                // Each shelf as the source lists it: its songs first (a song card plays the shelf's songs
                // from it), then its albums, playlists and artists.
                let shelves = feed.iter().filter(|f| !(f.songs.is_empty() && f.albums.is_empty() && f.playlists.is_empty() && f.artists.is_empty())).map(|f| {
                    let all = f.songs.iter().map(song_card).chain(f.albums.iter().map(album_card)).chain(f.playlists.iter().map(playlist_card)).chain(f.artists.iter().map(artist_card));
                    Shelf { title: f.title.as_str().into(), cards: cards(all), loaded: true }
                });
                self.shelves.set_vec(shelves.collect::<Vec<_>>());
                self.feed_songs = feed.into_iter().filter(|f| !(f.songs.is_empty() && f.albums.is_empty() && f.playlists.is_empty() && f.artists.is_empty())).map(|f| f.songs).collect();
            }
            Data::Home(HomePart::Shelf { at, items }) => {
                let ShelfItems::Albums(albums) = items else { return };
                if let Some(mut shelf) = self.shelves.row_data(at) {
                    shelf.cards = cards(albums.iter().map(album_card));
                    shelf.loaded = true;
                    self.shelves.set_row_data(at, shelf);
                }
            }
            Data::Page(PageGot::Albums(v)) => {
                // Pages of the list, each asked for once the one before came full.
                let Req::Page(PageAsk::Albums { sort, offset }) = req else { return };
                if sort != self.album_sort {
                    return;
                }
                if let Some(next) = albums_paged(&mut self.lists.albums, sort, offset, &v, &mut self.albums_asked) {
                    self.on_session(|s| s.load(Req::Page(PageAsk::Albums { sort, offset: next })));
                }
                self.narrowed();
            }
            Data::Page(PageGot::Artists(v)) => {
                self.lists.artists = v.iter().map(artist_card).collect();
                self.narrowed();
                // The Artists view opens on its first artist, unless one of these is chosen already (the last
                // profile's is not).
                let chosen = ui.get_artist_chosen();
                if !v.iter().any(|a| a.id == chosen.as_str()) {
                    match v.first() {
                        Some(first) => self.choose_artist(first.id.clone()),
                        None => ui.set_artist_chosen("".into()),
                    }
                }
            }
            Data::Page(PageGot::Playlists(v)) => {
                self.lists.playlists = v.iter().map(playlist_card).collect();
                self.narrowed();
            }
            Data::Page(PageGot::Favorites(v)) => {
                self.songs = v.songs;
                ui.set_more_songs(false);
                ui.set_songs(self.rows(&self.songs));
            }
            Data::Page(PageGot::Songs { songs: v, exhausted }) => {
                self.songs.extend(v);
                ui.set_more_songs(!exhausted);
                ui.set_songs(self.rows(&self.songs));
            }
            // Page details apply only if that page is still shown.
            Data::Page(PageGot::Album(d)) if shown => {
                ui.set_page_starred(self.session.as_ref().is_some_and(|s| s.core.starred(nori_core::client::Starrable::Album, &d.album.id, d.album.starred)));
                ui.set_page_title(d.album.name.as_str().into());
                ui.set_page_sub(d.album.artist.as_str().into());
                ui.set_page_sub_artist(d.album.artist_id.clone().unwrap_or_default().into());
                let mut caption = Vec::new();
                if d.album.year > 0 {
                    caption.push(d.album.year.to_string());
                }
                caption.push(words::songs_caption(d.songs.len(), d.seconds));
                if let Some(q) = d.songs.first().and_then(words::quality) {
                    caption.push(q);
                }
                ui.set_page_caption(caption.join(" · ").into());
                self.set_page_art(d.album.cover_art.clone());
                self.page_songs = d.songs;
                self.page_actions(PageKind::Album);
                self.show_page_songs();
            }
            Data::Page(PageGot::Artist(d)) if shown => {
                ui.set_page_starred(self.session.as_ref().is_some_and(|s| s.core.starred(nori_core::client::Starrable::Artist, &d.artist.id, d.artist.starred)));
                ui.set_page_title(d.artist.name.as_str().into());
                ui.set_page_sub("".into());
                ui.set_page_caption(words::artist_caption(d.albums.len(), d.albums.iter().map(|a| a.song_count as usize).sum(), d.songs.len()).into());
                self.set_page_art(d.artist.cover_art.clone().or_else(|| d.albums.first().and_then(|a| a.cover_art.clone())));
                let card = |a: &nori_core::Album| Card { sub: if a.year > 0 { a.year.to_string().into() } else { "".into() }, sub_artist: SharedString::default(), ..album_card(a) };
                let shelves = d.groups.iter().map(|g| Shelf { title: words::release_shelf(g).into(), cards: cards(g.albums.iter().filter_map(|&i| d.albums.get(i as usize)).map(card)), loaded: true });
                ui.set_page_shelves(ModelRc::new(VecModel::from(shelves.collect::<Vec<_>>())));
                self.page_songs = d.songs;
                self.page_actions(PageKind::Artist);
                self.show_page_songs();
            }
            Data::Page(PageGot::Playlist(d)) if shown => {
                ui.set_page_title(d.playlist.name.as_str().into());
                let notes = crate::session::app().settings.prefs(|p| p.playlist_notes);
                ui.set_page_sub(nori_core::pages::playlist_description(d.playlist.comment.clone(), notes).unwrap_or_default().into());
                ui.set_page_caption(if d.songs.is_empty() { String::new() } else { words::songs_caption(d.songs.len(), d.seconds) }.into());
                self.set_page_art(d.playlist.cover_art.clone());
                self.page_songs = d.songs;
                self.page_actions(PageKind::Playlist);
                self.show_page_songs();
            }
            Data::Page(PageGot::Mix(m)) if shown => {
                ui.set_page_title(words::mix_name(m.name).into());
                ui.set_page_caption(if m.songs.is_empty() { String::new() } else { words::songs_caption(m.songs.len(), m.seconds) }.into());
                ui.set_page_covers(ModelRc::new(VecModel::from(m.covers.iter().map(SharedString::from).collect::<Vec<_>>())));
                ui.set_page_can_save(self.session.as_ref().is_some_and(|s| s.rules().account) && !m.songs.is_empty());
                self.set_page_art(m.covers.first().cloned());
                self.page_songs = m.songs;
                self.page_mix = Some((m.id.clone(), words::mix_name(m.name).to_string()));
                self.page_actions(PageKind::Mix { refreshable: m.refreshable });
                self.show_page_songs();
            }
            Data::Page(PageGot::Album(_) | PageGot::Artist(_) | PageGot::Playlist(_) | PageGot::Mix(_)) => {}
        }
    }

    /// Sets the page picture, whose colors the page wears: a mix's first cover's.
    fn set_page_art(&mut self, art: Option<String>) {
        let art = art.unwrap_or_default();
        let ui = self.ui();
        ui.set_page_art(art.as_str().into());
        let c = self.art.borrow().colours.get(&art).cloned();
        self.page_colours(c.as_deref());
    }

    /// The list head's glyphs for the page's songs (the core's `page_actions`), how many are not downloaded
    /// read once, and its Shuffle and Play.
    fn page_actions(&mut self, kind: PageKind) {
        use nori_core::menus::{page_actions, DownloadAct, PageAction, SongDownload};
        let Some(s) = &self.session else { return };
        let ids: Vec<&str> = self.page_songs.iter().map(|s| s.id.as_str()).collect();
        let missing = s.download_states(&ids).iter().filter(|d| **d != SongDownload::Done).count();
        let acts: Vec<i32> = page_actions(kind, ids.len() as u32, missing as u32, s.rules().downloads)
            .into_iter()
            .filter_map(|a| match a {
                PageAction::Download { act: DownloadAct::All } => Some(0),
                PageAction::Download { act: DownloadAct::Missing } => Some(1),
                PageAction::Download { act: DownloadAct::Remove } => Some(2),
                PageAction::DownloadEverything => Some(3),
                PageAction::AddToQueue => Some(4),
                PageAction::Refresh => Some(5),
                PageAction::Edit => None,
            })
            .collect();
        self.ui().set_page_acts(ModelRc::new(VecModel::from(acts)));
        self.hero_shown();
    }

    /// A list head glyph pressed ([`App::page_actions`]'s numbers).
    fn page_act(&mut self, act: i32) {
        let Some(fetch) = self.page_fetch.clone() else { return };
        match act {
            0 | 3 => self.on_session(|s| s.download_later(fetch)),
            1 => {
                let Some(s) = &self.session else { return };
                let ids: Vec<&str> = self.page_songs.iter().map(|s| s.id.as_str()).collect();
                let states = s.download_states(&ids);
                let missing = self.page_songs.iter().zip(states).filter(|(_, d)| *d != nori_core::menus::SongDownload::Done).map(|(s, _)| s.clone()).collect();
                s.download(missing);
            }
            2 => self.on_session(|s| self.page_songs.iter().for_each(|song| s.download_remove(&song.id))),
            4 if matches!(fetch, Fetch::Artist(_)) => self.on_session(|s| s.enqueue_later(fetch, false)),
            4 => self.enqueue_page(),
            5 => self.redraw_mix(),
            _ => {}
        }
        if act != 5 {
            let kind = self.ui().get_page_kind();
            self.page_actions(match (kind, &self.page_mix) {
                (0, _) => PageKind::Album,
                (1, _) => PageKind::Artist,
                (_, Some(_)) => PageKind::Mix { refreshable: true },
                _ => PageKind::Playlist,
            });
        }
    }

    /// Home's "For you": each mix's card on the season's paper dyed with the mix (the core's
    /// `mix_card_colours`), or once its first cover's colors are known with those, as the mix's page wears them.
    fn show_picks(&self) {
        use nori_core::mixes::board::{mix_card_colours, mix_tile_colours};
        let prefs = crate::session::app().settings.current().unwrap_or_default();
        let (dark, today) = (self.compositor.system_dark(), crate::look::Today::now());
        let papers = crate::look::papers(&prefs, dark, today);
        let art = self.art.borrow();
        let picks: Vec<Pick> = self
            .mixes
            .iter()
            .map(|t| {
                let card = mix_card_colours(t.id.clone(), papers.light, papers.dark, papers.dark_theme);
                let (page, ink) = match t.covers.first().and_then(|c| art.colours.get(c)) {
                    Some(c) => {
                        let look = crate::look::cover(&prefs, dark, today, c);
                        (look.page(), look.ink())
                    }
                    None => (card.page, card.ink),
                };
                Pick {
                    title: words::mix_name(t.name).into(),
                    sub: words::mix_caption(t.favourites).into(),
                    covers: ModelRc::new(VecModel::from(t.covers.iter().map(SharedString::from).collect::<Vec<_>>())),
                    seed: colour(mix_tile_colours(t.id.clone())[0]),
                    page: colour(page),
                    ink: colour(ink),
                    id: t.id.as_str().into(),
                }
            })
            .collect();
        let ui = self.ui();
        ui.set_picks(ModelRc::new(VecModel::from(picks)));
        ui.set_picks_loaded(true);
    }

    /// Asks whether another device or app left a queue worth carrying on here, as Home comes into sight.
    fn look_for_offer(&self) {
        let Some(s) = self.session.as_ref().filter(|s| s.rules().account) else { return };
        let (client, tx, playing, here) = (s.client.clone(), self.tx.clone(), self.ui().get_playing(), self.heard.clone());
        nori_host::spawn("nori-queue-offer", move || {
            let offer = nori_core::transport::block_on(client.queue_offer(playing, here)).ok().flatten();
            tx.send(Msg::Offer(offer));
        });
    }

    /// Home's Continue card: the offered song, where it stands in its queue, and its cover.
    fn offer_shown(&self) {
        let ui = self.ui();
        let song = self.offer.as_ref().and_then(|o| o.songs.get(o.index as usize).map(|s| (o, s)));
        ui.set_offer_shown(song.is_some());
        if let Some((o, s)) = song {
            ui.set_offer_title(s.title.as_str().into());
            ui.set_offer_line(words::offer_place(&s.artist, o.index as usize + 1, o.songs.len()).into());
            ui.set_offer_art(s.cover_art.clone().unwrap_or_default().into());
        }
    }

    /// The offered queue taken up (played from its song and place) or turned down: either way it is not
    /// offered again.
    fn close_offer(&mut self, take: bool) {
        let (Some(o), Some(s)) = (self.offer.take(), &self.session) else { return };
        s.client.queue_offer_done(o.clone());
        if take {
            s.play(o.songs, o.index as usize, false, None);
            s.seek(o.position_ms as i64);
        }
        self.offer_shown();
    }

    /// The mix shown drawn again, then shown anew.
    fn redraw_mix(&self) {
        let (Some((id, _)), Some(s)) = (self.page_mix.clone(), &self.session) else { return };
        let (client, core, tx) = (s.client.clone(), s.core.clone(), self.tx.clone());
        nori_host::spawn("nori-mix-redraw", move || {
            nori_core::transport::block_on(client.mix_ensure(id.clone(), true));
            if let nori_core::mixes::board::MixLookup::Ready { sheet } = core.mix_page(id.clone()) {
                tx.send(Msg::Data(Req::Page(PageAsk::Mix(id)), Ok(Data::Page(PageGot::Mix(Box::new(sheet))))));
            }
        });
    }

    /// The mix shown kept on the account as a playlist, named by the core (`saved_mix_name`): "nori <mix>",
    /// counted on when that is taken.
    fn save_mix(&self) {
        let (Some((_, title)), Some(s)) = (self.page_mix.clone(), &self.session) else { return };
        let (client, tx) = (s.client.clone(), self.tx.clone());
        let ids: Vec<String> = self.page_songs.iter().map(|s| s.id.clone()).collect();
        nori_host::spawn("nori-mix-save", move || {
            use nori_core::cache_policy::{Page, Read};
            let existing = match nori_core::transport::block_on(client.read_now(Read::PlaylistList)) {
                Ok(Page::Playlists { v }) => v.into_iter().map(|p| p.name).collect(),
                _ => Vec::new(),
            };
            let name = nori_core::menus::saved_mix_name(title, existing);
            let note = match nori_core::transport::block_on(client.write(nori_core::client::Write::CreatePlaylist { name: name.clone(), song_ids: ids })) {
                Ok(_) => Msg::Note { text: words::playlist_created(&name), error: false },
                Err(e) => Msg::Note { text: words::playlist_failed(&words::net_error(&e)), error: true },
            };
            tx.send(note);
        });
    }

    /// The page's Shuffle and Play for the queue it started (the core's `hero_buttons`).
    fn hero_shown(&self) {
        let (Some(fetch), Some(s)) = (&self.page_fetch, &self.session) else { return };
        let ui = self.ui();
        let b = s.page_buttons(&fetch.origin(), ui.get_playing(), ui.get_buffering());
        ui.set_page_shuffle_lit(b.shuffle_lit);
        ui.set_page_pausing(b.pausing);
    }

    /// Songs as rows filtered by the find text; each row keeps its index in `songs`.
    /// The page's songs as listed: an artist's first few until Show all (`songs_shown`).
    fn show_page_songs(&self) {
        let ui = self.ui();
        let all = self.page_songs_all || ui.get_page_kind() != 1;
        let n = nori_core::browse::songs_shown(self.page_songs.len() as u32, all) as usize;
        ui.set_page_songs(self.rows(&self.page_songs[..n]));
        ui.set_page_songs_more(n < self.page_songs.len());
    }

    /// Search's songs as listed: the best few until Show all.
    fn show_search_songs(&self) {
        let ui = self.ui();
        let n = nori_core::browse::songs_shown(self.search_songs.len() as u32, self.search_songs_all) as usize;
        ui.set_search_songs(self.rows(&self.search_songs[..n]));
        ui.set_search_songs_more(n < self.search_songs.len());
    }

    fn rows(&self, songs: &[Song]) -> ModelRc<SongRow> {
        let heard = self.heard.as_deref();
        let find = nori_core::pages::Find::new(&self.find);
        let row = |(i, s): (usize, &Song)| SongRow { by: if self.asked.contains(&s.id) { words::ASKED.into() } else { SharedString::default() }, ..row(s, i, heard == Some(s.id.as_str())) };
        ModelRc::new(VecModel::from(songs.iter().enumerate().filter(|(_, s)| find.keeps_song(s)).map(row).collect::<Vec<_>>()))
    }

    /// Re-applies the find filter to every list.
    fn narrowed(&self) {
        let ui = self.ui();
        let find = nori_core::pages::Find::new(&self.find);
        let pick = |v: &[Card]| cards(v.iter().filter(|c| find.keeps(&[c.title.as_str(), c.sub.as_str()])).cloned());
        let albums: Vec<nori_core::Album> = self.lists.albums.iter().filter(|a| find.keeps(&[&a.name, &a.artist])).cloned().collect();
        if self.album_sort == AlbumSort::ArtistRows {
            let starts = nori_core::browse::artist_rows(albums.clone());
            let ends = starts.iter().skip(1).copied().chain([albums.len() as u32]);
            let rows = starts.iter().zip(ends).map(|(&from, to)| {
                let row = &albums[from as usize..to as usize];
                Shelf { title: row[0].artist.as_str().into(), cards: cards(row.iter().map(album_card)), loaded: true }
            });
            ui.set_album_rows(ModelRc::new(VecModel::from(rows.collect::<Vec<_>>())));
        } else {
            ui.set_album_rows(ModelRc::default());
        }
        ui.set_albums(cards(albums.iter().map(album_card)));
        ui.set_artists(pick(&self.lists.artists));
        ui.set_playlists(pick(&self.lists.playlists));
        ui.set_songs(self.rows(&self.songs));
        self.show_page_songs();
    }

    /// The Albums page's sort menu, the order in use marked.
    fn show_album_sorts(&self) {
        let ui = self.ui();
        let sorts = nori_core::browse::album_sorts();
        ui.set_album_sorts(ModelRc::new(VecModel::from(sorts.iter().map(|&s| SharedString::from(words::album_sort(s))).collect::<Vec<_>>())));
        ui.set_album_sort(sorts.iter().position(|&s| s == self.album_sort).unwrap_or(0) as i32);
        ui.set_albums_in_rows(self.album_sort == AlbumSort::ArtistRows);
    }

    /// Reads the albums again in the order chosen in the sort menu, and keeps it.
    fn sort_albums(&mut self, at: usize) {
        let Some(&sort) = nori_core::browse::album_sorts().get(at) else { return };
        if sort == self.album_sort {
            return;
        }
        let settings = &crate::session::app().settings;
        let mut prefs = settings.current().unwrap_or_default();
        let kept = nori_core::browse::album_sort_kept(sort);
        prefs.list_prefs.insert(kept.key, kept.value);
        settings.put(prefs);
        self.album_sort = sort;
        self.albums_asked = 0;
        self.lists.albums.clear();
        self.narrowed();
        self.show_album_sorts();
        self.load(Req::Page(PageAsk::Albums { sort, offset: 0 }));
    }

    /// Shows an artist beside the Artists list.
    fn choose_artist(&mut self, id: String) {
        let ui = self.ui();
        ui.set_artist_chosen(id.as_str().into());
        ui.set_page_kind(1);
        ui.set_page_id(id.as_str().into());
        ui.set_page_title("".into());
        ui.set_page_caption("".into());
        ui.set_page_shelves(ModelRc::default());
        self.page_songs.clear();
        self.page_songs_all = false;
        self.page_fetch = Some(Fetch::Artist(id.clone()));
        self.load(Req::Page(PageAsk::Artist(id)));
        // Keep the list visible while the artist loads.
        ui.set_loading(false);
    }

    /// Refreshes the now-playing marks in the lists.
    fn mark_playing(&self) {
        let ui = self.ui();
        self.show_page_songs();
        self.show_search_songs();
        if !self.songs.is_empty() {
            ui.set_songs(self.rows(&self.songs));
        }
    }

    fn more_songs(&mut self) {
        let req = Req::Page(PageAsk::Songs { offset: self.songs.len() as u32 });
        // Scrolling asks again as the list nears its end: the page already on its way is asked for once.
        if self.ui().get_loading() && self.want.as_ref() == Some(&req) {
            return;
        }
        self.load(req);
    }

    fn search_edited(&mut self, text: &str) {
        let Some(s) = &self.session else { return };
        let view = s.search_typed(text);
        let query = view.query.clone();
        self.show_search(view);
        let delay = nori_core::search::live_delay_ms(&query, nori_core::search::LIVE_SEARCH_DELAY_MS) as u64;
        let me = self.me.clone();
        self.search.start(TimerMode::SingleShot, Duration::from_millis(delay), move || {
            me.with(|a| a.on_session(|s| s.search_server(query.clone())));
        });
    }

    /// A recent search clicked: it is typed in the top pill's field and searched again.
    fn search_again(&mut self, text: &str) {
        if let Some(top) = &self.top {
            top.set_search_text(text.into());
        }
        self.search_edited(text);
    }

    fn forget_searches(&mut self) {
        if let Some(s) = &self.session {
            let _ = s.core.search_forget();
        }
        self.ui().set_recent_searches(ModelRc::default());
    }

    fn show_search(&mut self, v: SearchView) {
        let ui = self.ui();
        // Nothing typed: the searches made before, newest first.
        let blank = v.query.trim().is_empty();
        ui.set_search_blank(blank);
        ui.set_search_nothing(v.nothing_found && !v.searching && v.error.is_none());
        ui.set_search_query(v.query.trim().into());
        if blank {
            let recent = self.session.as_ref().and_then(|s| s.core.search_history().ok()).unwrap_or_default();
            ui.set_recent_searches(ModelRc::new(VecModel::from(recent.into_iter().map(SharedString::from).collect::<Vec<_>>())));
        }
        let found = v.shown.unwrap_or_default();
        ui.set_search_artists(cards(found.artists.iter().map(artist_card)));
        ui.set_search_albums(cards(found.albums.iter().map(album_card)));
        ui.set_search_playlists(cards(found.playlists.iter().map(playlist_card)));
        self.search_songs = found.songs;
        if self.search_query != v.query.trim() {
            self.search_query = v.query.trim().to_string();
            self.search_songs_all = false;
        }
        self.show_search_songs();
        ui.set_search_busy(v.searching);
        let note = if let Some(e) = v.error {
            format!("The server could not be searched ({}); these are from the offline index", e.reason.unwrap_or_default())
        } else {
            String::new()
        };
        ui.set_search_note(note.into());
    }

    fn play_fetch(&self, what: Fetch) {
        self.on_session(|s| s.play_later(what, false));
    }

    /// Card `id` of `kind` (`Card::kind`, or 4 a mix) next or at the end of the queue; a song card, the
    /// song alone.
    fn queue_card(&self, kind: i32, id: String, next: bool) {
        let what = match kind {
            0 => Fetch::Album(id),
            2 => Fetch::Playlist(id),
            3 => {
                if let Some(song) = self.feed_songs.iter().flatten().find(|s| s.id == id) {
                    self.on_session(|s| s.enqueue(vec![song.clone()], next));
                }
                return;
            }
            4 => Fetch::Mix(id),
            _ => return,
        };
        self.on_session(|s| s.enqueue_later(what, next));
    }

    fn list_song(&self, list: i32, i: usize) -> Option<Song> {
        match list {
            0 => &self.page_songs,
            1 => &self.songs,
            _ => &self.search_songs,
        }
        .get(i)
        .cloned()
    }

    /// Song `i` of `list`'s menu, made as it opens.
    fn song_menu(&self, list: i32, i: usize) -> ModelRc<SongGo> {
        self.list_song(list, i).map_or_else(ModelRc::default, |one| self.menu_lines(one, false))
    }

    /// The menu of the song playing, with the player's lines.
    fn player_menu(&self) -> ModelRc<SongGo> {
        self.song.clone().map_or_else(ModelRc::default, |one| self.menu_lines(one, true))
    }

    /// `one`'s menu as the core lists it (menus.rs's `song_menu`); `player`: opened from the player.
    fn menu_lines(&self, one: Song, player: bool) -> ModelRc<SongGo> {
        use nori_core::menus::{song_menu, JamOffer, SongAction as A, SongDownload};
        let Some(s) = self.session.as_ref() else { return ModelRc::default() };
        let ui = self.ui();
        let jam = ui.global::<crate::Jam>();
        let offer = if s.rules().asks { Some(JamOffer::Guest) } else { (jam.get_on() && !jam.get_hosting()).then_some(JamOffer::Start) };
        let starred = self.heart(&one);
        let download = if s.rules().downloads { s.download_state(&one.id) } else { SongDownload::None };
        let lyrics = player && self.lyrics.as_ref().is_some_and(|l| !l.is_empty());
        let items = song_menu(one, starred.unwrap_or_default(), download, player, lyrics, starred.is_some(), s.rules().account, s.rules().downloads, s.rules().radio, offer);
        let lines = items.into_iter().map(|m| {
            let (kind, id) = match &m.action {
                A::Favourite { .. } => (0, String::new()),
                A::GoToAlbum { id } => (1, id.clone()),
                A::GoToArtist { id, .. } => (2, id.clone()),
                A::Download => (3, String::new()),
                A::StartRadio => (4, String::new()),
                A::PlayNext => (5, String::new()),
                A::AddToQueue => (6, String::new()),
                A::AddToPlaylist => (7, String::new()),
                A::RemoveDownload => (8, String::new()),
                A::StopDownload => (9, String::new()),
                A::AddToLibrary => (10, String::new()),
                A::InstantMix => (11, String::new()),
                A::ExcludeFromMixes => (12, String::new()),
                A::Share => (13, String::new()),
                A::Details => (14, String::new()),
                A::StartJam => (15, String::new()),
                A::Lyrics => (16, String::new()),
                A::SleepTimer => (17, String::new()),
            };
            let filled = matches!(m.action, A::Favourite { on: false });
            SongGo { title: words::song_action(&m.action).into(), kind, id: id.into(), more: m.more, enabled: m.enabled, filled }
        });
        ModelRc::new(VecModel::from(lines.collect::<Vec<_>>()))
    }

    /// Plays song `i` of `list` (`how` -1), or does its menu's line `how` (`SongGo::kind`).
    fn song(&mut self, list: i32, i: usize, how: i32) {
        let Some(one) = self.list_song(list, i) else { return };
        let (songs, origin) = match list {
            0 => (self.page_songs.clone(), self.page_fetch.as_ref().map(Fetch::origin)),
            1 => (self.songs.clone(), None),
            _ => (self.search_songs.clone(), None),
        };
        match how {
            -1 => self.on_session(|s| s.play(songs, i, false, origin)),
            15 => self.on_session(|s| s.start_jam(songs, i)),
            _ => self.song_act(one, how),
        }
    }

    /// The player's menu line `how` (`SongGo::kind`), for the song playing.
    fn player_act(&mut self, how: i32) {
        match how {
            15 => self.on_session(|s| s.jam_open()),
            16 => self.lyrics_sheet(true),
            17 => {
                self.sleep_choices = nori_core::menus::sleep_choices(self.sleeping);
                let ui = self.ui();
                ui.set_sleep_choices(ModelRc::new(VecModel::from(self.sleep_choices.iter().map(|c| SharedString::from(words::sleep_choice(c))).collect::<Vec<_>>())));
                ui.invoke_show_sleep();
            }
            _ => {
                if let Some(one) = self.song.clone() {
                    self.song_act(one, how);
                }
            }
        }
    }

    /// Menu line `how` (`SongGo::kind`) for `one`, wherever its menu opened.
    fn song_act(&mut self, one: Song, how: i32) {
        match how {
            0 => self.star(&one),
            3 => self.on_session(|s| s.download(vec![one])),
            4 => self.on_session(|s| s.play_later(Fetch::Radio(one.id), false)),
            5 | 6 => self.on_session(|s| s.enqueue(vec![one], how == 5)),
            7 => {
                let ui = self.ui();
                ui.set_picker_playlists(cards(self.lists.playlists.iter().cloned()));
                ui.set_picker_name("".into());
                ui.set_picker_open(true);
                self.menu_song = Some(one);
            }
            8 | 9 => self.on_session(|s| s.download_remove(&one.id)),
            10 => self.on_session(|s| s.star(nori_core::client::Starrable::Song, one.id, true)),
            11 => self.on_session(|s| s.play_later(Fetch::InstantMix(one.id), false)),
            12 => {
                let done = self.session.as_ref().is_some_and(|s| s.core.mix_excluded_set(one.id, true).is_ok());
                if done {
                    self.say(words::EXCLUDED_FROM_MIXES, false);
                }
            }
            13 => self.share(one.id),
            14 => {
                let ui = self.ui();
                let rows = words::details(&one).into_iter().map(|(label, value)| Detail { label: label.into(), value: value.into() });
                ui.set_details(ModelRc::new(VecModel::from(rows.collect::<Vec<_>>())));
                ui.set_details_open(true);
            }
            _ => {}
        }
    }

    /// The sleep timer's choice `i`: a time counts down here; a number of songs, or this one's end, is
    /// the queue's to count (`sleep_set`).
    fn sleep(&mut self, i: usize) {
        let Some(c) = self.sleep_choices.get(i).copied() else { return };
        self.sleep_timer.stop();
        let pause = self.session.as_ref().is_some_and(|s| s.core.session.sleep_set(c.songs, c.end_of_track));
        self.on_session(|s| s.engine.pause_at_end(pause));
        self.sleeping = c.minutes > 0 || c.songs > 0 || c.end_of_track;
        if c.minutes > 0 {
            let delay = nori_core::rules::sleep_delay(c.minutes)[0];
            let weak = self.me.clone();
            self.sleep_timer.start(TimerMode::SingleShot, Duration::from_millis(delay.max(0) as u64), move || {
                weak.with(|a| {
                    a.sleeping = false;
                    a.on_session(|s| s.engine.pause());
                })
            });
        }
    }

    /// Fills the lyrics' sheet from the lyrics shown, opening it when `open`.
    fn lyrics_sheet(&self, open: bool) {
        let Some(l) = &self.lyrics else { return };
        let pick = l.pick();
        let ui = self.ui();
        ui.set_lyrics_from(words::lyrics_from(pick.origin, pick.lyrics.synced, pick.lyrics.word_timed).into());
        ui.set_lyrics_timing(words::lyrics_timing(pick.lyrics.timing).into());
        ui.set_lyrics_shift(words::lyrics_shift(pick.lyrics.timing.offset_ms()).into());
        ui.set_lyrics_yours(matches!(pick.lyrics.timing, nori_core::LyricsTiming::Yours { .. }));
        if open {
            ui.set_lyrics_sheet_open(true);
        }
    }

    /// The lyrics shown a step sooner (`dir` > 0), later (< 0) or back to the check's timing (0).
    fn lyrics_nudge(&mut self, dir: i32) {
        let (Some(l), Some(s), Some(id)) = (&self.lyrics, &self.session, self.heard.clone()) else { return };
        let pick = s.core.lyrics_nudge(id, l.pick().clone(), dir);
        self.show_lyrics(pick);
        self.lyrics_sheet(false);
    }

    /// Shows `pick` as the song playing's lyrics.
    fn show_lyrics(&mut self, pick: nori_core::race::LyricsPick) {
        let l = crate::lyrics::SongLyrics::new(pick, self.position_now(), self.lyric_face.clone());
        let ui = self.ui();
        ui.set_lyrics_lines(l.lines());
        ui.set_lyrics_synced(l.synced());
        ui.set_lyrics_note(if l.is_empty() { "No lyrics for this song".into() } else { l.credit().into() });
        ui.set_lyrics_waiting(false);
        self.lyrics = Some(l);
        self.lyrics_step(true);
    }

    /// Asks the server for a link to song `id`, copied when it comes.
    fn share(&self, id: String) {
        let Some(client) = self.session.as_ref().map(|s| s.client.clone()) else { return };
        let tx = self.tx.clone();
        nori_host::spawn("nori-share", move || {
            let link = match nori_core::transport::block_on(client.read_now(nori_core::cache_policy::Read::ShareLink { id })) {
                Ok(nori_core::cache_policy::Page::ShareUrl { v }) => Some(v),
                _ => None,
            };
            tx.send(Msg::Shared(link));
        });
    }

    /// Adds the song whose menu asked to playlist `id`, or to a new one named `name`.
    fn add_to_playlist(&mut self, to: Result<String, String>) {
        let ui = self.ui();
        ui.set_picker_open(false);
        let (Some(song), Some(client)) = (self.menu_song.take(), self.session.as_ref().map(|s| s.client.clone())) else { return };
        let (write, done) = match to {
            Ok(id) => {
                let name = self.lists.playlists.iter().find(|p| p.id == id.as_str()).map(|p| p.title.to_string()).unwrap_or_default();
                (nori_core::client::Write::AddToPlaylist { id, song_ids: vec![song.id] }, words::added_to_playlist(&name))
            }
            Err(name) => {
                let name = name.trim().to_string();
                (nori_core::client::Write::CreatePlaylist { name: name.clone(), song_ids: vec![song.id] }, words::playlist_created(&name))
            }
        };
        let tx = self.tx.clone();
        nori_host::spawn("nori-playlist-add", move || {
            let note = match nori_core::transport::block_on(client.write(write)) {
                Ok(_) => Msg::Note { text: done, error: false },
                Err(e) => Msg::Note { text: words::playlist_failed(&words::net_error(&e)), error: true },
            };
            tx.send(note);
        });
    }

    fn play_page(&self, shuffle: bool) {
        let Some(fetch) = self.page_fetch.clone() else { return };
        if shuffle && self.session.as_ref().is_some_and(|s| s.shuffle_page(&fetch.origin())) {
            return;
        }
        // The queue this page started: Play pauses and resumes it.
        let ui = self.ui();
        if !shuffle && self.session.as_ref().is_some_and(|s| s.page_buttons(&fetch.origin(), ui.get_playing(), ui.get_buffering()).play_press == nori_core::pages::HeroPress::Toggle) {
            self.on_session(|s| s.toggle());
            return;
        }
        // An artist's Play is all its songs, not the few its page lists.
        if self.page_songs.is_empty() || matches!(fetch, Fetch::Artist(_)) {
            self.on_session(|s| s.play_later(fetch, shuffle));
        } else {
            let origin = fetch.origin();
            self.on_session(|s| s.play(self.page_songs.clone(), 0, shuffle, Some(origin)));
        }
    }

    fn seek(&mut self, fraction: f32) {
        let Some(song) = &self.song else { return };
        let ms = (fraction as f64 * song.duration as f64 * 1000.0) as i64;
        position_shown(&self.ui(), self.player.as_ref(), ms as i32);
        self.seek_to(ms);
    }

    /// Seeks to `ms`; the lyrics show that place at once.
    fn seek_to(&mut self, ms: i64) {
        self.on_session(|s| s.seek(ms));
        self.lyrics_step(true);
    }

    /// The account's device playing while it is another one, which the controls reach.
    fn there(&self) -> Option<&Elsewhere> {
        match &self.playback {
            Playback::There(e) => Some(e),
            _ => None,
        }
    }

    /// Whether `s` shows starred: as the device playing shows it, while that is another one.
    fn starred(&self, s: &Song) -> bool {
        self.session.as_ref().is_some_and(|x| x.starred(s, self.there()))
    }

    /// Flips the album's or artist's seal on the page shown.
    fn star_page(&self) {
        let ui = self.ui();
        let kind = match ui.get_page_kind() {
            0 => nori_core::client::Starrable::Album,
            1 => nori_core::client::Starrable::Artist,
            _ => return,
        };
        let on = !ui.get_page_starred();
        ui.set_page_starred(on);
        let id = ui.get_page_id().to_string();
        self.on_session(|s| s.star(kind, id, on));
    }

    /// Flips `s`'s heart, once: through the device playing while it has the song, else here.
    fn star(&mut self, s: &Song) {
        if self.session.as_ref().is_some_and(|x| !x.rules().stars && x.rules().stars_after_sign_in) {
            return self.offer_sign_in();
        }
        let on = !self.starred(s);
        self.on_session(|x| x.star_song(s.id.clone(), on, self.there()));
    }

    /// Where the song shown is now: here, on the device playing or in the jam.
    fn position_now(&self) -> i64 {
        match (&self.playback, &self.session) {
            (Playback::There(e) | Playback::Jam(Some(e)), _) => e.position_ms(),
            (Playback::Here, Some(s)) => s.engine.status_with(|st| st.position_now()),
            _ => 0,
        }
    }

    /// The open profile is a jam guest's.
    fn guest(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.guest)
    }

    /// Whether the player offers `control`: a jam guest's by its role (the core's jam controls), this
    /// computer's own player all of them.
    fn offers(&self, control: fn(&nori_core::remote::Controls) -> nori_core::remote::Reach) -> bool {
        !self.guest() || self.jam_controls.is_some_and(|c| control(&c.controls) != nori_core::remote::Reach::Nowhere)
    }

    /// Whether the open profile has the account's things (hearts, playlists, settings): a jam guest's has
    /// not (the core's `ProfileRules`).
    fn account(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.rules().account)
    }

    /// `s`'s heart as the lists show it; none where songs carry no hearts (`ProfileRules::stars`), or one
    /// that asks to sign in.
    fn heart(&self, s: &Song) -> Option<bool> {
        self.session.as_ref().is_some_and(|x| x.rules().stars || x.rules().stars_after_sign_in).then(|| self.starred(s))
    }

    /// Joins the jam `link` invites to, as a guest profile of its own.
    /// Signs the account of the profile in use in with cookies pasted from a browser.
    fn sign_in(&mut self, answer: String) {
        let Some(s) = &self.session else { return };
        let ui = self.ui();
        if answer.trim().is_empty() || ui.get_signin_busy() {
            return;
        }
        ui.set_signin_busy(true);
        ui.set_signin_error("".into());
        let (h, tx) = ((***s).clone(), self.tx.clone());
        nori_host::spawn("nori-sign-in", move || tx.send(Msg::SignedIn(h.sign_in(answer))));
    }

    /// The account sheet of the profile in use, as its server's Edit and a seal tapped while signed out open
    /// it: who is signed in, or Sign in (the provider's own page in a window) with pasting folded under it.
    /// A server row's Edit: the profile in use's account sheet where its provider signs in to one, else
    /// the profile in the sign-in form, saved over itself.
    fn edit_profile(&mut self, id: String) {
        let prefs = crate::session::app().settings.current().unwrap_or_default();
        let Some(profile) = prefs.servers.iter().find(|s| s.id == id).cloned() else { return };
        let providers = nori_core::providers::providers();
        if id == prefs.active_server_id && providers.iter().any(|i| i.kind == profile.kind && !i.sign_in.is_empty()) {
            return self.offer_sign_in();
        }
        let ui = self.ui();
        ui.set_login_kind(providers.iter().position(|i| i.kind == profile.kind).unwrap_or(0) as i32);
        ui.set_login_name(profile.name.as_str().into());
        ui.set_login_url(profile.url.as_str().into());
        ui.set_login_user(profile.user.as_str().into());
        ui.set_login_password("".into());
        self.editing = Some(profile);
        self.go(LOGIN);
    }

    /// The account changed under the open profile: its seals and downloads follow, and a session the
    /// source ended asks to sign in again.
    fn account_changed(&mut self) {
        let Some(s) = self.session.as_ref() else { return };
        let Some(state) = s.account_changed() else { return };
        let rules = s.rules();
        let ui = self.ui();
        ui.set_downloads(rules.downloads);
        ui.set_page_can_star(rules.account && rules.stars);
        match state {
            nori_core::providers::AccountState::Ended => self.offer_sign_in(),
            nori_core::providers::AccountState::SignedIn => ui.set_signin_open(false),
            nori_core::providers::AccountState::SignedOut => {}
        }
    }

    /// The ended session's sheet no longer comes back at start.
    fn sign_in_quiet(&self) {
        self.on_session(|s| s.client.stop_reminding());
        self.ui().set_signin_open(false);
    }

    fn offer_sign_in(&mut self) {
        use nori_core::providers::AccountState;
        let ended = self.session.as_ref().is_some_and(|s| s.client.account() == AccountState::Ended);
        let user = crate::session::app().settings.current().and_then(|p| p.active_server().map(|sv| sv.user.clone())).filter(|_| !ended).unwrap_or_default();
        let ui = self.ui();
        ui.set_signin_user(user.into());
        ui.set_signin_pasting(false);
        ui.set_signin_error("".into());
        ui.set_signin_ended(ended);
        ui.set_signin_open(true);
    }

    /// The provider's own sign-in page in a window of its own (this program again, `--sign-in`: the system's
    /// web view); where that cannot start, the sheet unfolds pasting with why.
    fn sign_in_window(&mut self) {
        let Some(s) = &self.session else { return };
        let Some(kind) = crate::session::app().settings.current().and_then(|p| p.active_server().map(|sv| sv.kind)) else { return };
        if self.sign_in_window.as_mut().is_some_and(|w| w.open_now()) {
            return;
        }
        let (h, tx) = ((***s).clone(), self.tx.clone());
        let window = nori_sign_in_window::SignInWindow::open(kind, format!("Sign in to {}", words::provider(kind).name), move |id, cookies| {
            tx.send(Msg::SignInWindowDone(id, h.sign_in(cookies)))
        });
        let ui = self.ui();
        let Some(window) = window else {
            ui.set_signin_pasting(true);
            ui.set_signin_error(words::SIGN_IN_WINDOW_FAILED.into());
            return;
        };
        ui.set_signin_busy(true);
        self.sign_in_window = Some(window);
    }

    fn sign_out(&mut self) {
        self.on_session(|s| s.sign_out());
        self.ui().set_signin_open(false);
        self.reopen();
    }

    /// Opens the profile in use again, as the settings now keep it (its account signed in or out), on the
    /// page it was on.
    fn reopen(&mut self) {
        if let Some(p) = crate::session::app().settings.current().and_then(|p| p.active_server().cloned()) {
            let view = self.ui().get_view();
            self.open(p);
            self.go(view);
        }
    }

    fn join_jam(&mut self, link: String) {
        let ui = self.ui();
        if nori_core::remote::pairing_valid(link.clone()) {
            let paired = self.session.as_ref().and_then(|s| s.remote()).is_some_and(|r| r.pair(link));
            ui.set_join_open(!paired);
            if !paired { ui.set_join_error(words::PAIR_FAILED.into()); }
            self.devices_shown();
            return;
        }
        if !nori_core::remote::is_invite(&link) {
            return ui.set_join_error(words::NOT_AN_INVITE.into());
        }
        ui.set_join_busy(true);
        ui.set_join_error("".into());
        let (http, tx, remote) = (self.http.clone(), self.tx.clone(), self.session.as_ref().and_then(|s| s.remote()));
        nori_host::spawn("nori-jam-join", move || {
            let joined = nori_host::jam_join(http, &session::app().settings, link, &nori_host::device_name(), remote);
            tx.send(Msg::Joined(joined.map_err(|e| words::jam_join_failed(&e))))
        });
    }

    /// The guest profile joined with: opened in place of the user's own, which is opened again on leaving.
    fn joined(&mut self, pass: nori_core::remote::JamPass) {
        let ui = self.ui();
        ui.set_join_busy(false);
        ui.set_join_open(false);
        ui.set_join_link("".into());
        let guest = nori_host::jam_joined(&session::app().settings, pass, words::JAM_GUEST);
        self.open(guest);
    }

    /// Leaves the jam this guest is in, at once; the relay is told on the way.
    fn leave_jam(&mut self) {
        if let Some(r) = self.session.as_ref().and_then(|s| s.remote()) {
            r.jam_leave();
        }
        self.left(words::JAM_LEFT);
    }

    /// The guest profile is dropped, and the user's own opened again; `said` says why.
    fn left(&mut self, said: &str) {
        let back = nori_host::jam_left(&session::app().settings);
        self.playback = Playback::Here;
        self.asked.clear();
        match back {
            Some(p) => self.open(p),
            None => {
                if let Some(s) = self.session.take() {
                    s.close();
                }
                self.go(LOGIN);
            }
        }
        self.say(said, false);
    }

    fn take(&mut self, m: Msg) {
        match m {
            Msg::Engine(e) => {
                match &e {
                    // A seek landed: the lyrics follow from the new place.
                    Event::Position { .. } => self.lyrics_step(true),
                    // The output playing: the equalizer page suggests its headphones' AutoEQ curves.
                    Event::Output { name } => {
                        self.output = name.clone();
                        self.output_shown();
                        if self.ui().get_view() == EQUALIZER {
                            self.eq_extras();
                        }
                    }
                    Event::Buffering(b) if matches!(self.playback, Playback::Here) => {
                        self.ui().set_buffering(*b);
                        self.advancing();
                    }
                    Event::Error { message, .. } => self.say(&format!("Could not play: {message}"), true),
                    Event::Waiting(true) => self.say("Waiting for the server…", true),
                    // A sleep timer of songs, or this one's end, stops playback by itself.
                    Event::Stopped { .. } if self.sleeping && !self.sleep_timer.running() => self.sleeping = false,
                    _ => {}
                }
                self.follow();
            }
            Msg::Data(req, r) => {
                self.data(req, r);
                self.mirror();
                self.account_changed();
            }
            Msg::Cover { key, image, colours } => self.cover(key, &image, colours),
            Msg::Search(v) => {
                if self.ui().get_view() == SEARCH {
                    self.show_search(v);
                }
            }
            Msg::Lyrics { song, pick } => {
                if self.heard.as_deref() == Some(song.as_str()) && self.lyrics.as_ref().is_none_or(|l| l.replaced_by(&pick)) {
                    self.show_lyrics(pick);
                }
            }
            Msg::Facts(f) => {
                self.facts = *f;
                self.settings_shown();
            }
            Msg::Note { text, error } => self.say(&text, error),
            Msg::Shared(Some(link)) => self.copy(link),
            Msg::Offer(offer) => {
                let (playing, here) = (self.ui().get_playing(), self.heard.clone());
                self.offer = offer.filter(|o| o.songs.get(o.index as usize).is_some_and(|s| nori_core::library::offer_stands(&s.id, here, playing)));
                self.offer_shown();
            }
            Msg::Shared(None) => self.say(words::SHARE_FAILED, true),
            Msg::Ask { text, action } => self.ask(&text, &action),
            Msg::Reachable(Err(e)) => self.say(&e, true),
            Msg::Reachable(Ok(())) | Msg::From(..) => {}
            Msg::Remote => {
                self.playback_read();
                self.devices_shown();
                self.jam_shown();
                // Another device or the jam may have changed the queue (a song accepted, added from afar).
                self.follow();
            }
            Msg::Jam(Ok(())) => {
                // The jam shows in the queue panel.
                self.ui().set_inspector(1);
                self.jam_shown();
            }
            Msg::Jam(Err(e)) => self.say(&format!("{} ({e})", words::JAM_FAILED), true),
            Msg::Joined(Ok(pass)) => self.joined(pass),
            Msg::Joined(Err(e)) => {
                let ui = self.ui();
                ui.set_join_busy(false);
                ui.set_join_error(e.into());
            }
            Msg::JamEnded(host) => self.left(&words::jam_ended(host.as_deref())),
            Msg::Starred => {
                self.on_session(|s| s.mpris_changed());
                self.follow();
                self.mark_playing();
                // The Favorites tab lists what is liked now: a like from the player or a row joins or leaves it.
                if self.ui().get_view() == FAVORITES {
                    self.load(Req::Page(PageAsk::Favorites));
                    // Read again in place: the list stays while it comes.
                    self.ui().set_loading(false);
                }
                if let Some(s) = &self.session {
                    let core = s.core.clone();
                    let taste = crate::session::app().settings.prefs(|p| p.taste_model);
                    let id = nori_core::mixes::board::FAVOURITES_MIX;
                    if let nori_core::mixes::board::MixLookup::Ready { sheet } = core.mix_page(id.into()) {
                        self.data(Req::Page(PageAsk::Mix(id.into())), Ok(Data::Page(PageGot::Mix(Box::new(sheet)))));
                    }
                    let home = Req::Home(self.home_rows.clone());
                    self.data(home, Ok(Data::Home(HomePart::Mixes(core.mix_cards(taste)))));
                }
            }
            Msg::Volume(v) if self.there().is_none() => self.ui().set_volume(v),
            Msg::Volume(_) => {}
            Msg::AutoEqCurve(name, curve) => {
                let ui = self.ui();
                ui.set_eq_autoeq_busy(false);
                let note = match curve {
                    Ok(Some(text)) => match crate::session::app().settings.sound_tool(nori_core::settings_store::SoundTool::Import { text }) {
                        Ok(change) => {
                            if let Some(change) = change {
                                self.on_session(|s| s.applied(change.effect));
                                self.tune();
                            }
                            words::autoeq_applied(&name)
                        }
                        Err(e) => format!("{e:?}"),
                    },
                    Ok(None) => words::AUTOEQ_NO_CURVE.into(),
                    Err(e) => e,
                };
                ui.set_eq_autoeq_note(note.into());
                self.settings_shown();
            }
            Msg::AutoEqList(done) => {
                let ui = self.ui();
                ui.set_eq_autoeq_busy(false);
                if let Err(e) = done {
                    ui.set_eq_autoeq_note(e.into());
                }
                self.settings_shown();
            }
            Msg::SignInWindowDone(pid, r) => {
                self.sign_in_window.take_if(|w| w.id() == pid);
                let ui = self.ui();
                ui.set_signin_busy(false);
                match r {
                    nori_core::providers::SignInResult::SignedIn { .. } => {
                        ui.set_signin_open(false);
                        self.reopen();
                    }
                    failed => ui.set_signin_error(words::sign_in_failed(&failed).into()),
                }
            }
            Msg::SignedIn(r) => {
                let ui = self.ui();
                ui.set_signin_busy(false);
                match r {
                    nori_core::providers::SignInResult::SignedIn { .. } => {
                        ui.set_signin_open(false);
                        ui.set_signin_text("".into());
                        self.reopen();
                    }
                    failed => ui.set_signin_error(words::sign_in_failed(&failed).into()),
                }
            }
            Msg::LoggedIn(r) => {
                let ui = self.ui();
                ui.set_login_busy(false);
                match r {
                    Ok(p) => {
                        self.editing = None;
                        ui.set_login_password("".into());
                        self.open(p);
                    }
                    Err(e) => ui.set_login_error(e.into()),
                }
            }
        }
    }

    /// Requests the covers found missing while drawing.
    fn ask_covers(&mut self) {
        let mut art = self.art.borrow_mut();
        let a = &mut *art;
        let wanted = std::mem::take(&mut a.wanted);
        let Some(s) = &self.session else {
            // No session yet: forget them so they are requested again later.
            a.asked.clear();
            return;
        };
        // Cancel requests for covers no longer on screen.
        let now = Instant::now();
        self.tickets.retain(|(k, _)| {
            let shown = a.missing.get(k).is_some_and(|t| now.duration_since(*t) < GONE);
            if !shown {
                a.asked.remove(k);
                a.missing.remove(k);
            }
            shown
        });
        // The loader serves newest first; reversed so the top left of a frame comes first.
        for k in wanted.into_iter().rev() {
            // A mix card's first cover brings its colors, which the card wears.
            let colours = k.size != CoverSize::Card || self.mixes.iter().any(|m| m.covers.first() == Some(&k.id));
            let Some(t) = s.cover(k.clone(), cover_px(k.size), colours) else { continue };
            self.tickets.push_back((k, t));
        }
        // Dropping a ticket cancels it; the cover may be requested again.
        while self.tickets.len() > PENDING_KEPT {
            let (k, _) = self.tickets.pop_front().expect("longer than the limit");
            a.asked.remove(&k);
            a.missing.remove(&k);
        }
    }

    fn cover(&mut self, key: CoverKey, image: &Picture, colours: Option<Box<CoverColours>>) {
        let id = key.id.clone();
        let large = key.size != CoverSize::Card;
        self.tickets.retain(|(k, _)| *k != key);
        let picture = self.compositor.picture(&image.pixels, image.width, image.height);
        self.art.borrow_mut().insert(key, picture, colours, Instant::now());
        let ui = self.ui();
        ui.set_covers_rev(ui.get_covers_rev().wrapping_add(1));
        if let Some(p) = &self.player {
            p.set_covers_rev(ui.get_covers_rev());
        }
        if large {
            let c = self.art.borrow().colours.get(&id).cloned();
            if ui.get_now_art() == id.as_str() {
                self.now_colours(&id, c.as_deref());
            }
            if ui.get_view() == PAGE && ui.get_page_art() == id.as_str() {
                self.page_colours(c.as_deref());
            }
        }
        if self.mixes.iter().any(|m| m.covers.first() == Some(&id)) && self.art.borrow().colours.contains_key(&id) {
            self.show_picks();
        }
    }

    /// Copies the main window's state to the pills' windows.
    fn mirror(&self) {
        let ui = self.ui();
        if let Some(p) = &self.player {
            p.set_has_song(ui.get_has_song());
            p.set_now_title(ui.get_now_title());
            p.set_now_artist(ui.get_now_artist());
            p.set_now_album(ui.get_now_album());
            p.set_now_artist_id(ui.get_now_artist_id());
            p.set_now_album_id(ui.get_now_album_id());
            p.set_now_credits(ui.get_now_credits());
            p.set_now_starred(ui.get_now_starred());
            p.set_now_art(ui.get_now_art());
            p.set_playing(ui.get_playing());
            p.set_buffering(ui.get_buffering());
            p.set_position_ms(ui.get_position_ms());
            p.set_duration_ms(ui.get_duration_ms());
            p.set_live(ui.get_live());
            p.set_shuffle(ui.get_shuffle());
            p.set_repeat(ui.get_repeat());
            p.set_volume(ui.get_volume());
            p.set_inspector(ui.get_inspector());
            p.set_covers_rev(ui.get_covers_rev());
            p.set_devices_on(ui.get_devices_on());
            p.set_jam(ui.global::<crate::Jam>().get_strip());
            p.set_guest(ui.global::<crate::Jam>().get_guest());
            p.set_can_play(ui.global::<crate::Jam>().get_can_play());
            p.set_can_skip(ui.global::<crate::Jam>().get_can_skip());
            p.set_can_seek(ui.global::<crate::Jam>().get_can_seek());
            p.set_playing_on(ui.get_playing_on());
        }
        if let Some(t) = &self.top {
            t.set_view(ui.get_view());
            t.set_page_kind(ui.get_page_kind());
            t.set_guest(ui.global::<crate::Jam>().get_guest());
        }
    }

    /// Positions the pills; hidden over Now Playing and the login page.
    fn place_player(&self) {
        let ui = self.ui();
        let shown = !ui.get_full_player() && ui.get_view() != LOGIN;
        self.compositor.show_pills(shown && self.top.is_some(), shown && self.player.is_some());
        let chrome = ui.global::<crate::Chrome>();
        let (top, player) = self.compositor.pills();
        chrome.set_top_x(top.0);
        chrome.set_top_w(top.1);
        chrome.set_player_x(player.0);
        chrome.set_player_w(player.1);
        if let Some(t) = &self.top {
            t.set_room(top.1);
        }
        if let Some(p) = &self.player {
            p.set_room(player.1);
        }
    }

    /// Updates the lyrics view from the playback position and schedules the next update.
    fn lyrics_step(&mut self, force: bool) {
        let ui = self.ui();
        let at = self.position_now();
        let playing = ui.get_playing();
        let Some(l) = &mut self.lyrics else { return };
        let full = ui.get_full_player() && ui.get_full_panel() == 2;
        let side = ui.get_inspector() == 2;
        let now = l.advance(at, full || side, force);
        ui.set_lyrics_active(now.active);
        ui.set_lyric_sweeping(now.sweeping);
        let width = ui.window().size().to_logical(ui.window().scale_factor()).width;
        let mut pieces = |view, on: bool, size, w, lit, dim| if on && now.sweeping { l.pieces(view, &now, size, w, lit, dim) } else { Vec::new() };
        // Every line is drawn in the rows the sung pieces take, a hair narrower than the view so the
        // window's own wrapping never breaks them again.
        let (side_w, full_w) = (320.0 - 44.0 - 2.0, width / 2.0 - 140.0 - 2.0);
        renew(&self.pieces[0], pieces(0, side, 22.0, side_w, 1.0, 0.36));
        renew(&self.pieces[1], pieces(1, full, 44.0, full_w, 1.0, 0.36));
        if let Some(rows) = side.then(|| l.rows(0, 22.0, side_w)).flatten() {
            ui.set_lyrics_side_rows(rows);
        }
        if let Some(rows) = full.then(|| l.rows(1, 44.0, full_w)).flatten() {
            ui.set_lyrics_full_rows(rows);
        }
        ui.set_lyric_sung(now.sung.into());
        ui.set_lyric_now(now.now.into());
        ui.set_lyric_mix(now.mix);
        ui.set_lyric_rest(now.rest.into());
        match now.wait.filter(|_| playing && !self.hidden) {
            Some(ms) => {
                let me = self.me.clone();
                self.lyrics_timer.start(TimerMode::SingleShot, Duration::from_millis(ms), move || me.with(|a| a.lyrics_step(false)));
            }
            None => self.lyrics_timer.stop(),
        }
    }

    fn lyric_tapped(&mut self, line: i32) {
        let (Some(l), Ok(line)) = (&self.lyrics, usize::try_from(line)) else { return };
        let ms = l.tap(line);
        self.seek_to(ms);
    }

    /// Shows `rows` in the queue: leaving rows fold away first, then the list settles.
    fn queue_shown(&mut self, rows: Vec<SongRow>) {
        if let Some(next) = self.queue_next.take() {
            settle(&self.queue_rows, next);
        }
        self.queue_timer.stop();
        let m = &self.queue_rows;
        let keep: HashSet<i32> = rows.iter().map(|r| r.index).collect();
        let gone: Vec<usize> = (0..m.row_count()).filter(|&i| m.row_data(i).is_some_and(|r| !keep.contains(&r.index))).collect();
        if gone.is_empty() || gone.len() == m.row_count() {
            settle(m, rows);
            return;
        }
        for i in gone {
            if let Some(mut r) = m.row_data(i) {
                r.leaving = true;
                m.set_row_data(i, r);
            }
        }
        self.queue_next = Some(rows);
        let me = self.me.clone();
        self.queue_timer.start(TimerMode::SingleShot, Duration::from_millis(QUEUE_FOLD_MS), move || {
            me.with(|a| {
                if let Some(next) = a.queue_next.take() {
                    settle(&a.queue_rows, next);
                }
            })
        });
    }

    /// Takes the song at list index `index` out of the queue shown (here, or on the device playing), and
    /// offers its undo for a few seconds.
    fn remove_queued(&mut self, index: usize) {
        let song = self.queue.as_ref().and_then(|q| q.view.songs.get(index).cloned());
        let Some(song) = song else { return };
        // It starts folding away at the click, not once the queue is read again.
        let m = &self.queue_rows;
        if let Some(at) = (0..m.row_count()).find(|&i| m.row_data(i).is_some_and(|r| r.index == index as i32)) {
            if let Some(mut r) = m.row_data(at) {
                r.leaving = true;
                m.set_row_data(at, r);
            }
        }
        self.on_session(|s| s.remove(index));
        self.ui().set_queue_undo(words::removed(&song.title).into());
        self.removed = Some(song.id);
        let weak = self.me.clone();
        self.undo_timer.start(TimerMode::SingleShot, Duration::from_millis(UNDO_MS), move || {
            weak.with(|a| {
                a.removed = None;
                a.ui().set_queue_undo("".into());
            })
        });
        self.follow();
    }

    /// Follows the other devices while their panel is open (remote control), and lists them.
    fn devices_watched(&self) {
        let ui = self.ui();
        let open = ui.get_inspector() == 3;
        // A jam guest follows its jam all along.
        if let Some(r) = self.session.as_ref().and_then(|s| s.remote()).filter(|_| !self.guest()) {
            r.watch(open);
        }
        if open {
            self.devices_shown();
        }
    }

    /// The devices panel's rows: this computer, then the remote's devices, the one playing ticked.
    fn devices_shown(&self) {
        let ui = self.ui();
        let remote = self.session.as_ref().and_then(|s| s.remote());
        // A jam guest's music is the jam's; it moves to no other device.
        ui.set_devices_on(remote.is_some() && !self.guest());
        let Some(r) = remote.filter(|_| ui.get_inspector() == 3) else { return };
        let active = self.there().map(|e| e.mirror.id.as_str());
        let here = std::iter::once(words::device_row(None, active.is_none()));
        let mut devices = r.devices();
        let names = nori_core::remote::device_names(devices.clone(), r.me(), words::kind_words());
        devices.iter_mut().zip(names).for_each(|(d, name)| d.name = name);
        let rows: Vec<crate::DeviceRow> = here.chain(devices.iter().map(|d| words::device_row(Some(d), active == Some(d.id.as_str())))).collect();
        devices_set(&ui, rows);
    }

    /// Where the sound goes, for the output buttons: the device playing while it is another one, else this
    /// computer's output as the core's `output_look` reads it (this computer until the first song opens one).
    fn output_shown(&self) {
        use nori_core::outputs::{output_look, OutputGlyph};
        let (elsewhere, glyph, name) = match self.there() {
            Some(e) => (true, words::device_glyph(e.mirror.kind), e.mirror.name.clone()),
            None if self.output.is_empty() => (false, 0, words::THIS_COMPUTER.to_string()),
            None => {
                let look = output_look(self.output.clone());
                let glyph = match look.glyph {
                    OutputGlyph::Phone => 0,
                    OutputGlyph::Headphones => 1,
                    OutputGlyph::Bluetooth => 2,
                    OutputGlyph::Cast => 3,
                };
                (false, glyph, words::output_name(look.port, look.name))
            }
        };
        let set = |o: crate::Output| {
            o.set_elsewhere(elsewhere);
            o.set_glyph(glyph);
            o.set_name(name.as_str().into());
        };
        set(self.ui().global::<crate::Output>());
        if let Some(p) = &self.player {
            set(p.global::<crate::Output>());
        }
    }

    /// Moves the music to device `id`, or here ("").
    fn pick_device(&self, id: String) {
        if let Some(r) = self.session.as_ref().and_then(|s| s.remote()) {
            r.pick((!id.is_empty()).then_some(id));
        }
    }

    /// Reads whose playback the player shows: this computer's, the device playing while it is another
    /// one, or the jam. Moving to another device or back, the player starts over from what it then shows.
    fn playback_read(&mut self) {
        let Some(s) = &self.session else { return };
        let playback = s.playback();
        let ui = self.ui();
        let there = match &playback {
            Playback::There(e) => Some(e.mirror.name.clone()),
            _ => None,
        };
        ui.set_playing_on(there.as_deref().map_or_else(String::new, words::playing_on).into());
        let moved = there.is_some() != self.there().is_some();
        self.playback = playback;
        self.output_shown();
        if moved {
            self.queue = None;
            ui.set_buffering(false);
            if self.there().is_none() {
                ui.set_volume(s.volume());
            }
            let song = s.now().song;
            self.song_shown(song);
        }
        if moved || self.there().is_some() {
            self.on_session(|s| s.mpris_changed());
        }
    }

    /// Redraws the settings or equalizer page from the current settings.
    fn settings_shown(&mut self) {
        self.repaint();
        let ui = self.ui();
        let prefs = crate::session::app().settings.current().unwrap_or_default();
        self.devices_shown();
        let today = crate::look::Today::now();
        let seasons = crate::settings::seasons(&prefs, today);
        ui.set_season_worn(seasons.worn);
        ui.set_season_following(seasons.following);
        ui.set_season_switch(seasons.switch.into());
        ui.set_season_detail(seasons.detail.into());
        ui.set_appearance_pick(crate::settings::appearance_pick(&prefs));
        let view = ui.get_view();
        if view == SETTINGS {
            ui.set_settings_tabs(ModelRc::new(VecModel::from(crate::settings::tabs(&self.facts.rules.settings).into_iter().map(|p| SharedString::from(crate::settings::tab_name(p))).collect::<Vec<_>>())));
            ui.set_settings(crate::settings::rows(&prefs, &self.facts, ui.get_settings_tab(), today));
        }
        if view == EQUALIZER {
            crate::eq::fill(&ui, &prefs);
            self.eq_extras();
        }
    }

    fn setting(&mut self, name: &str, value: &str) {
        let Some(s) = &self.session else { return };
        if s.setting(name, value).is_none() {
            self.say(&format!("{name}: not a setting"), true);
        } else {
            self.tune();
        }
        self.settings_shown();
        self.jam_shown();
    }

    /// The `Jam` global from the jam this computer hosts; nothing is asked of a remote while jams are off.
    fn jam_shown(&mut self) {
        let ui = self.ui();
        let g = ui.global::<crate::Jam>();
        let guest = self.guest();
        let jams = guest || crate::session::app().settings.prefs(|p| p.remote_control);
        let remote = self.session.as_ref().and_then(|s| s.remote()).filter(|_| jams);
        let unsupported = remote.as_ref().is_some_and(|r| r.jam_start() == nori_core::remote::JamStart::Unsupported);
        let view = remote.as_ref().and_then(|r| r.jam_view());
        let pairing = remote.as_ref().and_then(|r| r.pairing_link()).unwrap_or_default();
        if ui.get_pairing_link() != pairing.as_str() {
            ui.set_pairing_qr(if pairing.is_empty() { Image::default() } else { crate::jam::qr(&pairing) });
            ui.set_pairing_link(pairing.as_str().into());
        }
        if pairing.is_empty() {
            ui.set_pairing_open(false);
        }
        g.set_on(remote.is_some() && !unsupported && !guest);
        g.set_guest(guest);
        g.set_note(if unsupported { words::JAM_UNSUPPORTED.into() } else { "".into() });
        g.set_hosting(view.as_ref().is_some_and(|v| v.hosting));
        g.set_along(view.as_ref().is_some_and(|v| v.along));
        g.set_remote_songs(view.as_ref().is_some_and(|v| v.remote_songs));
        let listening = view.as_ref().filter(|_| guest).map_or(nori_core::remote::Listening::Watching, |v| v.listening);
        g.set_here(listening != nori_core::remote::Listening::Watching);
        g.set_along_note(words::jam_along(listening).into());
        self.jam_controls = remote.as_ref().filter(|_| guest).and_then(|r| r.jam_controls());
        g.set_can_play(self.offers(|c| c.play_pause));
        g.set_can_skip(self.offers(|c| c.skip));
        g.set_can_seek(self.offers(|c| c.seek));
        let link = view.as_ref().and_then(|v| v.link.clone()).unwrap_or_default();
        if link != self.jam_link {
            g.set_qr(if link.is_empty() { Image::default() } else { crate::jam::qr(&link) });
            g.set_link(link.as_str().into());
            let home = nori_core::remote::parse_invite(&link).map(|(server, _)| server).filter(|s| nori_core::remote::is_home_only(s));
            g.set_reach(home.map_or_else(String::new, |s| words::jam_home_only(&s)).into());
            self.jam_link = link;
        }
        let shown = view.as_ref().map(crate::jam::shown);
        g.set_host(shown.as_ref().map_or_else(String::new, |s| s.host.clone()).into());
        let asked = shown.as_ref().map(|s| s.asked.clone()).unwrap_or_default();
        if asked != self.asked {
            self.asked = asked;
            self.mark_playing();
        }
        let strip = shown.as_ref().map_or_else(String::new, |s| s.strip.clone());
        // Paused here while the jam plays on: play joins it again.
        let strip = if self.jam_controls.is_some_and(|c| c.paused_here) { words::JAM_PAUSED_HERE.to_string() } else { strip };
        g.set_strip(strip.into());
        g.set_listening(shown.as_ref().map_or_else(String::new, |s| s.listening.clone()).into());
        let (people, asks) = shown.map_or_else(Default::default, |s| (s.people, s.asks));
        g.set_people(ModelRc::new(VecModel::from(people)));
        g.set_asks(ModelRc::new(VecModel::from(asks)));
        let hosting = view.as_ref().is_some_and(|v| v.hosting);
        if self.jam_hosting != hosting {
            // The queue's "added by" chips come and go with the jam.
            self.jam_hosting = hosting;
            self.queue = None;
            self.follow();
        }
        self.mirror();
    }

    /// A jam op of the host's own: accepting or refusing a request, a role, sending someone out.
    fn jam_act(&self, op: nori_core::remote::wire::Op) {
        if let Some(r) = self.session.as_ref().and_then(|s| s.remote()) {
            r.jam_act(op);
        }
    }

    fn jam_copy_link(&self) {
        self.copy(self.jam_link.clone());
    }

    fn copy(&self, text: String) {
        let copied = arboard::Clipboard::new().and_then(|mut c| c.set_text(text));
        match copied {
            Ok(()) => self.say(words::LINK_COPIED, false),
            Err(e) => self.say(&e.to_string(), true),
        }
    }

    /// Enters the engine's shallow buffer after a change on a sound page (`rules::sound_tuning`).
    fn tune(&mut self) {
        if !self.tuning && nori_core::rules::sound_tuning(self.sound_page(self.ui().get_view()), true) {
            self.tuning = true;
            self.on_session(|s| s.engine.set_shallow(true));
        }
    }

    /// Gives the deep buffer back unless `view` is a sound page.
    fn untune(&mut self, view: i32) {
        if self.tuning && !self.sound_page(view) {
            self.tuning = false;
            self.on_session(|s| s.engine.set_shallow(false));
        }
    }

    /// The equalizer page, or Settings on its Sound tab: where a sound change is heard at once.
    fn sound_page(&self, view: i32) -> bool {
        view == EQUALIZER
            || view == SETTINGS && crate::settings::tabs(&self.facts.rules.settings).get(self.ui().get_settings_tab() as usize) == Some(&nori_core::settings_pages::SettingsPage::Sound)
    }

    /// Edits a level in place; redraws the page when the slider is released.
    fn slid(&mut self, name: &str, v: f32, last: bool) {
        let Some(level) = crate::settings::level_of(name) else { return };
        if let Some((effect, _)) = crate::session::app().settings.edit_level(level, v) {
            self.on_session(|s| s.applied(effect));
            self.tune();
        }
        if last {
            self.settings_shown();
        }
    }

    /// Edits an EQ band; the curve updates at once, the page on release.
    fn eq_gain(&mut self, i: usize, v: f32, last: bool) {
        let Some(p) = crate::session::app().settings.current() else { return };
        let effect = if p.eq_mode == nori_core::settings::EqMode::Graphic {
            crate::session::app().settings.edit_graphic(i as u32, v).map(|e| e.0)
        } else {
            p.eq_bands.get(i).and_then(|b| crate::session::app().settings.edit_band(i as u32, nori_core::settings::SoundBand { gain_db: v, ..*b }).map(|e| e.0))
        };
        if let Some(effect) = effect {
            self.on_session(|s| s.applied(effect));
            self.tune();
        }
        if last {
            self.settings_shown();
        } else if let Some(p) = crate::session::app().settings.current() {
            crate::eq::curve_only(&self.ui(), &p);
        }
    }

    fn eq_tool(&mut self, name: &str, i: usize) {
        use nori_core::settings_store::SoundTool;
        let tool = match name {
            "preset" => nori_core::dsp::eq_presets().get(i).cloned().map(|preset| SoundTool::Preset { preset }),
            "auto-preamp" => Some(SoundTool::AutoPreamp { automatic: true }),
            "manual-preamp" => Some(SoundTool::AutoPreamp { automatic: false }),
            "add-band" => Some(SoundTool::AddBand),
            "remove-band" => Some(SoundTool::RemoveBand { index: i as u32 }),
            "reset" => Some(SoundTool::ResetBands),
            _ => None,
        };
        let Some(tool) = tool else { return };
        match crate::session::app().settings.sound_tool(tool) {
            Ok(Some(change)) => {
                self.on_session(|s| s.applied(change.effect));
                self.tune();
            }
            Ok(None) => {}
            Err(e) => self.say(&format!("{e:?}"), true),
        }
        self.settings_shown();
    }

    /// The equalizer page's headphones (the AutoEQ list, searched) and saved profiles.
    fn eq_extras(&mut self) {
        let Some(s) = &self.session else { return };
        let ui = self.ui();
        let count = s.core.autoeq_count().unwrap_or(0);
        ui.set_eq_autoeq_count(count as i32);
        ui.set_eq_autoeq_search_words(words::autoeq_search(count).into());
        self.profiles = s.core.profiles().unwrap_or_default();
        let lines = self.profiles.iter().map(|p| crate::EqLine { title: p.name.as_str().into(), detail: words::profile_use(&p.outputs).into() }).collect::<Vec<_>>();
        ui.set_eq_profiles(ModelRc::new(VecModel::from(lines)));
        // Nothing searched: the curves that may be the headphones playing now, best first.
        if self.autoeq_query.trim().is_empty() {
            self.autoeq_hits = s.core.autoeq_for_output(self.output.clone(), 3);
            ui.set_eq_autoeq_for(words::autoeq_for(&self.output).into());
            self.show_autoeq_hits();
        }
    }

    fn show_autoeq_hits(&self) {
        let lines = self.autoeq_hits.iter().map(|e| crate::EqLine { title: e.name.as_str().into(), detail: words::autoeq_caption(e).into() }).collect::<Vec<_>>();
        self.ui().set_eq_autoeq_hits(ModelRc::new(VecModel::from(lines)));
    }

    /// Searches the AutoEQ list for `query` (the core's own limits and too-short rule).
    fn autoeq_search(&mut self, query: &str) {
        let Some(s) = &self.session else { return };
        self.autoeq_query = query.to_string();
        if query.trim().is_empty() {
            return self.eq_extras();
        }
        self.autoeq_hits = s.core.autoeq_browse(query.to_string()).hits;
        self.show_autoeq_hits();
    }

    /// Fetches hit `i`'s curve; it becomes the equalizer when it comes ([`Msg::AutoEqCurve`]).
    fn autoeq_pick(&mut self, i: usize) {
        let (Some(s), Some(entry)) = (&self.session, self.autoeq_hits.get(i).cloned()) else { return };
        let ui = self.ui();
        ui.set_eq_autoeq_busy(true);
        ui.set_eq_autoeq_note("".into());
        let (client, tx) = (s.client.clone(), self.tx.clone());
        nori_host::spawn("nori-autoeq", move || {
            let name = entry.name.clone();
            let curve = nori_core::transport::block_on(client.autoeq_curve(entry)).map_err(|e| e.to_string());
            tx.send(Msg::AutoEqCurve(name, curve));
        });
    }

    /// Fetches the AutoEQ list (asked for: on any network).
    fn autoeq_download(&mut self) {
        let Some(s) = &self.session else { return };
        self.ui().set_eq_autoeq_busy(true);
        let (client, tx) = (s.client.clone(), self.tx.clone());
        nori_host::spawn("nori-autoeq", move || {
            let done = nori_core::transport::block_on(client.autoeq_update(true, false)).map(|_| ()).map_err(|e| e.to_string());
            tx.send(Msg::AutoEqList(done));
        });
    }

    /// Makes saved profile `i`'s sound the one in use.
    fn profile_apply(&mut self, i: usize) {
        let Some(sound) = self.profiles.get(i).and_then(|p| nori_core::settings::sound_from_json(p.json.clone())) else { return };
        let store = &crate::session::app().settings;
        let Some(prefs) = store.current() else { return };
        let effect = store.put(prefs.with_sound(sound));
        self.on_session(|s| s.applied(effect));
        self.tune();
        self.settings_shown();
    }

    fn profile_save(&mut self, name: &str) {
        let name = name.trim();
        let (Some(s), Some(prefs)) = (&self.session, crate::session::app().settings.current()) else { return };
        if name.is_empty() {
            return;
        }
        if let Err(e) = s.core.profile_save_sound(name.to_string(), prefs.sound()) {
            self.say(&format!("{e:?}"), true);
        }
        self.settings_shown();
    }

    fn profile_delete(&mut self, i: usize) {
        let (Some(s), Some(p)) = (&self.session, self.profiles.get(i)) else { return };
        let _ = s.core.profile_delete(p.name.clone());
        self.settings_shown();
    }

    /// Lyrics service `id` dragged `by` places (down +).
    fn source_moved(&mut self, id: &str, by: i32) {
        let Some(p) = crate::session::app().settings.current() else { return };
        let s = nori_core::settings_model::state(&p, nori_core::settings_model::Output::default(), &crate::session::app().settings.model);
        let Some(at) = s.lyrics_sources.iter().position(|x| x.id == id) else { return };
        let to = (at as i32 + by).clamp(0, s.lyrics_sources.len() as i32 - 1) as usize;
        if to != at {
            self.setting("lyricsPlace", &format!("{id}:{to}"));
        }
    }

    fn setting_action(&mut self, name: &str) {
        match Act::of(name) {
            Some(Act::Equalizer) => self.go(EQUALIZER),
            Some(Act::AddServer) => {
                self.editing = None;
                let ui = self.ui();
                for set in [AppWindow::set_login_name, AppWindow::set_login_user, AppWindow::set_login_password] {
                    set(&ui, "".into());
                }
                ui.set_login_url("https://".into());
                self.go(LOGIN);
            }
            Some(Act::Server(id)) => {
                let mut prefs = crate::session::app().settings.current().unwrap_or_default();
                let Some(p) = prefs.servers.iter().find(|s| s.id == id).cloned() else { return };
                prefs.active_server_id = id;
                crate::session::app().settings.put(prefs);
                self.open(p);
            }
            Some(Act::Edit(id)) => self.edit_profile(id),
            Some(Act::Chore(c)) => {
                self.on_session(|s| s.action(c));
                self.on_session(|s| s.facts());
            }
            None => {}
        }
    }

    /// Shows `song` as the one playing, and asks for its lyrics.
    fn song_shown(&mut self, song: Option<Song>) {
        let ui = self.ui();
        self.heard = song.as_ref().map(|s| s.id.clone());
        self.song = song;
        let song = self.song.clone().unwrap_or_default();
        ui.set_has_song(self.song.is_some());
        ui.set_now_title(words::title(&song.title).into());
        ui.set_now_artist(song.artist.as_str().into());
        ui.set_now_album(song.album.as_str().into());
        ui.set_now_artist_id(song.artist_id.clone().unwrap_or_default().into());
        ui.set_now_album_id(song.album_id.clone().unwrap_or_default().into());
        ui.set_now_credits(credits(&song));
        ui.set_duration_ms((song.duration as i64 * 1000) as i32);
        ui.set_live(nori_core::is_radio(&song.id));
        let art = song.cover_art.clone().unwrap_or_default();
        ui.set_now_art(art.as_str().into());
        let c = self.art.borrow().colours.get(&art).cloned();
        self.now_colours(&art, c.as_deref());
        // Request the large cover for the backdrop colours even when no view shows it.
        if c.is_none() {
            let _ = cover_image(&self.art, &self.me, art.as_str().into(), 1);
        }
        self.mark_playing();
        self.lyrics = None;
        // The last song's lines stay (faded out while waiting) until this one's come; none without a song.
        if self.song.is_none() {
            ui.set_lyrics_lines(ModelRc::default());
        }
        ui.set_lyrics_active(-1);
        ui.set_lyrics_note(if self.song.is_some() { "Looking for lyrics…".into() } else { "".into() });
        ui.set_lyrics_waiting(self.song.is_some());
        if let (Some(s), Some(id), true) = (&self.session, &self.heard, self.lyrics_viewed) {
            s.lyrics(id.clone());
        }
    }

    /// Fetches lyrics only while they are on screen: the song's as they come in sight, and the core warms
    /// the songs ahead meanwhile.
    fn lyrics_seen(&mut self) {
        let ui = self.ui();
        let viewed = !self.hidden && (ui.get_full_player() && ui.get_full_panel() == 2 || ui.get_inspector() == 2);
        if viewed == self.lyrics_viewed {
            return;
        }
        self.lyrics_viewed = viewed;
        let Some(s) = &self.session else { return };
        s.lyrics_view(viewed);
        if let (true, None, Some(id)) = (viewed, &self.lyrics, &self.heard) {
            s.lyrics(id.clone());
        }
    }

    /// The back button: the page this one was opened from, else the tab it was opened on; Home from a
    /// page of its own.
    fn back(&mut self) {
        if self.ui().get_view() != PAGE {
            return self.go(HOME);
        }
        match self.trail.back() {
            Back::Page(page) => self.show_page(page),
            Back::Tab(tab) => self.go(tab),
        }
    }

    /// Whether the playing bars move now: the music plays, not waiting for bytes, and Motion is not E-ink.
    fn advancing(&self) {
        let ui = self.ui();
        ui.global::<crate::Playing>().set_advancing(ui.get_playing() && !ui.get_buffering() && !self.hidden && !self.still);
    }

    /// Syncs the window with what the player shows: this computer's playback and queue, the device's
    /// playing elsewhere, or the jam's.
    fn follow(&mut self) {
        let Some(s) = &self.session else { return };
        let ui = self.ui();
        let now = s.now();
        // Copy the queue only when it changed.
        let queue = s.queue_stale(self.queue.as_ref()).then(|| s.queue_shown(self.queue.as_ref().map(|q| &q.view)));
        let playing = now.playing;
        ui.set_playing(playing);
        ui.set_mixing(now.mixing && now.state == State::Playing);
        if ui.get_view() == PAGE {
            self.hero_shown();
        }
        position_shown(&ui, self.player.as_ref(), now.position_now() as i32);
        self.lyrics_seen();
        if now.song.as_ref().map(|s| &s.id) != self.heard.as_ref() {
            self.song_shown(now.song.clone());
        }
        ui.set_now_starred(self.song.as_ref().is_some_and(|s| self.starred(s)));
        if !matches!(self.playback, Playback::Here) {
            ui.set_buffering(now.buffering);
            if let Some(v) = now.volume {
                ui.set_volume(v);
            }
        }
        self.advancing();
        if let Some(q) = queue {
            ui.set_shuffle(q.view.shuffle);
            ui.set_repeat(q.view.repeat as i32);
            // Every song in play order, as Android's queue: those played (dimmed, above), the one playing, the rest.
            let now = q.rows.now;
            let rows = q.rows.order.iter().enumerate().filter_map(|(at, &i)| {
                let s = q.view.songs.get(i as usize)?;
                let at = at as i32;
                Some(SongRow { by: q.by.get(&s.id).cloned().unwrap_or_default().into(), played: at < now, ..row(s, i as usize, at == now) })
            }).collect();
            self.queue_shown(rows);
            ui.set_queue_from(queue_from(q.upcoming().map(|(_, s)| s)).into());
            self.queue = Some(q);
        }
        // Seek bar timer: only while playing, quarter-pixel steps of the widest bar.
        let step = pixel_ms(ui.get_duration_ms() as i64);
        let ticks = playing && !self.hidden;
        if ticks && (!self.tick.running() || self.tick_ms != step) {
            self.tick_ms = step;
            let me = self.me.clone();
            self.tick.start(TimerMode::Repeated, Duration::from_millis(step), move || {
                me.with(|a| {
                    let ui = a.ui();
                    position_shown(&ui, a.player.as_ref(), a.position_now() as i32);
                });
            });
        } else if !ticks {
            self.tick.stop();
        }
        self.lyrics_step(true);
        self.mirror();
    }
}

fn position_shown(ui: &AppWindow, player: Option<&PlayerBar>, position_ms: i32) {
    ui.set_position_ms(position_ms);
    if ui.get_full_player() || player.is_none() { ui.set_display_position_ms(position_ms); }
    if let Some(player) = player.filter(|_| !ui.get_full_player()) { player.set_position_ms(position_ms); }
}

/// Seek bar step: a quarter pixel of the widest bar (~380 pt at 2x), clamped to 16..250 ms.
fn pixel_ms(duration_ms: i64) -> u64 {
    (duration_ms.max(1) as u64 / (760 * 4)).clamp(16, 250)
}

fn cards(it: impl Iterator<Item = Card>) -> ModelRc<Card> {
    ModelRc::new(VecModel::from(it.collect::<Vec<_>>()))
}

fn album_card(a: &nori_core::Album) -> Card {
    let sub_artist = a.artist_id.clone().unwrap_or_default().into();
    Card { id: a.id.as_str().into(), title: a.name.as_str().into(), sub: nori_core::lines::album_subtitle(&a.artist, a.year).into(), art: a.cover_art.clone().unwrap_or_default().into(), sub_artist, kind: 0 }
}

fn artist_card(a: &nori_core::Artist) -> Card {
    Card { id: a.id.as_str().into(), title: a.name.as_str().into(), sub: a.album_count.map(words::albums).unwrap_or_default().into(), art: a.cover_art.clone().unwrap_or_default().into(), sub_artist: SharedString::default(), kind: 1 }
}

/// A playlist's card: its song count, or its own line (a provider's, who made it) when not counted.
fn playlist_card(p: &nori_core::Playlist) -> Card {
    let sub = match &p.comment {
        Some(line) if p.song_count == 0 => line.clone(),
        _ => words::songs(p.song_count as usize),
    };
    Card { id: p.id.as_str().into(), title: p.name.as_str().into(), sub: sub.into(), art: p.cover_art.clone().unwrap_or_default().into(), sub_artist: SharedString::default(), kind: 2 }
}

/// A song's card on a feed's shelf: played from it, with the shelf's songs after.
fn song_card(s: &Song) -> Card {
    let sub_artist = s.artist_id.clone().unwrap_or_default().into();
    Card { id: s.id.as_str().into(), title: s.title.as_str().into(), sub: s.artist.as_str().into(), art: s.cover_art.clone().unwrap_or_default().into(), sub_artist, kind: 3 }
}

fn row(s: &Song, index: usize, playing: bool) -> SongRow {
    SongRow {
        title: words::title(&s.title).into(),
        artist: s.artist.as_str().into(),
        credits: credits(s),
        album: s.album.as_str().into(),
        artist_id: s.artist_id.clone().unwrap_or_default().into(),
        album_id: s.album_id.clone().unwrap_or_default().into(),
        time: nori_core::numbers::clock(s.duration as i64, false).into(),
        art: s.cover_art.clone().unwrap_or_default().into(),
        index: index as i32,
        playing,
        played: false,
        leaving: false,
        fresh: false,
        by: SharedString::default(),
    }
}

/// How `s`'s artist line links its artists (menus.rs's `artist_line`).
fn credits(s: &Song) -> Credits {
    use nori_core::menus::{artist_line, ArtistLine};
    let link = |text: String, id: Option<String>| ArtistLink { text: text.into(), id: id.unwrap_or_default().into() };
    let model = |v: Vec<ArtistLink>| ModelRc::new(VecModel::from(v));
    match artist_line(s) {
        ArtistLine::One => Credits::default(),
        ArtistLine::Split(pieces) => Credits { pieces: model(pieces.into_iter().map(|p| link(p.text, p.id)).collect()), choices: ModelRc::default() },
        ArtistLine::Several(artists) => Credits { pieces: ModelRc::default(), choices: model(artists.into_iter().map(|a| link(a.name, Some(a.id))).collect()) },
    }
}

/// Sets `m` to `rows`, updating in place when the length matches.
fn renew<T: Clone + PartialEq + 'static>(m: &VecModel<T>, rows: Vec<T>) {
    if m.row_count() != rows.len() {
        m.set_vec(rows);
        return;
    }
    for (i, r) in rows.into_iter().enumerate() {
        if m.row_data(i).as_ref() != Some(&r) {
            m.set_row_data(i, r);
        }
    }
}

/// Updates `m` to `rows` in place: removes rows not in `rows`, inserts new ones marked fresh (they
/// animate open), keeps the rest. A reorder replaces the whole list.
fn settle(m: &VecModel<SongRow>, rows: Vec<SongRow>) {
    let keep: HashSet<i32> = rows.iter().map(|r| r.index).collect();
    for i in (0..m.row_count()).rev() {
        if m.row_data(i).is_some_and(|r| !keep.contains(&r.index)) {
            m.remove(i);
        }
    }
    let had: HashSet<i32> = m.iter().map(|r| r.index).collect();
    if had.is_empty() {
        m.set_vec(rows);
        return;
    }
    let order: Vec<i32> = rows.iter().map(|r| r.index).filter(|i| had.contains(i)).collect();
    if order != m.iter().map(|r| r.index).collect::<Vec<_>>() {
        m.set_vec(rows);
        return;
    }
    for (j, mut r) in rows.into_iter().enumerate() {
        match m.row_data(j) {
            Some(old) if old.index == r.index => {
                r.fresh = old.fresh;
                if old != r {
                    m.set_row_data(j, r);
                }
            }
            _ => {
                r.fresh = true;
                m.insert(j, r);
            }
        }
    }
}

/// The album of all `upcoming` songs, if they share one.
fn queue_from<'a>(upcoming: impl Iterator<Item = &'a Song>) -> String {
    let mut albums = upcoming.map(|s| s.album.as_str());
    match albums.next() {
        Some(first) if !first.is_empty() && albums.all(|a| a == first) => first.to_string(),
        _ => String::new(),
    }
}

fn devices_set(ui: &AppWindow, rows: Vec<crate::DeviceRow>) {
    let model = ui.get_devices();
    match model.as_any().downcast_ref::<VecModel<crate::DeviceRow>>() {
        Some(model) => renew(model, rows),
        None => ui.set_devices(ModelRc::new(VecModel::from(rows))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dressed() -> crate::look::Look {
        crate::look::plain(&nori_core::settings::StoredPrefs::default(), true, crate::look::Today { month: 10, day: 19, southern: false })
    }

    /// A window in the theme's colors, as the app paints it.
    fn window() -> AppWindow {
        let ui = AppWindow::new().unwrap();
        dressed().paint(&ui.global::<Theme>());
        ui
    }

    #[test]
    fn back_retraces_pages_then_the_tab() {
        let (artist, album) = (PageAsk::Artist("ar".into()), PageAsk::Album("al".into()));
        let mut trail = PageTrail::default();
        trail.opened(artist.clone(), FAVORITES);
        trail.opened(album, PAGE);
        assert_eq!(trail.back(), Back::Page(artist), "the album opened from the artist goes back to it");
        assert_eq!(trail.back(), Back::Tab(FAVORITES), "then where the artist was opened from");
        trail.opened(PageAsk::Album("x".into()), HOME);
        assert_eq!(trail.back(), Back::Tab(HOME), "a page opened from a tab forgets the pages before");
        trail.opened(PageAsk::Artist("ar".into()), opened_from(SEARCH));
        assert_eq!(trail.back(), Back::Tab(SEARCH), "an artist found by search goes back to the search");
    }

    fn pill() -> crate::PlayerBar {
        let bar = crate::PlayerBar::new().unwrap();
        dressed().paint(&bar.global::<Theme>());
        bar
    }

    #[test]
    fn an_edited_profile_keeps_its_id_and_password() {
        let info = nori_core::providers::providers().into_iter().find(|i| i.kind == nori_core::settings::ServerKind::Subsonic).unwrap();
        let saved = SavedServer { id: "p1".into(), url: "https://old".into(), user: "me".into(), password: "secret".into(), music_folder_ids: vec!["f".into()], ..Default::default() };
        let saved = SavedServer { name: "Home".into(), ..saved };
        let form = |pw: &str| ["Typed".to_string(), "https://new/".into(), " me ".into(), pw.into()];
        let edited = login_draft(&info, Some(saved.clone()), form(""));
        assert_eq!((edited.id.as_str(), edited.url.as_str(), edited.user.as_str(), edited.password.as_str(), edited.name.as_str()), ("p1", "https://new", "me", "secret", "Home"));
        assert_eq!(edited.music_folder_ids, saved.music_folder_ids, "what the form does not ask stays, its name too");
        assert_eq!(login_draft(&info, Some(saved), form("new")).password, "new");
        assert_ne!(login_draft(&info, None, form("pw")).id, "p1", "a new profile");
    }

    #[test]
    fn albums_come_page_after_page() {
        let page = |from: usize, n: usize| (from..from + n).map(|i| nori_core::Album { id: i.to_string(), ..Default::default() }).collect::<Vec<_>>();
        let full = nori_host::pages::ALBUM_PAGE as usize;
        let (mut list, mut asked) = (Vec::new(), 0);
        assert_eq!(albums_paged(&mut list, AlbumSort::ByName, 0, &page(0, full), &mut asked), Some(full as u32));
        assert_eq!(albums_paged(&mut list, AlbumSort::ByName, 0, &page(0, full), &mut asked), None, "the server's copy of the first page");
        assert_eq!(albums_paged(&mut list, AlbumSort::ByName, full as u32, &page(full, 100), &mut asked), None, "the last page");
        assert_eq!(list.len(), full + 100);
        assert_eq!(list.last().map(|c| c.id.to_string()), Some((full + 99).to_string()));
        let (mut list, mut asked) = (Vec::new(), 0);
        assert_eq!(albums_paged(&mut list, AlbumSort::Random, 0, &page(0, full), &mut asked), None, "a random order is one page");
    }

    #[test]
    fn cover_bursts_keep_the_cache_bounded() {
        let now = Instant::now();
        for size in [CoverSize::Card, CoverSize::Large, CoverSize::Hero] {
            let mut art = Art::default();
            let side = cover_px(size);
            let image = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(side, side));
            let limit = if size == CoverSize::Card { SMALL_BYTES } else { LARGE_BYTES } / (side as usize * side as usize * 4);
            for i in 0..limit + 20 {
                art.insert(CoverKey { id: i.to_string(), size }, image.clone(), None, now + Duration::from_millis(i as u64));
            }
            assert_eq!(art.images.len(), limit, "a burst must evict covers even when they were recently drawn");
            assert!(!art.images.contains_key(&CoverKey { id: "0".into(), size }));
            assert!(art.images.contains_key(&CoverKey { id: (limit + 19).to_string(), size }));
        }
    }

    #[test]
    fn cover_colours_last_until_both_large_sizes_leave() {
        let mut art = Art::default();
        let now = Instant::now();
        let image = |size| { let side = cover_px(size); Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(side, side)) };
        let key = |id: &str, size| CoverKey { id: id.into(), size };
        let colours = nori_look::cover::derive(&[0xff406080], 1, 1, false);
        art.insert(key("shared", CoverSize::Card), image(CoverSize::Card), None, now);
        art.insert(key("shared", CoverSize::Large), image(CoverSize::Large), Some(Box::new(colours)), now);
        art.insert(key("shared", CoverSize::Hero), image(CoverSize::Hero), None, now + Duration::from_millis(1));
        let large = image(CoverSize::Large);
        let count = (LARGE_BYTES - (HERO_PX * HERO_PX * 4) as usize) / (LARGE_PX * LARGE_PX * 4) as usize;
        for i in 0..count {
            art.insert(key(&i.to_string(), CoverSize::Large), large.clone(), None, now + Duration::from_millis(i as u64 + 2));
        }
        assert!(!art.images.contains_key(&key("shared", CoverSize::Large)));
        assert!(art.images.contains_key(&key("shared", CoverSize::Hero)));
        assert!(art.colours.contains_key("shared"));
        art.insert(key("last", CoverSize::Large), large, None, now + Duration::from_millis(count as u64 + 2));
        assert!(!art.images.contains_key(&key("shared", CoverSize::Hero)));
        assert!(!art.colours.contains_key("shared"));
        assert!(art.images.contains_key(&key("shared", CoverSize::Card)));
    }

    fn image_pixels(image: &Image) -> SharedPixelBuffer<Rgba8Pixel> {
        if let Some(pixels) = image.to_rgba8() { return pixels; }
        let image = i_slint_renderer_skia::image_to_skia(image).unwrap();
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(image.width() as u32, image.height() as u32);
        let info = skia_safe::ImageInfo::new((image.width(), image.height()), skia_safe::ColorType::RGBA8888, skia_safe::AlphaType::Unpremul, None);
        assert!(image.read_pixels(&info, pixels.make_mut_bytes(), image.width() as usize * 4, (0, 0), skia_safe::image::CachingHint::Disallow));
        pixels
    }

    #[test]
    fn visible_mosaic_keeps_source_covers() {
        let art = RefCell::new(Art::default());
        let now = Instant::now();
        let image = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(SMALL_PX, SMALL_PX));
        let limit = SMALL_BYTES / (SMALL_PX * SMALL_PX * 4) as usize;
        for i in 0..limit {
            art.borrow_mut().insert(CoverKey { id: i.to_string(), size: CoverSize::Card }, image.clone(), None, now);
        }
        let covers = ModelRc::new(VecModel::from((0..4).map(|i| SharedString::from(i.to_string())).collect::<Vec<_>>()));
        let app = AppHandle(Weak::new());
        pick_image(&art, &app, covers.clone(), mosaic);
        for i in 0..4 {
            art.borrow_mut().images.get_mut(&CoverKey { id: i.to_string(), size: CoverSize::Card }).unwrap().1 = now - Duration::from_secs(1);
        }
        pick_image(&art, &app, covers, |_| panic!("an unchanged mosaic stays cached"));
        art.borrow_mut().insert(CoverKey { id: "new".into(), size: CoverSize::Card }, image, None, Instant::now());
        for i in 0..4 {
            assert!(art.borrow().images.contains_key(&CoverKey { id: i.to_string(), size: CoverSize::Card }), "visible mosaic source {i} must remain cached");
        }
    }

    #[test]
    fn pick_updates_replace_the_changed_quadrant() {
        let art = RefCell::new(Art::default());
        let now = Instant::now();
        let solid = |r, g, b| {
            let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(16, 16);
            pixels.make_mut_slice().fill(Rgba8Pixel { r, g, b, a: 255 });
            Image::from_rgba8(pixels)
        };
        for (i, image) in [solid(255, 0, 0), solid(0, 255, 0), solid(0, 0, 255), solid(255, 255, 0)].into_iter().enumerate() {
            art.borrow_mut().insert(CoverKey { id: i.to_string(), size: CoverSize::Card }, image, None, now);
        }
        let covers = ModelRc::new(VecModel::from((0..4).map(|i| SharedString::from(i.to_string())).collect::<Vec<_>>()));
        let app = AppHandle(Weak::new());
        let before = image_pixels(&pick_image(&art, &app, covers.clone(), mosaic));
        art.borrow_mut().insert(CoverKey { id: "0".into(), size: CoverSize::Card }, solid(255, 0, 255), None, now);
        let pending = image_pixels(&pick_image(&art, &app, covers.clone(), |_| None));
        assert_eq!(pending.as_slice(), before.as_slice(), "the displayed mosaic remains while its replacement is pending");
        let after = image_pixels(&pick_image(&art, &app, covers, mosaic));
        for (i, expected) in [(255, 0, 255), (0, 255, 0), (0, 0, 255), (255, 255, 0)].into_iter().enumerate() {
            let offset = ((i / 2) * SMALL_PX as usize + 20) * after.width() as usize + (i % 2) * SMALL_PX as usize + 20;
            let p = after.as_slice()[offset];
            assert_eq!((p.r, p.g, p.b, p.a), (expected.0, expected.1, expected.2, 255));
            assert_eq!(before.as_slice()[offset] != p, i == 0);
        }
    }

    #[test]
    fn resizing_does_not_animate_layout() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        for (view, full, panel) in [(HOME, false, 0), (ALBUMS, false, 1), (HOME, true, 0), (HOME, true, 2)] {
            let ui = window();
            ui.window().set_size(slint::PhysicalSize::new(1280, 820));
            ui.set_view(view);
            ui.set_full_player(full);
            ui.set_inspector(panel);
            ui.set_full_panel(panel);
            ui.set_picks_loaded(true);
            let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(16, 16);
            pixels.make_mut_slice().fill(Rgba8Pixel { r: 120, g: 180, b: 240, a: 255 });
            let image = Image::from_rgba8(pixels);
            let pick_image = image.clone();
            ui.on_art(move |_, _, _| image.clone());
            ui.on_pick_art(move |_, _| pick_image.clone());
            let cards = ModelRc::new(VecModel::from((0..50).map(|i| crate::Card {
                id: i.to_string().into(), art: "cover".into(), title: format!("Album {i}").into(), ..Default::default()
            }).collect::<Vec<_>>()));
            ui.set_albums(cards.clone());
            ui.set_shelves(ModelRc::new(VecModel::from(vec![crate::Shelf { loaded: true, cards, title: "Recently played".into() }])));
            ui.set_picks(ModelRc::new(VecModel::from(vec![crate::Pick { title: "Favorites".into(), covers: ModelRc::new(VecModel::from(vec!["cover".into(); 4])), ..Default::default() }; 8])));
            ui.set_now_art("cover".into());
            ui.set_now_title("Playing track".into());
            ui.show().unwrap();
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
            for (width, height) in [(1600, 900), (960, 600), (1440, 820)] {
                ui.window().set_size(slint::PhysicalSize::new(width, height));
                let immediate = ui.window().take_snapshot().unwrap();
                i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
                slint::platform::update_timers_and_animations();
                let settled = ui.window().take_snapshot().unwrap();
                let changed = immediate.as_slice().iter().zip(settled.as_slice()).filter(|(a, b)| a != b).count();
                assert_eq!(changed, 0, "resize {width}x{height}, view {view}, full {full}, panel {panel} must be laid out immediately");
            }
            if full { ui.set_full_panel(if panel == 0 { 1 } else { 0 }); }
            else { ui.set_inspector(if panel == 0 { 1 } else { 0 }); }
            let start = ui.window().take_snapshot().unwrap();
            i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(100));
            slint::platform::update_timers_and_animations();
            let middle = ui.window().take_snapshot().unwrap();
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            let end = ui.window().take_snapshot().unwrap();
            assert!(start.as_slice() != middle.as_slice(), "opening or closing a panel still animates");
            assert!(middle.as_slice() != end.as_slice(), "the panel passes through an intermediate layout");
        }
    }

    /// A page's song rows show their songs: a row with a song draws more than an empty one.
    #[test]
    fn song_rows_show_their_songs() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let drawn = |song: SongRow| {
            let ui = window();
            ui.window().set_size(slint::PhysicalSize::new(1280, 820));
            ui.set_view(PAGE);
            ui.set_page_songs(ModelRc::new(VecModel::from(vec![song])));
            ui.show().unwrap();
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            let shot = ui.window().take_snapshot().unwrap();
            // The first row, under the page's head.
            let (w, rows) = (shot.width() as usize, 455..495);
            rows.flat_map(|y| shot.as_slice()[y * w..(y + 1) * w].to_vec()).collect::<Vec<_>>()
        };
        let song = SongRow { title: "Glass Harbour".into(), time: "3:21".into(), index: 0, ..Default::default() };
        assert_ne!(drawn(song), drawn(SongRow::default()), "the row draws its song");
    }

    /// A song's menu does what its entries say: Favorite stars the song, the album's opens it, a line
    /// folded under More is told by its kind.
    #[test]
    fn a_songs_menu_entries_act() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_view(PAGE);
        let menu = vec![
            crate::SongGo { kind: 0, title: "Favorite".into(), enabled: true, ..Default::default() },
            crate::SongGo { kind: 1, title: "Go to album".into(), id: "al".into(), enabled: true, ..Default::default() },
            crate::SongGo { kind: 14, title: "Details".into(), more: true, enabled: true, ..Default::default() },
        ];
        ui.on_song_menu(move |_, _| ModelRc::new(VecModel::from(menu.clone())));
        ui.set_page_songs(ModelRc::new(VecModel::from(vec![SongRow { title: "Glass Harbour".into(), ..Default::default() }])));
        let did = Rc::new(RefCell::new(Vec::new()));
        let seen = did.clone();
        ui.on_song(move |list, i, how| seen.borrow_mut().push(format!("song {list} {i} {how}")));
        let seen = did.clone();
        ui.global::<crate::Go>().on_album(move |id| seen.borrow_mut().push(format!("album {id}")));
        ui.show().unwrap();
        let settle = || {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
        };
        settle();
        let press = |x: f32, y: f32, button| {
            let at = slint::LogicalPosition::new(x, y);
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position: at, button });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: at, button });
            settle();
        };
        use slint::platform::PointerEventButton::{Left, Right};
        // The menu opens where the row is pressed: the song at its head, then Play, the song's own lines and
        // More, whose lines take the card's place under Back.
        let row = (400.0, 470.0);
        // Its 6 px inset, the song's head (53 px with its hairline), then 32 px rows.
        let entry = |n: f32| row.1 + 6.0 + 53.0 + n * 32.0 + 16.0;
        press(row.0, row.1, Right);
        press(row.0 + 40.0, entry(2.0), Left);
        press(row.0, row.1, Right);
        press(row.0 + 40.0, entry(1.0), Left);
        press(row.0, row.1, Right);
        press(row.0 + 40.0, entry(0.0), Left);
        press(row.0, row.1, Right);
        press(row.0 + 40.0, entry(3.0), Left);
        press(row.0 + 40.0, entry(0.0), Left);
        press(row.0 + 40.0, entry(2.0), Left);
        press(row.0, row.1, Right);
        press(row.0 + 40.0, entry(3.0), Left);
        press(row.0 + 40.0, entry(1.0), Left);
        assert_eq!(*did.borrow(), ["album al", "song 0 0 0", "song 0 0 -1", "album al", "song 0 0 14"]);
    }

    /// The player's menu opens above the pill with the lines it is given, and a line reaches the app by
    /// its kind.
    #[test]
    fn the_players_menu_acts() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.global::<crate::Chrome>().set_player_x(140.0);
        ui.global::<crate::Chrome>().set_player_w(1000.0);
        ui.on_player_menu(|| ModelRc::new(VecModel::from(vec![crate::SongGo { kind: 17, title: "Sleep timer…".into(), enabled: true, ..Default::default() }])));
        let did = Rc::new(RefCell::new(Vec::new()));
        let seen = did.clone();
        ui.on_player_act(move |kind, _| seen.borrow_mut().push(kind));
        ui.show().unwrap();
        let settle = || {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
        };
        settle();
        ui.invoke_show_player_menu();
        settle();
        // One row under the song's head: the popover's foot on the pill's room, its right edge the ⋯ button's.
        let at = slint::LogicalPosition::new(1128.0 - 100.0, 820.0 - 96.0 - 6.0 - 16.0);
        use slint::platform::PointerEventButton::Left;
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at });
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position: at, button: Left });
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: at, button: Left });
        settle();
        assert_eq!(*did.borrow(), [17]);
    }

    /// A queue song dragged two rows down glides into its slot, then is moved once; on the frame the list
    /// takes its new order, the rows are already where they end, with nothing sliding back.
    #[test]
    fn a_dropped_queue_song_lands_where_it_is_let_go() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_has_song(true);
        ui.set_inspector(1);
        let row = |i: i32, n: i32| SongRow { title: format!("Song {n}").into(), index: i, playing: i == 0, ..Default::default() };
        let queue = Rc::new(VecModel::from((0..6).map(|i| row(i, i)).collect::<Vec<_>>()));
        ui.set_queue(ModelRc::from(queue.clone()));
        let moves = Rc::new(RefCell::new(Vec::new()));
        let (seen, model) = (moves.clone(), queue.clone());
        // As the app does: the session moves the song and the rows are read again, in their new order.
        ui.on_move_queued(move |from, to| {
            seen.borrow_mut().push((from, to));
            let mut order: Vec<i32> = (0..6).collect();
            let n = order.remove(from as usize);
            order.insert(to as usize, n);
            for (i, n) in order.into_iter().enumerate() {
                model.set_row_data(i, row(i as i32, n));
            }
        });
        ui.show().unwrap();
        let pass = |ms: u64| {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(ms));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap()
        };
        pass(1000);
        let pointer = |e| ui.window().dispatch_event(e);
        use slint::platform::{PointerEventButton::Left, WindowEvent::*};
        let at = |y: f32| slint::LogicalPosition::new(1100.0, y);
        pointer(PointerMoved { position: at(271.0) });
        pointer(PointerPressed { position: at(271.0), button: Left });
        for y in (271..=383).step_by(8) {
            pointer(PointerMoved { position: at(y as f32) });
            pass(16);
        }
        pointer(PointerReleased { position: at(383.0), button: Left });
        pass(60);
        assert!(moves.borrow().is_empty(), "still gliding into its slot");
        pass(100);
        assert_eq!(*moves.borrow(), [(1, 3)], "Song 1 lands two rows down, once");
        let landed = pass(1);
        let settled = pass(1000);
        assert!(landed.as_slice() == settled.as_slice(), "the rows land in place, without sliding back first");
    }

    /// A lyrics service dragged by its handle past two others moves two places down, once let go.
    #[test]
    fn a_lyrics_service_is_dragged_to_its_rank() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.global::<crate::Chrome>().set_top_x(190.0);
        ui.global::<crate::Chrome>().set_top_w(900.0);
        let facts = crate::settings::Facts { rules: nori_core::browse::profile_rules(false), ..Default::default() };
        let tab = crate::settings::tabs(&facts.rules.settings).iter().position(|p| *p == nori_core::settings_pages::SettingsPage::Lyrics).unwrap() as i32;
        ui.set_view(SETTINGS);
        ui.set_settings(crate::settings::rows(&nori_core::settings::StoredPrefs::default(), &facts, tab, crate::look::Today { month: 10, day: 19, southern: false }));
        let moved = Rc::new(RefCell::new(Vec::new()));
        let seen = moved.clone();
        ui.on_source_moved(move |id, by| seen.borrow_mut().push((id.to_string(), by)));
        ui.show().unwrap();
        let pass = |ms: u64| {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(ms));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
        };
        pass(1000);
        use slint::platform::{PointerEventButton::Left, WindowEvent::*};
        // The first service's handle; the rows below it stand 56 px apart.
        let at = |y: f32| slint::LogicalPosition::new(978.0, y);
        ui.window().dispatch_event(PointerMoved { position: at(532.0) });
        ui.window().dispatch_event(PointerPressed { position: at(532.0), button: Left });
        for y in (532..=662).step_by(10) {
            ui.window().dispatch_event(PointerMoved { position: at(y as f32) });
            pass(16);
        }
        ui.window().dispatch_event(PointerReleased { position: at(662.0), button: Left });
        pass(16);
        assert_eq!(*moved.borrow(), [("LYRICS_PLUS".to_string(), 2)]);
    }

    /// A card opens on a click with jams on too, where a right press opens its menu.
    #[test]
    fn a_card_opens_on_a_click_with_jams_on() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = window();
        ui.window().set_size(slint::LogicalSize::new(1280.0, 820.0));
        ui.global::<crate::Jam>().set_on(true);
        ui.set_view(ALBUMS);
        ui.set_albums(cards((0..4).map(|i| Card { id: i.to_string().into(), title: "Album".into(), ..Default::default() })));
        let opened = Rc::new(RefCell::new(Vec::new()));
        let seen = opened.clone();
        ui.on_open_album(move |id| seen.borrow_mut().push(id.to_string()));
        ui.show().unwrap();
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        // The first card's cover, under the page's title.
        let at = slint::LogicalPosition::new(100.0, 220.0);
        let button = slint::platform::PointerEventButton::Left;
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at });
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position: at, button });
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: at, button });
        assert_eq!(*opened.borrow(), ["0"]);
    }

    #[test]
    fn a_press_on_the_sheet_stays_on_it() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = window();
        ui.window().set_size(slint::LogicalSize::new(1280.0, 820.0));
        ui.set_view(ALBUMS);
        ui.set_albums(cards((0..24).map(|i| Card { id: i.to_string().into(), title: "Album".into(), ..Default::default() })));
        let opened = Rc::new(RefCell::new(Vec::new()));
        let seen = opened.clone();
        ui.on_open_album(move |id| seen.borrow_mut().push(id.to_string()));
        ui.show().unwrap();
        let click = || {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            // The last column's card in the second row.
            let at = slint::LogicalPosition::new(1180.0, 450.0);
            let button = slint::platform::PointerEventButton::Left;
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position: at, button });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: at, button });
        };
        click();
        assert_eq!(*opened.borrow(), ["11"], "the card is there");
        ui.set_inspector(1);
        click();
        assert_eq!(*opened.borrow(), ["11"], "the queue sheet over it takes the press");
    }

    #[test]
    fn a_menu_rises_clear_of_the_player() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = window();
        let chrome = ui.global::<crate::Chrome>();
        chrome.set_window_height(820.0);
        assert_eq!(chrome.invoke_menu_lift(200.0, 300.0), 0.0, "room below");
        assert_eq!(chrome.invoke_menu_lift(700.0, 200.0), 176.0, "its foot meets the player pill's top");
        assert_eq!(chrome.invoke_menu_lift(300.0, 900.0), 224.0, "taller than the room: its top meets the top pill");
    }

    #[test]
    fn playback_only_moves_the_visible_player() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = window();
        let player = pill();
        ui.set_display_position_ms(10_000);
        position_shown(&ui, Some(&player), 20_000);
        assert_eq!(ui.get_position_ms(), 20_000, "the control socket keeps the current position");
        assert_eq!(player.get_position_ms(), 20_000);
        assert_eq!(ui.get_display_position_ms(), 10_000, "the hidden fullscreen layout stays still");
        ui.set_full_player(true);
        position_shown(&ui, Some(&player), 30_000);
        assert_eq!(ui.get_display_position_ms(), 30_000, "opening fullscreen shows its current position");
        assert_eq!(player.get_position_ms(), 20_000, "the hidden compact controls stay still");
        ui.set_full_player(false);
        position_shown(&ui, None, 40_000);
        assert_eq!(ui.get_display_position_ms(), 40_000, "an embedded player follows playback");
    }

    // The glyphs drawn in `columns` x `rows` of a snapshot, left to right, as (first column, last column,
    // first row, last row): runs of columns with a pixel apart from the area's top-left one.
    fn glyphs(shot: &SharedPixelBuffer<Rgba8Pixel>, columns: std::ops::Range<usize>, rows: std::ops::Range<usize>) -> Vec<(usize, usize, usize, usize)> {
        let width = shot.width() as usize;
        let px = shot.as_slice();
        let ground = px[rows.start * width + columns.start];
        let inked = |x: usize, y: usize| {
            let p = px[y * width + x];
            (p.r as i32 - ground.r as i32).abs() + (p.g as i32 - ground.g as i32).abs() + (p.b as i32 - ground.b as i32).abs() > 60
        };
        let mut found: Vec<(usize, usize, usize, usize)> = Vec::new();
        for x in columns {
            let inked_rows: Vec<usize> = rows.clone().filter(|&y| inked(x, y)).collect();
            let (Some(&top), Some(&bottom)) = (inked_rows.first(), inked_rows.last()) else { continue };
            match found.last_mut() {
                Some(g) if g.1 + 1 == x => *g = (g.0, x, g.2.min(top), g.3.max(bottom)),
                _ => found.push((x, x, top, bottom)),
            }
        }
        found
    }

    #[test]
    fn lighting_a_panel_tab_moves_no_other() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_devices_on(true);
        ui.show().unwrap();
        // The tabs head the side sheet; their glyphs' tops sit above the names.
        let glyph_tops = |panel: i32| {
            ui.set_inspector(panel);
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            let shot = ui.window().take_snapshot().unwrap();
            let top = glyphs(&shot, 960..1250, 80..112).iter().map(|g| g.2).min().unwrap();
            glyphs(&shot, 960..1250, top..top + 3).iter().map(|g| (g.0 + g.1) as f32 / 2.0).collect::<Vec<_>>()
        };
        let devices = glyph_tops(3);
        assert!(devices.len() >= 3, "{devices:?}");
        for panel in [2, 1] {
            let lit = glyph_tops(panel);
            assert_eq!(lit.len(), devices.len(), "panel {panel}");
            assert!(lit.iter().zip(&devices).all(|(a, b)| (a - b).abs() <= 1.0), "panel {panel}: {lit:?} against {devices:?}");
        }
    }

    #[test]
    fn the_pills_seal_matches_the_output_beside_it() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        for dark in [false, true] {
            let bar = crate::PlayerBar::new().unwrap();
            crate::look::plain(&nori_core::settings::StoredPrefs::default(), dark, crate::look::Today { month: 10, day: 19, southern: false }).paint(&bar.global::<Theme>());
            bar.window().set_size(slint::PhysicalSize::new(1200, 64));
            bar.set_room(1200.0);
            bar.set_has_song(true);
            bar.set_devices_on(true);
            bar.show().unwrap();
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            let shot = bar.window().take_snapshot().unwrap();
            // Right of the transport: where the sound goes, then the seal.
            let found = glyphs(&shot, 840..1200, 8..56);
            let (speaker, seal) = (found[0], found[1]);
            let height = |g: (usize, usize, usize, usize)| g.3 - g.2 + 1;
            assert!(height(seal).abs_diff(height(speaker)) <= 1, "dark {dark}: seal {seal:?}, speaker {speaker:?}");
            // A glyph's ink: its pixel furthest from the pill under it.
            let ground = shot.as_slice()[8 * 1200 + 840];
            let apart = |p: Rgba8Pixel, q: Rgba8Pixel| (p.r as i32 - q.r as i32).abs() + (p.g as i32 - q.g as i32).abs() + (p.b as i32 - q.b as i32).abs();
            let ink = |g: (usize, usize, usize, usize)| (g.2..=g.3).flat_map(|y| (g.0..=g.1).map(move |x| (x, y)))
                .map(|(x, y)| shot.as_slice()[y * 1200 + x]).max_by_key(|&p| apart(p, ground)).unwrap();
            assert!(apart(ink(seal), ink(speaker)) <= 24, "dark {dark}: seal {:?}, speaker {:?}", ink(seal), ink(speaker));
        }
    }

    #[test]
    fn the_pills_transport_stays_put_whatever_the_song_says() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let bar = pill();
        let (w, h) = (1000u32, 64u32);
        bar.window().set_size(slint::PhysicalSize::new(w, h));
        bar.set_room(w as f32);
        bar.set_has_song(true);
        bar.show().unwrap();
        // The middle third, which only the transport and the seek bar draw into.
        let middle = |title: &str, artist: &str, album: &str| {
            bar.set_now_title(title.into());
            bar.set_now_artist(artist.into());
            bar.set_now_album(album.into());
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            let shot = bar.window().take_snapshot().unwrap();
            let px = shot.as_slice();
            (0..h as usize).flat_map(|y| px[y * w as usize + w as usize / 3..y * w as usize + 2 * w as usize / 3].to_vec()).collect::<Vec<_>>()
        };
        let with_album = middle("Paper Boats", "fakemink", "Some Album");
        assert!(with_album == middle("Paper Boats", "fakemink", ""), "a song without an album");
        assert!(with_album == middle("A Long Title (Official Music Video) [Remastered]", "One, Two & Three", "A Long Album Name"), "a long song");
    }

    #[test]
    fn the_top_pills_search_is_typed_in_where_it_is() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let bar = TopBar::new().unwrap();
        bar.window().set_size(slint::PhysicalSize::new(1000, 48));
        bar.set_room(1000.0);
        bar.show().unwrap();
        let typed = Rc::new(RefCell::new(Vec::<String>::new()));
        let typing = Rc::new(std::cell::Cell::new(false));
        let t = typed.clone();
        bar.on_search_edited(move |s| t.borrow_mut().push(s.into()));
        let k = typing.clone();
        bar.on_typing(move |on| k.set(on));
        let settle = || {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(300));
            slint::platform::update_timers_and_animations();
        };
        let key = |text: &str| {
            bar.window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.into() });
            bar.window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: text.into() });
        };
        // The capsule sits left of the settings button at the pill's right end.
        let at = slint::LogicalPosition::new(900.0, 24.0);
        let button = slint::platform::PointerEventButton::Left;
        bar.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position: at, button });
        bar.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: at, button });
        settle();
        assert!(typing.get(), "a click gives the field the keys");
        key("a");
        key("b");
        assert_eq!(bar.get_search_text(), "ab");
        assert_eq!(typed.borrow().last().map(String::as_str), Some("ab"));
        key(&SharedString::from(slint::platform::Key::Escape));
        settle();
        assert!(!typing.get(), "Escape gives the keys back to the page");
    }

    #[test]
    fn volume_and_seek_animate_between_updates() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let bar = pill();
        bar.window().set_size(slint::PhysicalSize::new(1000, 64));
        bar.set_room(1000.0);
        bar.set_has_song(true);
        bar.set_duration_ms(200_000);
        bar.set_position_ms(50_000);
        bar.set_volume(0.2);
        bar.show().unwrap();
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        bar.window().take_snapshot().unwrap();
        bar.set_position_ms(50_100);
        slint::platform::update_timers_and_animations();
        bar.window().take_snapshot().unwrap();
        assert!(!bar.window().has_active_animations(), "subpixel playback does not animate a full layer every frame");
        for seek in [false, true] {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            let before = bar.window().take_snapshot().unwrap();
            if seek { bar.set_position_ms(0); } else { bar.set_volume(0.8); }
            let first = bar.window().take_snapshot().unwrap();
            i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(90));
            slint::platform::update_timers_and_animations();
            let middle = bar.window().take_snapshot().unwrap();
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            let after = bar.window().take_snapshot().unwrap();
            assert!(first.as_slice() != after.as_slice(), "the bar must start from its previous value, seek={seek}");
            assert!(middle.as_slice() != before.as_slice() && middle.as_slice() != after.as_slice(), "the bar must draw an intermediate value, seek={seek}");
        }
    }

    #[test]
    fn mixing_is_only_visible_in_fullscreen() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_mixing_label(words::MIXING.into());
    ui.set_login_kinds(ModelRc::new(VecModel::from(nori_core::providers::providers().iter().map(login_kind).collect::<Vec<_>>())));
        ui.set_has_song(true);
        ui.set_playing(true);
        ui.set_now_title("Track".into());
        ui.show().unwrap();
        for full in [false, true] {
            ui.set_full_player(full);
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            let before = ui.window().take_snapshot().unwrap();
            ui.set_mixing(true);
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            let during = ui.window().take_snapshot().unwrap();
            assert_eq!(before.as_slice() != during.as_slice(), full, "mixing is shown below the fullscreen seek bar");
            ui.set_mixing(false);
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            let after = ui.window().take_snapshot().unwrap();
            assert!(before.as_slice() == after.as_slice(), "the indicator disappears when the transition finishes");
        }
    }

    #[test]
    fn fullscreen_does_not_draw_the_hidden_page() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_view(ALBUMS);
        ui.set_albums(ModelRc::new(VecModel::from(vec![crate::Card { art: "page".into(), ..Default::default() }; 50])));
        ui.set_now_art("playing".into());
        let requests = Rc::new(RefCell::new(Vec::<String>::new()));
        let called = requests.clone();
        ui.on_art(move |id, _, _| {
            called.borrow_mut().push(id.to_string());
            Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(16, 16))
        });
        ui.show().unwrap();
        ui.window().take_snapshot().unwrap();
        ui.set_full_player(true);
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        ui.window().take_snapshot().unwrap();
        requests.borrow_mut().clear();
        ui.set_covers_rev(1);
        ui.window().take_snapshot().unwrap();
        assert!(requests.borrow().iter().any(|id| id == "playing"));
        assert!(!requests.borrow().iter().any(|id| id == "page"), "the opaque player covers the page, so its covers must not be drawn");
        requests.borrow_mut().clear();
        ui.set_full_player(false);
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        ui.window().take_snapshot().unwrap();
        assert!(requests.borrow().iter().any(|id| id == "page"), "the page returns when the player closes");
    }

    #[test]
    fn album_views_release_offscreen_covers() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        for view in [ALBUMS, HOME, 10] {
            let ui = window();
            ui.window().set_size(slint::PhysicalSize::new(1280, 820));
            ui.set_view(view);
            ui.set_picks_loaded(true);
            let cards = ModelRc::new(VecModel::from((0..500).map(|i| crate::Card {
                id: i.to_string().into(), art: i.to_string().into(), title: format!("Album {i}").into(), ..Default::default()
            }).collect::<Vec<_>>()));
            ui.set_albums(cards.clone());
            ui.set_shelves(ModelRc::new(VecModel::from(vec![crate::Shelf { loaded: true, cards, ..Default::default() }])));
            let requests = Rc::new(RefCell::new(Vec::<String>::new()));
            let called = requests.clone();
            ui.on_art(move |id, _, _| {
                called.borrow_mut().push(id.to_string());
                Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(16, 16))
            });
            ui.show().unwrap();
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
            assert!(requests.borrow().iter().any(|id| id == "0"));
            let initial = requests.borrow().iter().filter(|id| id.as_str() == "0").count();
            let position = slint::LogicalPosition::new(600.0, if view == HOME { 250.0 } else { 400.0 });
            let (delta_x, delta_y) = if view == HOME { (-6000.0, 0.0) } else { (0.0, -3000.0) };
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled { position, delta_x, delta_y });
            ui.window().take_snapshot().unwrap();
            assert!(requests.borrow().iter().any(|id| id.parse::<usize>().is_ok_and(|i| i > 20)), "later albums are drawn in view {view}: {:?}", requests.borrow());
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled { position, delta_x: -delta_x, delta_y: -delta_y });
            ui.window().take_snapshot().unwrap();
            assert!(requests.borrow().iter().filter(|id| id.as_str() == "0").count() > initial, "returning to a row must load it again after its offscreen component was released");
        }
    }

    #[test]
    fn show_all_opens_scrollable_collections() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        for view in [10, 11] {
            let ui = window();
            ui.window().set_size(slint::PhysicalSize::new(1280, 820));
            ui.set_picks_loaded(true);
            ui.set_show_all_text(words::SHOW_ALL.into());
    ui.set_mixing_label(words::MIXING.into());
    ui.set_login_kinds(ModelRc::new(VecModel::from(nori_core::providers::providers().iter().map(login_kind).collect::<Vec<_>>())));
            ui.set_shelves(ModelRc::new(VecModel::from(vec![Shelf {
                title: "Recently played".into(), loaded: true,
                cards: cards((0..100).map(|i| Card { id: i.to_string().into(), art: i.to_string().into(), ..Default::default() })),
            }])));
            if view == 11 {
                ui.set_picks(ModelRc::new(VecModel::from((0..100).map(|i| crate::Pick {
                    id: i.to_string().into(), covers: ModelRc::new(VecModel::from(vec![i.to_string().into()])), ..Default::default()
                }).collect::<Vec<_>>())));
            }
            let weak = ui.as_weak();
            ui.on_go(move |view| weak.unwrap().set_view(view));
            let weak = ui.as_weak();
            ui.on_back(move || weak.unwrap().set_view(HOME));
            let requests = Rc::new(RefCell::new(Vec::new()));
            let seen = requests.clone();
            ui.on_art(move |id, _, _| { seen.borrow_mut().push(id.to_string()); Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(16, 16)) });
            let seen = requests.clone();
            ui.on_pick_art(move |ids, _| { seen.borrow_mut().push(ids.row_data(0).unwrap().to_string()); Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(16, 16)) });
            ui.show().unwrap();
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            let frame = ui.window().take_snapshot().unwrap();
            let dim = ui.global::<Theme>().get_dim().to_argb_u8();
            let links: Vec<_> = frame.as_slice().iter().enumerate().filter(|(i, p)| i % 1280 > 1100 && (100..320).contains(&(i / 1280)) && p.r.abs_diff(dim.red) < 24 && p.g.abs_diff(dim.green) < 24 && p.b.abs_diff(dim.blue) < 24).map(|(i, _)| (i % 1280, i / 1280)).collect();
            assert!(!links.is_empty(), "Show all is visible in the shelf header");
            let position = slint::LogicalPosition::new(links[links.len() / 2].0 as f32, links[links.len() / 2].1 as f32);
            for event in [
                slint::platform::WindowEvent::PointerPressed { position, button: slint::platform::PointerEventButton::Left },
                slint::platform::WindowEvent::PointerReleased { position, button: slint::platform::PointerEventButton::Left },
            ] { ui.window().dispatch_event(event); }
            assert_eq!(ui.get_view(), view, "the shelf's Show all link opens its collection");
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
            requests.borrow_mut().clear();
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(600.0, 400.0), delta_x: 0.0, delta_y: -3000.0,
            });
            ui.window().take_snapshot().unwrap();
            assert!(requests.borrow().iter().any(|id| id.parse::<usize>().is_ok_and(|i| i > 20)), "the collection reveals later items by scrolling vertically");
            let position = slint::LogicalPosition::new(40.0, 100.0);
            for event in [
                slint::platform::WindowEvent::PointerPressed { position, button: slint::platform::PointerEventButton::Left },
                slint::platform::WindowEvent::PointerReleased { position, button: slint::platform::PointerEventButton::Left },
            ] { ui.window().dispatch_event(event); }
            assert_eq!(ui.get_view(), HOME, "the back button returns to Home");
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
        }
    }

    #[test]
    fn shelves_drag_with_a_mouse() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_view(HOME);
        ui.set_picks_loaded(true);
        ui.set_shelves(ModelRc::new(VecModel::from(vec![Shelf {
            loaded: true, cards: cards((0..60).map(|i| Card { id: i.to_string().into(), art: i.to_string().into(), ..Default::default() })), ..Default::default()
        }])));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let seen = requests.clone();
        ui.on_art(move |id, _, _| { seen.borrow_mut().push(id.to_string()); Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(16, 16)) });
        ui.show().unwrap();
        ui.window().take_snapshot().unwrap();
        let initial = requests.borrow().iter().filter_map(|id| id.parse::<usize>().ok()).max().unwrap();
        let position = slint::LogicalPosition::new(1000.0, 250.0);
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button: slint::platform::PointerEventButton::Left });
        for x in [900.0, 650.0, 300.0] {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(20));
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(x, 250.0) });
        }
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: slint::LogicalPosition::new(300.0, 250.0), button: slint::platform::PointerEventButton::Left });
        ui.window().take_snapshot().unwrap();
        assert!(requests.borrow().iter().any(|id| id.parse::<usize>().is_ok_and(|i| i > initial)), "dragging must reveal cards beyond the original viewport");
    }

    #[test]
    fn home_keeps_pick_covers() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1800, 1100));
        ui.set_view(HOME);
        ui.set_picks_loaded(true);
        ui.set_picks(ModelRc::new(VecModel::from((0..6).map(|i| crate::Pick {
            seed: Color::from_rgb_u8(255, 0, 0), page: Color::from_rgb_u8(0, 0, 255),
            covers: ModelRc::new(VecModel::from((0..4).map(|j| format!("pick-{i}-{j}").into()).collect::<Vec<_>>())),
            ..Default::default()
        }).collect::<Vec<_>>())));
        ui.set_shelves(ModelRc::new(VecModel::from((0..20).map(|i| Shelf {
            loaded: true, cards: cards((0..20).map(|j| Card { art: format!("album-{i}-{j}").into(), ..Default::default() })),
            ..Default::default()
        }).collect::<Vec<_>>())));
        ui.set_has_song(true);
        ui.set_now_art("playing-cover".into());
        ui.set_queue(ModelRc::new(VecModel::from((0..500).map(|i| SongRow {
            index: i, art: format!("queue-{i}").into(), title: format!("Song {i}").into(),
            ..Default::default()
        }).collect::<Vec<_>>())));
        let art = Rc::new(RefCell::new(Art::default()));
        let requested = Rc::new(RefCell::new(HashSet::new()));
        let seen = requested.clone();
        let cache = art.clone();
        ui.on_art(move |id, size, _| {
            seen.borrow_mut().insert(id.to_string());
            cover_image(&cache, &AppHandle(Weak::new()), id, size)
        });
        let cache = art.clone();
        ui.on_pick_art(move |ids, _| pick_image(&cache, &AppHandle(Weak::new()), ids, mosaic));
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(SMALL_PX, SMALL_PX);
        pixels.make_mut_slice().fill(Rgba8Pixel { r: 27, g: 91, b: 173, a: 255 });
        let image = Image::from_rgba8(pixels);
        ui.show().unwrap();
        ui.window().take_snapshot().unwrap();
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        let load_covers = || {
            for _ in 0..4 {
                ui.set_covers_rev(ui.get_covers_rev() + 1);
                ui.window().take_snapshot().unwrap();
                let pending: Vec<_> = art.borrow_mut().wanted.drain(..).collect();
                for key in pending {
                    art.borrow_mut().insert(key, image.clone(), None, Instant::now());
                }
            }
            ui.set_covers_rev(ui.get_covers_rev() + 1);
            ui.window().take_snapshot().unwrap();
        };
        load_covers();
        assert!(art.borrow().wanted.is_empty(), "Home must keep loaded covers while stationary: {} covers requested again", art.borrow().wanted.len());
        let cache = art.borrow();
        assert!(cache.images.keys().any(|key| key.id.starts_with("pick-0-")), "the first pick loads its artwork");
        assert!(!cache.images.keys().any(|key| key.id.starts_with("album-19-")), "offscreen shelves must not load covers");
        drop(cache);
        assert!(!requested.borrow().iter().any(|id| id.starts_with("queue-")), "the closed queue must not request artwork");
        for full in [false, true] {
            ui.set_inspector(if full { 0 } else { 1 });
            ui.set_full_panel(1);
            ui.set_full_player(full);
            ui.window().take_snapshot().unwrap();
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            requested.borrow_mut().clear();
            load_covers();
            assert!(art.borrow().wanted.is_empty(), "an open queue keeps visible covers cached");
            assert!(requested.borrow().contains("queue-0"), "the queue loads its first cover");
            assert!(!requested.borrow().contains("queue-499"), "offscreen queue artwork must not evict visible covers");
            requested.borrow_mut().clear();
            let position = slint::LogicalPosition::new(if full { 1100.0 } else { 1600.0 }, 500.0);
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position, delta_x: 0.0, delta_y: -30_000.0,
            });
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            load_covers();
            assert!(requested.borrow().contains("queue-499"), "scrolling the queue loads its last cover, full={full}");
            ui.set_full_player(false);
            ui.set_inspector(0);
            ui.window().take_snapshot().unwrap();
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            requested.borrow_mut().clear();
            load_covers();
            assert!(!requested.borrow().iter().any(|id| id.starts_with("queue-")), "closing either queue releases its artwork bindings");
        }

        ui.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(1780.0, 950.0), delta_x: 0.0, delta_y: -30_000.0,
        });
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        ui.window().take_snapshot().unwrap();
        assert!(art.borrow().wanted.iter().any(|key| key.id.starts_with("album-19-")), "scrolling reveals the last shelf's covers");
    }

    #[test]
    fn pick_covers_have_no_background_seams() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.set_view(HOME);
        ui.set_inspector(3);
        ui.set_picks_loaded(true);
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(16, 16);
        pixels.make_mut_slice().fill(Rgba8Pixel { r: 255, g: 255, b: 255, a: 255 });
        let image = Image::from_rgba8(pixels);
        let pick = mosaic(&vec![image.clone(); 4]).unwrap();
        ui.on_pick_art(move |_, _| pick.clone());
        ui.on_art(move |_, _, _| image.clone());
        let pick = crate::Pick { seed: Color::from_rgb_u8(255, 0, 0), page: Color::from_rgb_u8(0, 0, 255), covers: ModelRc::new(VecModel::from(vec!["cover".into(); 4])), ..Default::default() };
        ui.set_picks(ModelRc::new(VecModel::from(vec![pick; 2])));
        ui.show().unwrap();
        for (logical_width, scale) in [(1280, 1.0), (1261, 1.0), (1258, 1.0), (1280, 1.25), (1261, 1.5), (1280, 2.0)] {
            ui.window().dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: scale });
            let width = (logical_width as f32 * scale) as u32;
            ui.window().set_size(slint::PhysicalSize::new(width, (820.0 * scale) as u32));
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            let frame = ui.window().take_snapshot().unwrap();
            let pixels = frame.as_slice();
            let white = |p: &Rgba8Pixel| p.r == 255 && p.g == 255 && p.b == 255;
            let top = (0..frame.height() as usize).find(|&y| white(&pixels[y * width as usize + (150.0 * scale) as usize])).expect("the pick is visible");
            let line = &pixels[(top + 20) * width as usize..(top + 21) * width as usize];
            let left = line.iter().position(white).unwrap();
            let right = line.iter().rposition(white).unwrap();
            // The rightmost white is in the second card; each card fills half the shelf.
            let side = (right - left - (22.0 * scale) as usize).div_ceil(2);
            for y in top + (14.0 * scale) as usize..top + side * 45 / 100 {
                for x in left + 2..left + side - 2 {
                    let p = pixels[y * width as usize + x];
                    assert!(p.r.abs_diff(p.g) <= 1, "background shows through at {x},{y}, width {width}: {p:?}");
                }
            }
        }
    }

    #[test]
    fn pick_covers_scroll_together() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_view(HOME);
        ui.set_picks_loaded(true);
        let images: Vec<Image> = [(255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 0)].into_iter().map(|(r, g, b)| {
            let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(16, 16);
            pixels.make_mut_slice().fill(Rgba8Pixel { r, g, b, a: 255 });
            Image::from_rgba8(pixels)
        }).collect();
        let pick = mosaic(&images).unwrap();
        ui.on_pick_art(move |_, _| pick.clone());
        ui.on_art(move |id, _, _| images[id.as_str().parse::<usize>().unwrap_or(0)].clone());
        let pick = crate::Pick { title: "Mix".into(), covers: ModelRc::new(VecModel::from(vec!["0".into(), "1".into(), "2".into(), "3".into()])), ..Default::default() };
        ui.set_picks(ModelRc::new(VecModel::from(vec![pick; 6])));
        let card = crate::Card { art: "0".into(), ..Default::default() };
        ui.set_shelves(ModelRc::new(VecModel::from((0..3).map(|_| crate::Shelf { loaded: true, cards: ModelRc::new(VecModel::from(vec![card.clone()])), ..Default::default() }).collect::<Vec<_>>())));
        ui.show().unwrap();
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        let profile = |ui: &AppWindow| {
            let frame = ui.window().take_snapshot().unwrap();
            let pixels = frame.as_slice();
            let x = 100;
            let top = (0..frame.height() as usize).find(|&y| { let p = pixels[y * frame.width() as usize + x]; p.r == 255 && p.g == 0 && p.b == 0 }).expect("the first cover is visible");
            let left = (0..frame.width() as usize).find(|&x| { let p = pixels[(top + 20) * frame.width() as usize + x]; p.r == 255 && p.g == 0 && p.b == 0 }).expect("the left edge is visible");
            (0..180).map(|dy| pixels[(top + dy) * frame.width() as usize + x])
                .chain((0..180).map(|dx| pixels[(top + 20) * frame.width() as usize + left + dx])).collect::<Vec<_>>()
        };
        let before = profile(&ui);
        for (position, delta_x, delta_y) in [(slint::LogicalPosition::new(1258.0, 600.0), 0.0, -0.25), (slint::LogicalPosition::new(800.0, 250.0), -0.25, 0.0)] {
            for _ in 0..4 {
                ui.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled { position, delta_x, delta_y });
                let after = profile(&ui);
                // Gradient dithering may change a channel by one.
                assert!(before.iter().zip(after).all(|(a, b)| a.r.abs_diff(b.r) <= 1 && a.g.abs_diff(b.g) <= 1 && a.b.abs_diff(b.b) <= 1), "the mosaic must retain its seams and gradient while scrolling");
            }
        }
    }

    #[test]
    fn device_updates_keep_the_picker_rows() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = window();
        let here = crate::DeviceRow { name: "This computer".into(), active: true, ..Default::default() };
        let phone = crate::DeviceRow { id: "phone".into(), name: "Phone".into(), ..Default::default() };
        devices_set(&ui, vec![here.clone(), phone.clone()]);
        let model = ui.get_devices();
        devices_set(&ui, vec![here.clone(), phone.clone()]);
        assert!(model == ui.get_devices(), "a remote update must preserve a mouse press on a device row");
        let picked = crate::DeviceRow { active: true, ..phone };
        devices_set(&ui, vec![crate::DeviceRow { active: false, ..here.clone() }, picked.clone()]);
        assert!(model == ui.get_devices());
        assert_eq!(model.row_data(1), Some(picked));
        devices_set(&ui, vec![here.clone()]);
        assert!(model == ui.get_devices(), "a disconnected device leaves the existing picker");
        assert_eq!(model.row_count(), 1);
        assert_eq!(model.row_data(0), Some(here));
    }
}
