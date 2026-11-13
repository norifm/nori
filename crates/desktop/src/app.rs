//! Window state and UI callbacks, on Slint's event loop thread. Worker results arrive as [`Msg`]s through
//! the inbox ([`session::Tx`]). The only repeating timer is the seek bar's, and only while playing.

use nori_host::desktop::Root;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use nori_core::browse::AlbumSort;
use nori_core::lyrics_follow::LyricsNow;
use nori_core::menus::PageKind;
use nori_core::offer::Offered;
use nori_core::paged::Paged;
use nori_core::search::{SearchShown, SearchView};
use nori_core::settings::SavedServer;
use nori_core::Song;
use nori_covers::loader::Ticket;
use nori_covers::memory::Image as Picture;
use nori_engine::{Event, State};
use nori_host::remote::Elsewhere;
use nori_http::Http;
use nori_look::cover::CoverColours;
use slint::{Color, ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode, VecModel};

use crate::compositor::{Compositor, Focus, Raised};
use crate::session::{self, CoverKey, CoverSize, Data, Fetch, Msg, PlaylistChange, Req, Session, Tx};
use nori_host::pages::{HomePart, PageAsk, PageGot, ShelfItems};
use nori_host::player::{Playback, QueueShown};
use crate::settings::{Act, Target};
use crate::sung::Part;
use crate::words;
use nori_core::providers::flow::{AddDo, AddEvent, AddFlow, AddPage, SignInEvent, SignInFlow, SignInShown};
use nori_core::providers::AfterAdding;

use crate::{AppWindow, ArtistLink, Card, Credits, Detail, IdentityLine, LibraryTabs, LoginKind, LyricPiece, Motion, PageLayout, Panel, Pick, PlayerBar, PlaylistChoice, Shelf, SongGo, SongPlace, SongRow, TopBar, View, WorkCard};

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

/// The way back from the page shown: the pages opened from one another before it, and the tab the first
/// of them was opened from, each with where it was scrolled; and the pages Back left, for Forward.
#[derive(Default)]
struct PageTrail {
    /// The page shown, and those before it, oldest first.
    shown: Option<PageAsk>,
    before: Vec<(PageAsk, f32)>,
    tab: View,
    tab_at: f32,
    /// The pages Back left, the nearest last.
    ahead: Vec<(PageAsk, f32)>,
}

/// Where Back leads.
#[derive(Debug, PartialEq)]
enum Back {
    Page(PageAsk),
    Tab(View),
}

impl PageTrail {
    /// `ask` is shown, opened from tab `from` (or from the page shown, when `from` is the page view) scrolled
    /// to `at`.
    fn opened(&mut self, ask: PageAsk, from: View, at: f32) {
        if from == View::Page {
            self.before.extend(self.shown.take().map(|page| (page, at)));
        } else {
            self.before.clear();
            self.tab = from;
            self.tab_at = at;
        }
        self.ahead.clear();
        self.shown = Some(ask);
    }

    /// Back from the page shown, scrolled to `at`: the page before it (now shown), else the tab it was opened
    /// from, and where that was scrolled.
    fn back(&mut self, at: f32) -> (Back, f32) {
        self.ahead.extend(self.shown.take().map(|page| (page, at)));
        match self.before.pop() {
            Some((page, was)) => {
                self.shown = Some(page.clone());
                (Back::Page(page), was)
            }
            None => (Back::Tab(self.tab), self.tab_at),
        }
    }

    /// Forward from what is shown, scrolled to `at`: the page Back last left, and where it was scrolled.
    fn forward(&mut self, at: f32) -> Option<(PageAsk, f32)> {
        let (page, was) = self.ahead.pop()?;
        match self.shown.replace(page.clone()) {
            Some(left) => self.before.push((left, at)),
            None => self.tab_at = at,
        }
        Some((page, was))
    }

    /// A tab is chosen: Forward has nothing left to return to.
    fn left(&mut self) {
        self.ahead.clear();
    }
}

/// The view Back returns to from a page opened on `view`. Search keeps its field and answer while a page
/// is open, so Back finds them as they were.
fn opened_from(view: View) -> View {
    if view == View::Login { View::Home } else { view }
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
            Err(_) => nori_core::alog::info(&format!("a callback came while the window's state was busy ({}); dropped", std::panic::Location::caller())),
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

/// The kinds of item plugins add menu actions to, by `Card::kind`.
const MENU_KINDS: [nori_plugins::ItemKind; 4] = [nori_plugins::ItemKind::Album, nori_plugins::ItemKind::Artist, nori_plugins::ItemKind::Playlist, nori_plugins::ItemKind::Song];
/// A song menu line's kind (`SongGo::kind`) for plugin action 0; the next ones follow.
const PLUGIN_MENU: i32 = 100;

/// Library lists as loaded, before the find filter.
#[derive(Default)]
struct Lists {
    albums: Vec<nori_core::Album>,
    artists: Vec<Card>,
    playlists: Vec<nori_core::Playlist>,
}

/// Decoded covers, looked up by the `art` callback while drawing. A missing cover is requested once;
/// its arrival bumps `covers-rev` so pictures look again. Least recently drawn are evicted first.
#[derive(Default)]
struct Art {
    images: HashMap<CoverKey, (Image, Instant)>,
    mosaics: HashMap<Vec<String>, Mosaic>,
    /// Page colours of large covers, by cover id.
    colours: HashMap<String, Rc<CoverColours>>,
    /// The covers waiting for a ticket (`wanted`) or for their ticket's answer (`tickets`).
    asked: HashSet<CoverKey>,
    wanted: Vec<CoverKey>,
    /// Pending cover requests, oldest first; dropping a ticket cancels the fetch.
    tickets: VecDeque<(CoverKey, Ticket)>,
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
    /// Requests the covers found missing with `request`, which gives no ticket when none can be fetched
    /// now: those are forgotten, to be asked again when next drawn. Requests for covers off screen since
    /// `now - GONE` are cancelled.
    fn ask(&mut self, now: Instant, mut request: impl FnMut(&CoverKey) -> Option<Ticket>) {
        let wanted = std::mem::take(&mut self.wanted);
        let (asked, missing) = (&mut self.asked, &mut self.missing);
        self.tickets.retain(|(k, _)| {
            let shown = missing.get(k).is_some_and(|t| now.duration_since(*t) < GONE);
            if !shown {
                asked.remove(k);
                missing.remove(k);
            }
            shown
        });
        // The loader serves newest first; reversed so the top left of a frame comes first.
        for k in wanted.into_iter().rev() {
            match request(&k) {
                Some(t) => self.tickets.push_back((k, t)),
                None => {
                    self.asked.remove(&k);
                }
            }
        }
        // Dropping a ticket cancels it; the cover may be requested again.
        while self.tickets.len() > PENDING_KEPT {
            let (k, _) = self.tickets.pop_front().expect("longer than the limit");
            self.asked.remove(&k);
            self.missing.remove(&k);
        }
    }

    fn insert(&mut self, key: CoverKey, image: Image, colours: Option<Box<CoverColours>>, now: Instant) {
        let id = key.id.clone();
        let large = key.size != CoverSize::Card;
        self.missing.remove(&key);
        self.tickets.retain(|(k, _)| *k != key);
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

/// Drops the mosaics no tile in `tiles` shows; those still shown stay drawn.
fn keep_shown_mosaics(art: &mut Art, tiles: &[nori_core::mixes::board::MixTile]) {
    art.mosaics.retain(|ids, _| tiles.iter().any(|t| t.covers.iter().take(4).eq(ids.iter())));
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
    let mut w = w.clone();
    if let Some(d) = dye {
        d.dye_wash(&mut w);
    }
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(side, side);
    for (px, argb) in buf.make_mut_slice().iter_mut().zip(&w) {
        let [a, r, g, b] = argb.to_be_bytes();
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
    /// The look it fades from, and when it began.
    from: Option<(crate::look::Look, Instant)>,
    /// What was painted last.
    shown: Option<crate::look::Look>,
    timer: Timer,
}

const FADE: Duration = Duration::from_millis(340);

impl Fade {
    fn start(&mut self, from: crate::look::Look) {
        self.from = Some((from, Instant::now()));
    }

    /// The look on the way to `to` now, and whether it has arrived.
    fn at(&self, to: &crate::look::Look) -> (crate::look::Look, bool) {
        let Some((from, began)) = &self.from else { return (to.clone(), true) };
        let t = (began.elapsed().as_secs_f32() / FADE.as_secs_f32()).min(1.0);
        (from.towards(to, t * t * (3.0 - 2.0 * t)), t >= 1.0)
    }
}

/// A song's moving cover, and what plays it while the sleeve shows.
struct Moving {
    song: Song,
    artwork: nori_plugins::Artwork,
    kept: crate::motion::Kept,
    playing: Option<crate::motion::Playing>,
}

/// The page shown (the page view's, or the artist beside the Artists list), and what its read brought.
enum PageShown {
    Album,
    Artist,
    /// The playlist, once read, while the profile may change it (the core's `playlist_editable`).
    Playlist(Option<nori_core::Playlist>),
    /// The mix's id and name, once read: it can be drawn again and kept as a playlist.
    Mix(Option<(String, String)>),
}

/// What the queue's undo would put back.
enum Taken {
    /// A song taken out, by id.
    Song(String),
    /// What a clear took.
    Cleared,
}

pub struct App {
    me: AppHandle,
    ui: slint::Weak<AppWindow>,
    compositor: Compositor,
    tx: Tx,
    inbox: mpsc::Receiver<Msg>,
    /// Media controls for the process; each session drives them while open.
    mpris: Option<Arc<nori_mpris::Mpris>>,
    /// The queue session over the settings, and the plugins.
    root: Arc<Root>,
    http: Arc<Http>,
    session: Option<Session>,
    art: Rc<RefCell<Art>>,
    lyric_faces: Option<crate::sung::Faces>,
    /// Id and record of the song being heard.
    heard: Option<String>,
    song: Option<Song>,
    queue: Option<QueueShown>,
    /// Songs behind the clickable lists: the page's, the Songs view's, the search's.
    page_songs: Vec<Song>,
    /// The artist page's Show all pressed: every song listed, not the first few.
    page_songs_all: bool,
    page_fetch: Option<Fetch>,
    /// The page shown, set with `page_fetch`.
    page: Option<PageShown>,
    /// The queue another device or app left on the server, offered at the top of Home (the core's `queue_offer`).
    offer: Offered,
    /// Home's mixes, whose cards take their first cover's colors once known.
    mixes: Vec<nori_core::mixes::board::MixTile>,
    songs: Vec<Song>,
    search_songs: Vec<Song>,
    /// The search field's preview of the answer shown, line by line as the main window lists it.
    search_preview: Vec<nori_core::search::PreviewLine>,
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
    /// The code sign-in running while the account sheet shows its code, with its number; and how many
    /// have run.
    code: Option<(u32, Arc<nori_core::providers::code::CodeSignIn>)>,
    codes: u32,
    /// The reads the view shown waits for: their answers end its loading and show their failure.
    want: Vec<Req>,
    shelves: Rc<VecModel<Shelf>>,
    /// The rows of the shelves shown.
    home_rows: Vec<nori_core::settings::HomeRow>,
    /// The saved profile the sign-in form edits; None while it adds one.
    editing: Option<SavedServer>,
    /// The album list's pages, its order, and whether only the favorites.
    album_pages: Paged<u32>,
    album_sort: AlbumSort,
    albums_favorites: bool,
    /// The songs list's pages.
    song_pages: Paged<u32>,
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
    tick: Timer,
    /// The pace the song plays at, for the seek bar's steps.
    pace: f32,
    /// When the app started: the playing bars' clock counts from it.
    started: std::time::Instant,
    /// The window is off screen (minimized, covered): the seek bar, lyrics and playing bars stand still.
    hidden: bool,
    /// Motion is E-ink: the playing bars stand still.
    still: bool,
    /// Lyrics are on screen (Now Playing's lyrics or the side sheet): only then are they fetched.
    lyrics_viewed: bool,
    lists: Lists,
    find: String,
    facts: crate::settings::Facts,
    /// Settings › Plugins as last read.
    plugged: crate::settings::Plugged,
    /// The plugin its install sheet shows.
    preview: Option<nori_plugins::PluginPreview>,
    /// The page adding a profile shows and the profile on its way (the core's).
    add: AddFlow,
    /// The account sheet and the last sign-in (the core's).
    sign_in: SignInFlow,
    /// The profile in use while its plugin doesn't run here, opened once it does.
    waiting: Option<SavedServer>,
    /// The plugins' menu actions by `Card::kind` ([`MENU_KINDS`]).
    plugin_menus: [Vec<nori_plugins::MenuEntry>; 4],
    /// The song whose details are shown, for the plugins' lines that come after.
    details_song: Option<String>,
    /// The plugin's still sleeve being fetched for the song playing.
    sleeve: Option<Ticket>,
    /// The song playing's moving cover.
    moving: Option<Moving>,
    /// Now Playing was open when last seen: its opening asks the plugins' status lines.
    full_seen: bool,
    /// The sound page the session was last told is in sight.
    sound_page: Option<nori_core::tuning::SoundPage>,
    lyrics: Option<crate::lyrics::SongLyrics>,
    lyrics_timer: Timer,
    /// Active lyric line pieces in the side panel and Now Playing, and its backing vocals', updated in
    /// place each frame.
    pieces: [Rc<VecModel<LyricPiece>>; 2],
    backing_pieces: [Rc<VecModel<LyricPiece>>; 2],
    /// Queue rows, edited in place; leaving rows fold first, then the list settles to `queue_next`.
    queue_rows: Rc<VecModel<SongRow>>,
    queue_next: Option<Vec<SongRow>>,
    queue_timer: Timer,
    /// What was last taken out of the queue, while its undo is offered.
    taken: Option<Taken>,
    undo_timer: Timer,
    note: Timer,
    /// Follows what the downloads are doing, each second while they do something.
    work: Timer,
    search: Timer,
    /// The hosted jam's invite link its QR code was drawn for, and whether a jam is hosted.
    jam_link: String,
    jam_hosting: bool,
    /// The song whose menu opened the playlist picker, and the playlists offered for it.
    menu_song: Option<Song>,
    /// The downloads asked to be removed, waiting for the ask sheet's yes.
    removing: Vec<String>,
    picker: Vec<nori_core::menus::PlaylistChoice>,
    /// The sleep timer's choices as offered.
    sleep_choices: Vec<nori_core::menus::SleepChoice>,
    /// Whose playback the player shows and controls, as last read.
    playback: Playback,
    /// A jam guest's controls by its role: what the player offers, and whether it paused here.
    jam_controls: Option<nori_core::remote::JamControls>,
    /// The songs this guest asked for that wait for the host.
    asked: HashSet<String>,
    /// Those the signed-in account may act as, as last read, for the account sheet.
    identities: Vec<nori_core::providers::Identity>,
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

/// Where in the main window a menu asked for at `x` in the player pill opens: above the pill.
fn over_pill(m: &AppWindow, x: f32) -> (f32, f32) {
    let chrome = m.global::<crate::Chrome>();
    (chrome.get_player_x() + x, chrome.get_window_height() - chrome.get_bottom_room())
}

/// The player pill's window; its buttons forward to the main window.
fn player(ui: &AppWindow, art: impl Fn(SharedString, i32) -> Image + 'static) -> Option<PlayerBar> {
    let bar = PlayerBar::new().map_err(|e| nori_core::alog::info(&format!("no player window: {e}"))).ok()?;
    bar.on_art(move |id, size, _rev| art(id, size));
    bar.on_toggle(to_main(ui, |m| m.invoke_toggle()));
    bar.on_star(to_main(ui, |m| m.invoke_star()));
    bar.on_noise_next(to_main(ui, |m| m.invoke_noise_next()));
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
    bar.on_song_menu(to_main_with(ui, |m, x| {
        let (x, y) = over_pill(m, x);
        m.invoke_song_menu_at(x, y);
    }));
    bar.on_album_menu(to_main_with(ui, |m, x| {
        let (x, y) = over_pill(m, x);
        m.invoke_album_menu_at(x, y);
    }));
    let go = bar.global::<crate::Go>();
    go.on_artist(to_main_with(ui, |m, id| m.global::<crate::Go>().invoke_artist(id)));
    go.on_album(to_main_with(ui, |m, id| m.global::<crate::Go>().invoke_album(id)));
    let main = ui.as_weak();
    go.on_artist_menu(move |id, name, x, _| {
        if let Some(m) = main.upgrade() {
            let (x, y) = over_pill(&m, x);
            m.invoke_artist_menu_at(id, name, x, y);
        }
    });
    bar.on_set_inspector(to_main_with(ui, |m, i| {
        m.set_inspector(i);
        m.invoke_player_changed();
    }));
    Some(bar)
}

/// The top pill's window; its tabs and search field forward to the main window.
/// The top pill's library tabs: the sections `rules` has.
fn library_tabs(rules: &nori_core::browse::ProfileRules) -> LibraryTabs {
    use nori_core::settings::LibrarySection::*;
    let has = |s| rules.sections.contains(&s);
    LibraryTabs { albums: has(Albums), artists: has(Artists), songs: has(Songs), playlists: has(Playlists), favorites: has(Favourites) }
}

fn top(ui: &AppWindow) -> Option<TopBar> {
    let bar = TopBar::new().map_err(|e| nori_core::alog::info(&format!("no top pill: {e}"))).ok()?;
    bar.on_go(to_main_with(ui, |m, v| m.invoke_go(v)));
    bar.on_open_profiles(to_main(ui, |m| m.invoke_show_profiles()));
    bar.on_search_edited(to_main_with(ui, |m, text| m.invoke_search_edited(text)));
    bar.on_search_step(to_main_with(ui, |m, by| m.invoke_search_step(by)));
    bar.on_search_accepted(to_main(ui, |m| m.invoke_search_accepted()));
    bar.on_search_escaped(to_main(ui, |m| m.set_search_preview_open(false)));
    let (pill, main) = (bar.as_weak(), ui.as_weak());
    bar.on_search_placed(move || {
        let (Some(bar), Some(ui)) = (pill.upgrade(), main.upgrade()) else { return };
        let chrome = ui.global::<crate::Chrome>();
        chrome.set_search_end(bar.get_search_end());
        chrome.set_search_w(bar.get_search_width());
    });
    Some(bar)
}

/// The album list once its page from `offset` came (`page`), and the page to ask for next: while pages
/// come full and new, each once (`pages`).
fn albums_paged(list: &mut Vec<nori_core::Album>, pages: &mut Paged<u32>, sort: AlbumSort, favorites: bool, offset: u32, page: &[nori_core::Album]) -> Option<u32> {
    let kept = &list[..list.len().min(offset as usize)];
    // A server that ignores the offset would bring the same page without end.
    let repeats = nori_core::browse::album_page_repeats(kept, page);
    let next = nori_core::browse::next_album_page(sort, favorites, offset, nori_host::pages::ALBUM_PAGE, page.len() as u32).filter(|_| !repeats);
    if !pages.answered(&offset, next) {
        return None;
    }
    list.truncate(offset as usize);
    if !repeats {
        list.extend_from_slice(page);
    }
    // Every page is read at once here.
    pages.ask()
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
        kind: info.kind.clone(),
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
pub fn start(ui: &AppWindow, root: Arc<Root>, compositor: Compositor) -> Rc<RefCell<App>> {
    let shelves = Rc::new(VecModel::from(Vec::new()));
    ui.set_shelves(ModelRc::from(shelves.clone()));
    ui.set_greeting(words::LISTEN_NOW.into());
    ui.set_show_all_text(words::SHOW_ALL.into());
    ui.set_mixing_label(words::MIXING.into());
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
            root,
            http: Http::new(),
            session: None,
            art,
            lyric_faces: crate::sung::Faces::inter(),
            heard: None,
            song: None,
            queue: None,
            page_songs: Vec::new(),
            page_songs_all: false,
            page_fetch: None,
            page: None,
            offer: Offered::default(),
            mixes: Vec::new(),
            songs: Vec::new(),
            search_songs: Vec::new(),
            search_preview: Vec::new(),
            search_songs_all: false,
            search_query: String::new(),
            autoeq_query: String::new(),
            autoeq_hits: Vec::new(),
            output: String::new(),
            feed_songs: Vec::new(),
            profiles: Vec::new(),
            sign_in_window: None,
            code: None,
            codes: 0,
            want: Vec::new(),
            shelves,
            home_rows: Vec::new(),
            editing: None,
            album_pages: Paged::new(0),
            song_pages: Paged::new(0),
            album_sort: AlbumSort::ByName,
            albums_favorites: false,
            backdrop: None,
            page_cover: None,
            now_cover: None,
            trail: PageTrail { tab: View::Home, ..Default::default() },
            look: None,
            fade: Fade::default(),
            tick: Timer::default(),
            pace: 1.0,
            started: std::time::Instant::now(),
            hidden: false,
            still: false,
            lyrics_viewed: false,
            facts: crate::settings::Facts::default(),
            plugged: Default::default(),
            preview: None,
            add: AddFlow::default(),
            sign_in: SignInFlow::default(),
            waiting: None,
            plugin_menus: Default::default(),
            details_song: None,
            sleeve: None,
            moving: None,
            full_seen: false,
            lists: Lists::default(),
            find: String::new(),
            sound_page: None,
            lyrics: None,
            lyrics_timer: Timer::default(),
            pieces: [Rc::new(VecModel::default()), Rc::new(VecModel::default())],
            backing_pieces: [Rc::new(VecModel::default()), Rc::new(VecModel::default())],
            queue_rows: Rc::new(VecModel::default()),
            queue_next: None,
            queue_timer: Timer::default(),
            taken: None,
            undo_timer: Timer::default(),
            note: Timer::default(),
            work: Timer::default(),
            search: Timer::default(),
            jam_link: String::new(),
            jam_hosting: false,
            menu_song: None,
            removing: Vec::new(),
            picker: Vec::new(),
            sleep_choices: Vec::new(),
            playback: Playback::Here,
            jam_controls: None,
            asked: HashSet::new(),
            identities: Vec::new(),
        })
    });
    let h = app.borrow().me.clone();
    wire(ui, &h);
    app.borrow().output_shown();
    let (me, root) = (Rc::downgrade(&app), app.borrow().root.clone());
    app.borrow().compositor.on_exit(move || {
        if let Some(app) = me.upgrade() {
            stop(&app);
        }
        root.app.settings.writer.flush();
    });
    ui.set_display_position_ms(ui.get_position_ms());
    h.with(|a| {
        ui.set_queue(ModelRc::from(a.queue_rows.clone()));
        ui.set_lyric_pieces_side(ModelRc::from(a.pieces[0].clone()));
        ui.set_lyric_pieces_full(ModelRc::from(a.pieces[1].clone()));
        ui.set_lyric_backing_pieces_side(ModelRc::from(a.backing_pieces[0].clone()));
        ui.set_lyric_backing_pieces_full(ModelRc::from(a.backing_pieces[1].clone()));
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
        a.compositor.on_history(move |forward| me.with(|a| if forward { a.forward() } else { a.back() }));
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
        // Now Playing's lyrics: the line they are scrolled to sharp, the rest blurred by the compositor.
        let weak = ui.as_weak();
        a.compositor.set_focus_source(move || lyrics_focus(&weak.upgrade()?));
        // Plugins reach the web before any profile opens; a session's own transport takes over.
        a.kinds().set_transport(a.http.clone());
        let tx = a.tx.clone();
        a.root.plugins().watched.hear(move |n| tx.send(Msg::Note { text: n.words, error: n.problem }));
        let tx = a.tx.clone();
        a.root.plugins().watched.on_changed(move || tx.send(Msg::PluginsChanged));
        let tx = a.tx.clone();
        a.root.plugins().watched.hear_casts(move |_| tx.send(Msg::Cast));
        let tx = a.tx.clone();
        a.root.plugins().watched.sign_in_with(move |plugin, url| tx.send(Msg::PluginSignIn { plugin, url }));
        if let Some((safe, names)) = a.root.safe_start() {
            a.say(&words::plugins_safe(&safe, &names), true);
        }
        a.plugins_read();
        let wake = a.me.clone();
        a.compositor.on_wake(move || wake.with(|a| a.repaint()));
        let prefs = a.root.app.settings.current().unwrap_or_default();
        match prefs.active_server().cloned() {
            Some(p) => a.open(p, None),
            None => {
                a.choose_provider();
                a.update_plugins();
            }
        }
    });
    app
}

/// Where Now Playing's synced lyrics are sharp: the band of the line they are scrolled to, the next one before the
/// first is sung and the last once all are, and an open dialog's pane. None while they are not shown or scrolled
/// by hand.
fn lyrics_focus(ui: &AppWindow) -> Option<Focus> {
    if !ui.get_full_player() || ui.get_full_panel() != Panel::Lyrics || ui.get_lyrics_lines().row_count() == 0 || !ui.get_lyrics_synced() || ui.get_full_lyrics_browsed() {
        return None;
    }
    let pane = ui.global::<crate::DialogPane>().get_at();
    let raised = ui.get_dialog_open().then_some(Raised { rect: [pane.x, pane.y, pane.width, pane.height], radius: pane.radius });
    let size = ui.window().size().to_logical(ui.window().scale_factor());
    let x = size.width / 2.0;
    // Clear of the volume pill above and the lyrics/queue pill below.
    // The band where the line sung is now, moving with it as the lyrics scroll; a step a line of 44 px
    // text and its 36 px gap.
    Some(Focus { region: [x, 48.0, size.width - x - 100.0, size.height - 48.0 - 56.0], band_top: ui.get_full_lyric_top() - 8.0, band_h: ui.get_full_lyric_h() + 16.0, pitch: 90.0, raised: raised.into_iter().collect() })
}

/// Menu bar items do what the window's buttons and keys do.
fn menu_actions(ui: &AppWindow, compositor: &Compositor) {
    let on = |id: &str, f: fn(&AppWindow)| compositor.menu().action(id, to_main(ui, f));
    on("toggle", |m| m.invoke_toggle());
    on("next", |m| m.invoke_next());
    on("previous", |m| m.invoke_previous());
    on("shuffle", |m| m.invoke_toggle_shuffle());
    on("repeat", |m| m.invoke_cycle_repeat());
    on("home", |m| m.invoke_go(View::Home));
    on("albums", |m| m.invoke_go(View::Albums));
    on("artists", |m| m.invoke_go(View::Artists));
    on("songs", |m| m.invoke_go(View::Songs));
    on("search", |m| m.invoke_go(View::Search));
    on("settings", |m| m.invoke_go(View::Settings));
    on("join-jam", |m| m.set_join_open(true));
    on("queue", |m| m.set_inspector(if m.get_inspector() == Panel::Queue { Panel::None } else { Panel::Queue }));
    on("lyrics", |m| m.set_inspector(if m.get_inspector() == Panel::Lyrics { Panel::None } else { Panel::Lyrics }));
    on("full", |m| {
        if m.get_has_song() {
            m.set_full_player(!m.get_full_player());
        }
    });
}

/// The app is quitting: saves the volume and queue, stops the engine.
fn stop(app: &RefCell<App>) {
    let settings = app.borrow().root.app.settings.clone();
    if let Some(s) = app.borrow_mut().session.take() {
        settings.keep_app_value(session::own::VOLUME, s.volume().to_string());
        s.close();
    }
}

/// Opens `url` in the system's browser.
fn open_in_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(opener).arg(url).spawn();
}

/// A kind of profile as its page shows it: `about`, the plugin's own words, in place of the note.
fn login_form(kinds: &nori_core::providers::Kinds, info: &nori_core::providers::ProviderInfo, about: Option<&str>, step: nori_core::providers::FirstStep) -> LoginKind {
    use nori_core::providers::{FirstStep, FormField};
    let w = words::provider_of(kinds, &info.kind);
    let asks = |what| step == FirstStep::Form && info.fields.iter().any(|f| f.what == what && !f.advanced);
    let account = step == FirstStep::Account;
    LoginKind {
        sign_in: if account { words::provider_sign_in(&w.name).into() } else { SharedString::default() },
        guest: if account { words::CONTINUE_AS_GUEST.into() } else { SharedString::default() },
        name: w.name.into(),
        note: about.unwrap_or(w.note).into(),
        add: w.add.into(),
        asks_name: asks(FormField::Name),
        asks_address: asks(FormField::Address),
        asks_user: asks(FormField::User),
        asks_password: asks(FormField::Password),
    }
}

/// The progress card for `p`: its words, the song's cover, and its bar (the bytes, then the work after them).
fn work_card(p: &nori_core::transfers::Progress) -> WorkCard {
    use nori_core::transfers::Progress;
    let [title, line, header] = words::progress(p);
    let (cover, kind, stages, stage, permille) = match p {
        Progress::Fetching(n) => (n.cover.clone(), 0, 1, 0, n.permille),
        Progress::Processing { work, after_fetching } => (String::new(), if work.lyrics > 0 { 1 } else { 2 }, 1 + *after_fetching as i32, *after_fetching as i32, work.permille),
    };
    WorkCard { title: title.into(), line: line.into(), header: header.into(), cover: cover.into(), kind, stages, stage, permille }
}

fn wire(ui: &AppWindow, h: &AppHandle) {
    on!(ui.on_messages_arrived, h, |a| a.drain_inbox());
    on!(ui.on_go, h, |a, v| {
        a.trail.left();
        a.go(v);
    });
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
    on!(ui.on_play_artist, h, |a, id| a.play_fetch(Fetch::Artist(id.into())));
    on!(ui.on_play_playlist, h, |a, id| a.play_fetch(Fetch::Playlist(id.into())));
    on!(ui.on_play_mix, h, |a, id| a.play_fetch(Fetch::Mix(id.into())));
    on!(ui.on_queue_card, h, |a, kind, id, next| a.queue_card(kind, id.into(), next));
    let plugin_menus = ui.global::<crate::PluginMenus>();
    on!(plugin_menus.on_run, h, |a, kind, i, id, title| a.plugin_card_menu(kind, i as usize, id.into(), title.into()));
    let icons = ui.global::<crate::ProfileIcons>();
    let picker = h.clone();
    icons.on_picker(move |id| {
        let mut shown = crate::IconPicker::default();
        picker.with(|a| shown = a.icon_picker(&id));
        shown
    });
    on!(icons.on_picked, h, |a, id, at| a.profile_icon_picked(&id, at.max(0) as usize));
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
    on!(ui.on_add_to_playlist, h, |a, i| a.add_to_playlist(Ok(i as usize)));
    on!(ui.on_add_to_new_playlist, h, |a, name| a.add_to_playlist(Err(name.into())));
    on!(ui.on_rename_playlist, h, |a, name| a.rename_playlist(name.as_str()));
    on!(ui.on_delete_playlist, h, |a| a.delete_playlist());
    on!(ui.on_play_page, h, |a, shuffle| a.play_page(shuffle));
    on!(ui.on_page_star, h, |a| a.star_page());
    on!(ui.on_page_act, h, |a, act| a.page_act(act));
    on!(ui.on_page_save, h, |a| a.save_mix());
    on!(ui.on_take_offer, h, |a| a.close_offer(true));
    on!(ui.on_dismiss_offer, h, |a| a.close_offer(false));
    on!(ui.on_more, h, |a| a.more_songs());
    on!(ui.on_search_edited, h, |a, t| a.search_edited(&t));
    on!(ui.on_search_step, h, |a, by| a.search_step(by));
    on!(ui.on_search_accepted, h, |a| a.search_accepted());
    on!(ui.on_search_preview_pick, h, |a, i| a.search_pick(i as usize));
    on!(ui.on_search_preview_changed, h, |a| a.mirror());
    on!(ui.on_search_again, h, |a, t| a.search_again(&t));
    on!(ui.on_forget_searches, h, |a| a.forget_searches());
    on!(ui.on_toggle, h, |a| a.on_session(|s| s.toggle()));
    on!(ui.on_noise_next, h, |a| a.noise_next());
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
        a.motion_follow();
        // Another view or the full player: no menu of the last one stays open over it.
        a.compositor.close_menus();
        a.repaint();
        position_shown(&a.ui(), a.player.as_ref(), a.position_now() as i32);
        a.lyrics_seen();
        a.lyrics_step(true);
        a.mirror();
        a.place_player();
        a.devices_watched();
        let full = a.ui().get_full_player();
        if full && !a.full_seen {
            a.ask_player_status();
        }
        a.full_seen = full;
    });
    on!(ui.on_pick_device, h, |a, id| a.pick_device(id.to_string()));
    let go = ui.global::<crate::Go>();
    on!(go.on_artist, h, |a, id| a.go_to(PageAsk::Artist(id.into())));
    on!(go.on_album, h, |a, id| a.go_to(PageAsk::Album(id.into())));
    let main = ui.as_weak();
    go.on_artist_menu(move |id, name, x, y| {
        if let Some(m) = main.upgrade() {
            m.invoke_artist_menu_at(id, name, x, y);
        }
    });
    on!(ui.on_jam_play, h, |a, kind, id| {
        let Some(what) = card_fetch(kind, id.into()) else { return };
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
    on!(ui.on_copy_pairing, h, |a| a.copy(a.ui().get_pairing_link().into(), words::LINK_COPIED));
    on!(jam.on_leave, h, |a| a.leave_jam());
    on!(ui.on_forget_device, h, |a, id| {
        if let Some(remote) = a.session.as_ref().and_then(|s| s.remote()) {
            remote.forget(id.to_string());
        }
        a.devices_shown();
        a.jam_shown();
    });
    on!(ui.on_pair, h, |a, link| a.pair(link.trim().to_string()));
    on!(ui.on_join_jam, h, |a, link| a.join_jam(link.trim().to_string()));
    on!(ui.on_sign_in, h, |a, answer| a.sign_in(answer.to_string()));
    on!(ui.on_sign_in_window, h, |a| a.sign_in_window());
    on!(ui.on_sign_out, h, |a| a.sign_out());
    on!(ui.on_choose_identity, h, |a, i| a.choose_identity(i));
    on!(ui.on_rename_profile, h, |a, id, name| a.rename_profile(&id, &name));
    on!(ui.on_sign_in_quiet, h, |a| a.sign_in_quiet());
    on!(ui.on_signin_closed, h, |a| {
        a.sign_in_on(SignInEvent::Close);
    });
    on!(ui.on_open_sign_in_page, h, |a| a.open_sign_in_page());
    on!(ui.on_sign_in_code, h, |a| a.sign_in_code());
    on!(ui.on_open_code_page, h, |a| a.open_code_page());
    on!(ui.on_copy_code, h, |a| a.copy_code());
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
    on!(ui.on_login, h, |a| a.login(AfterAdding::Remind));
    on!(ui.on_cancel_login, h, |a| {
        a.editing = None;
        a.go(View::Home);
    });
    on!(ui.on_find_edited, h, |a, t| {
        a.find = t.to_string();
        a.narrowed();
    });
    on!(ui.on_choose_artist, h, |a, id| a.choose_artist(id.to_string()));
    on!(ui.on_album_sort_chosen, h, |a, at| a.sort_albums(at as usize));
    on!(ui.on_albums_favorites_toggled, h, |a| a.toggle_album_favorites());
    on!(ui.on_open_accounts, h, |a| {
        let accounts = crate::settings::tabs(&a.facts.rules.settings).iter().position(|p| *p == nori_core::settings_pages::SettingsPage::Servers);
        a.ui().set_settings_tab(accounts.unwrap_or(0) as i32);
        a.go(View::Settings);
    });
    on!(ui.on_clear_queue, h, |a| a.clear_queue());
    on!(ui.on_remove_queued, h, |a, i| a.remove_queued(i.max(0) as usize));
    on!(ui.on_move_queued, h, |a, from, to| {
        a.on_session(|s| s.move_song(from.max(0) as usize, to.max(0) as usize));
        a.follow();
    });
    on!(ui.on_undo_removed, h, |a| {
        match a.taken.take() {
            Some(Taken::Song(id)) => a.on_session(|s| s.put_back(&id)),
            Some(Taken::Cleared) => a.on_session(|s| s.put_back_cleared()),
            None => {}
        }
        a.undo_timer.stop();
        a.ui().set_queue_undo("".into());
        a.follow();
    });
    on!(ui.on_close_plugins, h, |a| a.close_plugins());
    on!(ui.on_settings_tab_chosen, h, |a, t| {
        a.ui().set_settings_tab(t);
        a.plugins_read();
        if t == crate::settings::tabs(&a.facts.rules.settings).len() as i32 {
            a.plugins_look();
        }
        a.follow_sound_page(View::Settings);
        a.settings_shown();
    });
    on!(ui.on_setting_toggled, h, |a, name, on| match crate::settings::PluginAct::of(&name) {
        Some(act) => a.plugin_set(act, nori_plugins::Value::Switch(on)),
        None => {
            let (name, value) = crate::settings::toggled(&name, on);
            a.setting(name, &value);
        }
    });
    on!(ui.on_setting_chosen, h, |a, name, i| match crate::settings::PluginAct::of(&name) {
        Some(act) => a.plugin_set(act, nori_plugins::Value::Choice(i.max(0) as u32)),
        None => a.setting_chosen(&name, i.max(0) as usize),
    });
    on!(ui.on_setting_action, h, |a, name| match crate::settings::PluginAct::of(&name) {
        Some(act) => a.plugin_act(act),
        None => a.setting_action(&name),
    });
    // A plugin's level is kept once let go.
    on!(ui.on_setting_slid, h, |a, name, v, last| match crate::settings::PluginAct::of(&name) {
        Some(act) if last => a.plugin_set(act, nori_plugins::Value::Level(v as f64)),
        Some(_) => {}
        None => a.slid(&name, v, last),
    });
    on!(ui.on_setting_typed, h, |a, name, text| match (crate::settings::PluginAct::of(&name), crate::settings::Act::of(&name)) {
        _ if name == words::REMOVE_DOWNLOADS => a.remove_asked(),
        (Some(act), _) => a.plugin_typed(act, text.to_string()),
        (None, Some(act)) => a.backup_typed(act, text.to_string()),
        (None, None) => a.setting(&name, &text),
    });
    on!(ui.on_row_placed, h, |a, kind, id, by| a.row_placed(kind, &id, by));
    on!(ui.on_eq_set, h, |a, name, value| {
        a.setting(&name, &value);
    });
    on!(ui.on_eq_typed, h, |a, i, field, text| {
        use nori_core::settings::BandField;
        let field = [BandField::Frequency, BandField::Gain, BandField::Q][field.clamp(0, 2) as usize];
        a.eq_typed(i.max(0) as u32, field, &text)
    });
    on!(ui.on_eq_band, h, |a, i, pos, gain, q, kind, channel, last| a.eq_band(i.max(0) as u32, nori_core::settings_store::BandPoint { pos, gain_db: gain, q, kind, channel }, last));
    on!(ui.on_eq_add, h, |a, pos, gain| a.eq_add(pos, gain));
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
        if let Some(p) = a.root.app.settings.current() {
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
                self.root.app.settings.keep_app_value(session::own::DEVICE, v.clone());
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

    /// Opens `profile`; `added` says what follows when it was just added.
    fn open(&mut self, profile: SavedServer, added: Option<AfterAdding>) {
        if let Some(s) = self.session.take() {
            s.close_for(Some(&profile));
        }
        let ui = self.ui();
        self.profiles_shown();
        self.waiting = self.kinds().waits(profile.clone()).map(|_| profile.clone());
        if self.waiting.is_some() {
            // The first screen, where its plugin's row installs it or turns it on; one built for an older
            // nori is updated now, and the profile opens once it is (`open_waiting`).
            self.update_plugins();
            return self.choose_provider();
        }
        match Session::open(&self.root, self.http.clone(), profile, self.tx.clone(), self.mpris.clone()) {
            Ok(s) => {
                s.check();
                if self.hidden {
                    s.in_sight(false);
                }
                ui.set_volume(s.volume());
                ui.set_downloads(s.rules().downloads);
                // Albums and artists carry the account's seals, as on Android.
                ui.set_page_can_star(s.rules().account && s.rules().stars);
                ui.set_favorites_offered(s.rules().stars);
                ui.set_library_tabs(library_tabs(&s.rules()));
                self.session = Some(s);
                self.update_plugins();
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
                self.go(View::Home);
                match self.session.as_ref().and_then(|s| s.client.sign_in_due(added)) {
                    Some(offer @ SignInEvent::Offer { .. }) => self.offer_sign_in(offer),
                    Some(SignInEvent::Code) => {
                        self.offer_sign_in(SignInEvent::Offer { ended: false });
                        self.sign_in_code();
                    }
                    Some(SignInEvent::Close) => {}
                    Some(_) => {
                        self.offer_sign_in(SignInEvent::Offer { ended: false });
                        self.sign_in_window();
                    }
                    None => {}
                }
                if !self.account() {
                    // A jam guest's: the host's library, and the jam in the queue panel.
                    ui.set_inspector(Panel::Queue);
                }
            }
            Err(said) => {
                self.choose_provider();
                self.add_on(AddEvent::Unopened { said });
            }
        }
    }

    fn login(&mut self, after: AfterAdding) {
        let ui = self.ui();
        let Some(info) = self.add.kind().and_then(|k| self.kinds().providers().into_iter().find(|i| i.kind == *k)) else { return };
        let form = [ui.get_login_name(), ui.get_login_url(), ui.get_login_user(), ui.get_login_password()].map(|t| t.to_string());
        let draft = login_draft(&info, self.editing.clone(), form);
        let ready = self.kinds().server_ready(draft.clone());
        let prefs = self.root.app.settings.current().unwrap_or_default();
        let list = nori_core::settings::ServerList { servers: prefs.servers, active_server_id: prefs.active_server_id };
        let added = self.kinds().adds_profile(list, draft.clone()).then_some(after);
        if let AddDo::Send { .. } = self.add_on(AddEvent::Send { ready, added }) {
            let (root, http, tx) = (self.root.clone(), self.http.clone(), self.tx.clone());
            std::thread::spawn(move || tx.send(Msg::LoggedIn(session::check_login(&root, http, draft))));
        }
    }

    /// Takes `event` on the pages of adding a profile and shows where that leaves them.
    fn add_on(&mut self, event: AddEvent) -> AddDo {
        let then = self.add.on(event);
        let ui = self.ui();
        ui.set_login_choosing(self.add.choosing());
        ui.set_login_busy(self.add.busy());
        ui.set_login_error(self.add.failure().map_or("", words::add_failure).into());
        self.plugged.choosing = self.add.choosing() && ui.get_view() == View::Login;
        then
    }

    /// Takes `event` on the account sheet and shows where that leaves it; false for an answer not to send.
    fn sign_in_on(&mut self, event: SignInEvent) -> bool {
        let send = self.sign_in.on(event);
        if !self.sign_in.checks_code() {
            self.code = None;
        }
        let ui = self.ui();
        ui.set_signin_open(self.sign_in.shown != SignInShown::Closed);
        ui.set_signin_busy(self.sign_in.busy());
        ui.set_signin_ended(self.sign_in.shown == SignInShown::Offered { ended: true });
        let (step, code, expired) = match &self.sign_in.shown {
            SignInShown::Code { code, expired } => (true, code.as_ref().map_or("", |c| c.code.as_str()), *expired),
            _ => (false, "", false),
        };
        ui.set_signin_code_step(step);
        ui.set_signin_code(code.into());
        ui.set_signin_code_expired(expired);
        let error = match &self.sign_in.attempt {
            nori_core::providers::flow::SignInAttempt::Failed { result } if step => words::code_failed(result),
            nori_core::providers::flow::SignInAttempt::Failed { result } => words::sign_in_failed(result),
            _ => String::new(),
        };
        ui.set_signin_error(error.into());
        send
    }

    /// The account sheet's code step: a new code asked for, shown and checked while the step shows.
    fn sign_in_code(&mut self) {
        let Some(s) = &self.session else { return };
        let (client, h) = (s.client.clone(), (***s).clone());
        if !self.sign_in_on(SignInEvent::Code) {
            return;
        }
        self.codes += 1;
        let heard = CodeHeard { session: h, tx: self.tx.clone(), code: self.codes };
        self.code = Some((self.codes, client.sign_in_with_code(Arc::new(heard))));
    }

    /// The code's page, in the browser.
    fn open_code_page(&self) {
        if let SignInShown::Code { code: Some(c), .. } = &self.sign_in.shown {
            open_in_browser(&c.url);
        }
    }

    fn copy_code(&self) {
        if let SignInShown::Code { code: Some(c), .. } = &self.sign_in.shown {
            self.copy(c.code.clone(), words::CODE_COPIED);
        }
    }

    fn go(&mut self, view: View) {
        self.go_at(view, 0.0);
    }

    /// Shows tab `view` scrolled to `at`.
    fn go_at(&mut self, view: View, at: f32) {
        self.start_at(at);
        // A new page starts unfiltered.
        if !self.find.is_empty() {
            self.find.clear();
            self.ui().set_find_text("".into());
            self.narrowed();
        }
        self.follow_sound_page(view);
        self.plugged.stack.clear();
        self.plugged.choosing &= view == View::Login;
        let ui = self.ui();
        ui.set_failed("".into());
        ui.set_loading(false);
        self.want.clear();
        if view == View::Login {
            ui.set_can_cancel_login(self.session.is_some());
            ui.set_view(View::Login);
            return;
        }
        ui.set_view(view);
        // The search page's field is the top pill's.
        if let Some(top) = &self.top {
            if view == View::Search { top.invoke_focus_search() } else { top.invoke_end_search() }
        }
        if view == View::Search && self.top.as_ref().is_none_or(|t| t.get_search_text().trim().is_empty()) {
            self.search_edited("");
        }
        let req = match view {
            View::Home => {
                self.look_for_offer();
                self.home_req()
            }
            View::Albums => {
                self.album_sort = nori_core::browse::album_sort_saved(self.root.app.settings.current().unwrap_or_default().list_prefs);
                self.show_album_sorts();
                self.album_pages.reset();
                let (sort, favorites) = (self.album_sort, self.albums_favorites);
                self.album_pages.ask().map(|offset| Req::Page(PageAsk::Albums { sort, offset, favorites }))
            }
            View::Artists => Some(Req::Page(PageAsk::Artists)),
            View::Playlists => Some(Req::Page(PageAsk::Playlists)),
            View::Favorites => {
                self.songs.clear();
                // The favorites come whole.
                self.song_pages.end();
                Some(Req::Page(PageAsk::Favorites))
            }
            View::Songs => {
                self.songs.clear();
                self.song_pages.reset();
                self.song_pages.ask().map(|offset| Req::Page(PageAsk::Songs { offset }))
            }
            View::Settings => {
                self.settings_shown();
                self.on_session(|s| s.facts());
                None
            }
            View::Equalizer => {
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
        if !self.want.contains(&req) {
            self.want.push(req.clone());
        }
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
        let at = self.ui().global::<crate::Scroll>().get_at();
        self.trail.opened(ask.clone(), opened_from(self.ui().get_view()), at);
        self.show_page(ask, 0.0);
    }

    /// The next page or tab shown starts scrolled to `at` ([`crate::Scroll`]).
    fn start_at(&self, at: f32) {
        let ui = self.ui();
        let scroll = ui.global::<crate::Scroll>();
        scroll.set_want(at);
        scroll.set_at(at);
        scroll.set_asked(scroll.get_asked().wrapping_add(1));
    }

    /// Shows page `ask` scrolled to `at`, the way back already kept ([`PageTrail`]).
    fn show_page(&mut self, ask: PageAsk, at: f32) {
        self.start_at(at);
        if !self.find.is_empty() {
            self.find.clear();
            self.ui().set_find_text("".into());
            self.narrowed();
        }
        let ui = self.ui();
        self.page_songs.clear();
        self.page_songs_all = false;
        let (fetch, page) = match &ask {
            PageAsk::Album(id) => (Fetch::Album(id.clone()), PageShown::Album),
            PageAsk::Artist(id) => (Fetch::Artist(id.clone()), PageShown::Artist),
            PageAsk::Playlist(id) => (Fetch::Playlist(id.clone()), PageShown::Playlist(None)),
            PageAsk::Mix(id) => (Fetch::Mix(id.clone()), PageShown::Mix(None)),
            _ => return,
        };
        (self.page_fetch, self.page) = (Some(fetch), Some(page));
        self.want.clear();
        ui.set_page_id(match &ask {
            PageAsk::Album(id) | PageAsk::Artist(id) | PageAsk::Playlist(id) | PageAsk::Mix(id) => id.as_str().into(),
            _ => "".into(),
        });
        ui.set_page_kind(match &ask {
            PageAsk::Album(_) => PageLayout::Album,
            PageAsk::Artist(_) => PageLayout::Artist,
            _ => PageLayout::List,
        });
        ui.set_page_mix(matches!(ask, PageAsk::Mix(_)));
        ui.set_page_pinnable(matches!(ask, PageAsk::Playlist(_)));
        ui.set_page_title("".into());
        ui.set_page_sub("".into());
        ui.set_page_sub_artist("".into());
        ui.set_page_caption("".into());
        ui.set_page_art("".into());
        ui.set_page_covers(ModelRc::default());
        ui.set_page_acts(ModelRc::default());
        ui.set_page_can_save(false);
        ui.set_page_editable(false);
        ui.set_page_songs(ModelRc::default());
        ui.set_page_shelves(ModelRc::default());
        self.page_colours(None);
        self.follow_sound_page(View::Page);
        ui.set_view(View::Page);
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
        let tinted = self.root.app.settings.prefs(|p| p.theme.covers_tint_pages());
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
        let prefs = self.root.app.settings.current().unwrap_or_default();
        crate::look::dye(&prefs, self.compositor.system_dark(), crate::look::Today::now(), c).map(|d| wash(c, Some(&d)))
    }

    /// The look of the page the app is on: the season's, or a cover's.
    fn page_look(&self) -> crate::look::Look {
        let ui = self.ui();
        let prefs = self.root.app.settings.current().unwrap_or_default();
        let (dark, today) = (self.compositor.system_dark(), crate::look::Today::now());
        let cover = if ui.get_full_player() {
            self.now_cover.as_ref()
        } else if ui.get_view() == View::Page {
            self.page_cover.as_ref()
        } else {
            None
        };
        match cover {
            Some(c) => crate::look::cover(&self.root.app, &prefs, dark, today, c),
            None => crate::look::plain(&self.root.app, &prefs, dark, today),
        }
    }

    /// Dresses every window in the page's look, cross-fading from the one worn unless motion is reduced.
    fn repaint(&mut self) {
        let want = self.page_look();
        let prefs = self.root.app.settings.current().unwrap_or_default();
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
        look.paint(&self.ui());
        if let Some(p) = &self.player {
            look.paint(p);
        }
        if let Some(t) = &self.top {
            look.paint(t);
        }
        self.compositor.set_page_colour(look.page());
    }

    fn data(&mut self, req: Req, r: Result<Data, String>) {
        let ui = self.ui();
        let shown = self.want.contains(&req);
        if shown {
            ui.set_loading(false);
        }
        let d = match r {
            Ok(d) => d,
            Err(e) => {
                if self.page_failed(&req) && shown {
                    ui.set_failed(e.into());
                }
                return;
            }
        };
        match d {
            Data::Home(HomePart::Mixes(tiles)) => {
                // A like reads the tiles again; unchanged, nothing on Home is drawn anew.
                if tiles == self.mixes {
                    return;
                }
                keep_shown_mosaics(&mut self.art.borrow_mut(), &tiles);
                self.mixes = tiles;
                self.show_picks();
            }
            Data::Home(HomePart::Feed(feed)) => {
                // Each shelf as the source lists it: its songs first (a song card plays the shelf's songs
                // from it), then its albums, playlists and artists.
                let shelves = feed.iter().filter(|f| !(f.songs.is_empty() && f.albums.is_empty() && f.playlists.is_empty() && f.artists.is_empty())).enumerate().map(|(at, f)| {
                    let all = f.songs.iter().map(song_card).chain(f.albums.iter().map(album_card)).chain(f.playlists.iter().map(playlist_card)).chain(f.artists.iter().map(artist_card));
                    let old = self.shelves.row_data(at).filter(|o| o.title == f.title.as_str());
                    Shelf { title: f.title.as_str().into(), cards: cards(moved(old.as_ref(), all.collect()).into_iter()), loaded: true }
                });
                put_shelves(&self.shelves, shelves.collect());
                self.feed_songs = feed.into_iter().filter(|f| !(f.songs.is_empty() && f.albums.is_empty() && f.playlists.is_empty() && f.artists.is_empty())).map(|f| f.songs).collect();
            }
            Data::Home(HomePart::Shelf { at, items }) => {
                let ShelfItems::Albums(albums) = items else { return };
                if let Some(mut shelf) = self.shelves.row_data(at) {
                    shelf.cards = cards(moved(Some(&shelf), albums.iter().map(album_card).collect()).into_iter());
                    shelf.loaded = true;
                    self.shelves.set_row_data(at, shelf);
                }
            }
            Data::Page(PageGot::Albums(v)) => {
                // Pages of the list, each asked for once the one before came full.
                let Req::Page(PageAsk::Albums { sort, offset, favorites }) = req else { return };
                if (sort, favorites) != (self.album_sort, self.albums_favorites) {
                    return;
                }
                if let Some(next) = albums_paged(&mut self.lists.albums, &mut self.album_pages, sort, favorites, offset, &v) {
                    self.on_session(|s| s.load(Req::Page(PageAsk::Albums { sort, offset: next, favorites })));
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
                self.lists.playlists = v;
                self.narrowed();
            }
            Data::Page(PageGot::Favorites(v)) => {
                self.songs = v.songs;
                ui.set_more_songs(false);
                self.show_songs();
            }
            Data::Page(PageGot::Songs { songs: v, exhausted }) => {
                let Req::Page(PageAsk::Songs { offset }) = req else { return };
                if !self.song_pages.answered(&offset, (!exhausted).then_some(offset + v.len() as u32)) {
                    return;
                }
                self.songs.truncate(offset as usize);
                self.songs.extend(v);
                ui.set_more_songs(self.song_pages.more());
                self.show_songs();
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
                ui.set_page_starred(nori_core::browse::playlist_pinned(&self.root.app.settings, &d.playlist.id));
                ui.set_page_title(d.playlist.name.as_str().into());
                let notes = self.root.app.settings.prefs(|p| p.playlist_notes);
                ui.set_page_sub(nori_core::pages::playlist_description(d.playlist.comment.clone(), notes).unwrap_or_default().into());
                ui.set_page_caption(if d.songs.is_empty() { String::new() } else { words::songs_caption(d.songs.len(), d.seconds) }.into());
                self.set_page_art(d.playlist.cover_art.clone());
                let editable = Some(d.playlist).filter(|p| self.editable(p));
                ui.set_page_editable(editable.is_some());
                self.page = Some(PageShown::Playlist(editable));
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
                self.page = Some(PageShown::Mix(Some((m.id.clone(), words::mix_name(m.name).to_string()))));
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
            2 => return self.ask_remove(self.page_songs.iter().map(|s| s.id.clone()).collect()),
            4 if matches!(fetch, Fetch::Artist(_)) => self.on_session(|s| s.enqueue_later(fetch, false)),
            4 => self.enqueue_page(),
            5 => self.redraw_mix(),
            _ => {}
        }
        if act != 5 {
            self.page_head();
        }
        self.mark_playing();
    }

    /// The list head's glyphs again, after the page's downloads changed.
    fn page_head(&mut self) {
        self.page_actions(match &self.page {
            Some(PageShown::Album) => PageKind::Album,
            Some(PageShown::Artist) => PageKind::Artist,
            Some(PageShown::Mix(Some(_))) => PageKind::Mix { refreshable: true },
            Some(PageShown::Mix(None) | PageShown::Playlist(_)) | None => PageKind::Playlist,
        });
    }

    /// Home's "For you": each mix's card on the season's paper dyed with the mix (the core's
    /// `mix_card_colours`), or once its first cover's colors are known with those, as the mix's page wears them.
    fn show_picks(&self) {
        use nori_core::mixes::board::{mix_card_colours, mix_tile_colours};
        let prefs = self.root.app.settings.current().unwrap_or_default();
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
                        let look = crate::look::cover(&self.root.app, &prefs, dark, today, c);
                        (look.page(), look.ink())
                    }
                    None => (card.page, card.ink),
                };
                Pick {
                    title: words::mix_name(t.name).into(),
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
        let Some(s) = &self.session else { return };
        let (h, tx) = (nori_host::session::Handle::clone(s), self.tx.clone());
        nori_host::spawn("nori-queue-offer", move || {
            tx.send(Msg::Offer(h.queue_offer()));
        });
    }

    /// Home's Continue card: the offered song, where it stands in its queue, and its cover.
    fn offer_shown(&self) {
        let ui = self.ui();
        let song = self.offer.shown().and_then(|o| o.songs.get(o.index as usize).map(|s| (o, s)));
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
        let (Some(o), Some(s)) = (self.offer.close(), &self.session) else { return };
        s.queue_offer_closed(o, take);
        self.offer_shown();
    }

    /// The mix shown drawn again, then shown anew.
    fn redraw_mix(&self) {
        let (Some(PageShown::Mix(Some((id, _)))), Some(s)) = (&self.page, &self.session) else { return };
        let id = id.clone();
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
        let (Some(PageShown::Mix(Some((_, title)))), Some(s)) = (&self.page, &self.session) else { return };
        let title = title.clone();
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
                Ok(_) => Msg::Playlist { text: words::playlist_created(&name), change: PlaylistChange::Made },
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
        let all = self.page_songs_all || ui.get_page_kind() != PageLayout::Artist;
        let n = nori_core::browse::songs_shown(self.page_songs.len() as u32, all) as usize;
        show_in(ui.get_page_songs(), self.rows(&self.page_songs[..n], ui.get_page_kind() != PageLayout::Artist), |m| ui.set_page_songs(m));
        ui.set_page_songs_more(n < self.page_songs.len());
    }

    /// Search's songs as listed: the best few until Show all.
    fn show_search_songs(&self) {
        let ui = self.ui();
        let n = nori_core::browse::songs_shown(self.search_songs.len() as u32, self.search_songs_all) as usize;
        show_in(ui.get_search_songs(), self.rows(&self.search_songs[..n], false), |m| ui.set_search_songs(m));
        ui.set_search_songs_more(n < self.search_songs.len());
    }

    /// The Songs tab's or Favorites' songs.
    fn show_songs(&self) {
        let ui = self.ui();
        show_in(ui.get_songs(), self.rows(&self.songs, false), |m| ui.set_songs(m));
    }

    /// `page`: the songs are a page's, whose head says when every one is downloaded.
    fn rows(&self, songs: &[Song], page: bool) -> Vec<SongRow> {
        use nori_core::menus::DownloadGlyph;
        let heard = self.heard.as_deref();
        let find = nori_core::pages::Find::new(&self.find);
        let ids: Vec<&str> = songs.iter().map(|s| s.id.as_str()).collect();
        let marks = self.session.as_ref().map(|s| s.download_glyphs(&ids, page)).unwrap_or_default();
        let stars = self.session.as_ref().is_some_and(|s| s.rules().stars);
        let row = |(i, s): (usize, &Song)| SongRow {
            by: if self.asked.contains(&s.id) { words::ASKED.into() } else { SharedString::default() },
            mark: match marks.get(i) {
                Some(DownloadGlyph::Done) => 2,
                Some(DownloadGlyph::Ring | DownloadGlyph::Failed) => 1,
                _ => 0,
            },
            sealed: stars && self.starred(s),
            ..row(s, i, heard == Some(s.id.as_str()))
        };
        songs.iter().enumerate().filter(|(_, s)| find.keeps_song(s)).map(row).collect()
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
        ui.set_playlists(pick(&self.lists.playlists.iter().map(playlist_card).collect::<Vec<_>>()));
        self.show_songs();
        self.show_page_songs();
    }

    /// The Albums page's sort menu, the order in use marked, and its Favorites chip.
    fn show_album_sorts(&self) {
        let ui = self.ui();
        ui.set_albums_favorites(self.albums_favorites);
        let sorts = nori_core::browse::album_sorts();
        ui.set_album_sorts(ModelRc::new(VecModel::from(sorts.iter().map(|&s| SharedString::from(words::album_sort(s))).collect::<Vec<_>>())));
        let way = |s| match nori_core::browse::album_sort_direction(s) {
            nori_core::browse::SortDirection::Ascending => 1,
            nori_core::browse::SortDirection::Descending => -1,
            nori_core::browse::SortDirection::Shuffled => 0,
        };
        ui.set_album_sort_ways(ModelRc::new(VecModel::from(sorts.iter().map(|&s| way(s)).collect::<Vec<i32>>())));
        ui.set_album_sort(sorts.iter().position(|&s| s == self.album_sort).unwrap_or(0) as i32);
        ui.set_albums_in_rows(self.album_sort == AlbumSort::ArtistRows);
    }

    /// Reads the albums again in the order chosen in the sort menu, and keeps it.
    fn sort_albums(&mut self, at: usize) {
        let Some(&sort) = nori_core::browse::album_sorts().get(at) else { return };
        if sort == self.album_sort {
            return;
        }
        let settings = &self.root.app.settings;
        let mut prefs = settings.current().unwrap_or_default();
        let kept = nori_core::browse::album_sort_kept(sort);
        prefs.list_prefs.insert(kept.key, kept.value);
        settings.put(prefs);
        self.album_sort = sort;
        self.reload_albums();
    }

    /// Lists only the favorite albums, or every album again, in the same order.
    fn toggle_album_favorites(&mut self) {
        self.albums_favorites = !self.albums_favorites;
        self.reload_albums();
    }

    /// Reads the album list again from the top, in its order and filter.
    fn reload_albums(&mut self) {
        self.lists.albums.clear();
        self.narrowed();
        self.show_album_sorts();
        self.album_pages.reset();
        if let Some(offset) = self.album_pages.ask() {
            self.load(Req::Page(PageAsk::Albums { sort: self.album_sort, offset, favorites: self.albums_favorites }));
        }
    }

    /// Page `req` could not be read. A paged list's later page is asked for again as the list goes on
    /// (scrolled, or opened again) and keeps what it lists; true when the failure is to be shown.
    fn page_failed(&mut self, req: &Req) -> bool {
        match req {
            Req::Page(PageAsk::Albums { offset, .. }) => {
                self.album_pages.failed(offset);
                *offset == 0
            }
            Req::Page(PageAsk::Songs { offset }) => {
                self.song_pages.failed(offset);
                *offset == 0
            }
            _ => true,
        }
    }

    /// Shows an artist beside the Artists list.
    fn choose_artist(&mut self, id: String) {
        let ui = self.ui();
        ui.set_artist_chosen(id.as_str().into());
        ui.set_page_kind(PageLayout::Artist);
        ui.set_page_id(id.as_str().into());
        ui.set_page_title("".into());
        ui.set_page_caption("".into());
        ui.set_page_shelves(ModelRc::default());
        self.page_songs.clear();
        self.page_songs_all = false;
        (self.page_fetch, self.page) = (Some(Fetch::Artist(id.clone())), Some(PageShown::Artist));
        // The artist chosen before is no longer shown.
        self.want.retain(|r| !matches!(r, Req::Page(PageAsk::Artist(_))));
        self.load(Req::Page(PageAsk::Artist(id)));
        // Keep the list visible while the artist loads.
        ui.set_loading(false);
    }

    /// Refreshes the lists' marks: the song playing, downloads and seals.
    fn mark_playing(&self) {
        self.show_page_songs();
        self.show_search_songs();
        if !self.songs.is_empty() {
            self.show_songs();
        }
    }

    fn more_songs(&mut self) {
        if let Some(offset) = self.song_pages.ask() {
            self.load(Req::Page(PageAsk::Songs { offset }));
        }
    }

    fn search_edited(&mut self, text: &str) {
        let Some(s) = &self.session else { return };
        // Away from the search page, what it finds shows in the field's preview.
        let ui = self.ui();
        if ui.get_view() != View::Search {
            ui.set_search_preview_open(!text.trim().is_empty());
        }
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
        ui.set_search_nothing(matches!(v.answer, SearchShown::NothingFound));
        ui.set_search_query(v.query.trim().into());
        if blank {
            let recent = self.session.as_ref().and_then(|s| s.core.search_history().ok()).unwrap_or_default();
            ui.set_recent_searches(ModelRc::new(VecModel::from(recent.into_iter().map(SharedString::from).collect::<Vec<_>>())));
        }
        let busy = matches!(v.answer, SearchShown::Asking { .. });
        let note = match &v.answer {
            SearchShown::Failed { fallback, .. } => format!("The server could not be searched ({}); these are from the offline index", fallback.reason.clone().unwrap_or_default()),
            _ => String::new(),
        };
        let found = v.answer.into_result().unwrap_or_default();
        self.search_preview = nori_core::search::search_preview(&found);
        ui.set_search_preview(ModelRc::new(VecModel::from(preview_cards(&self.search_preview, &found))));
        ui.set_search_preview_at(-1);
        ui.set_search_artists(cards(found.artists.iter().map(artist_card)));
        ui.set_search_albums(cards(found.albums.iter().map(album_card)));
        ui.set_search_playlists(cards(found.playlists.iter().map(playlist_card)));
        self.search_songs = found.songs;
        if self.search_query != v.query.trim() {
            self.search_query = v.query.trim().to_string();
            self.search_songs_all = false;
        }
        self.show_search_songs();
        ui.set_search_busy(busy);
        ui.set_search_note(note.into());
    }

    /// The arrows in the search field: the preview's line `by` lines on, past the groups' heads.
    fn search_step(&mut self, by: i32) {
        let ui = self.ui();
        if !ui.get_search_preview_open() {
            return;
        }
        ui.set_search_preview_at(preview_step(&self.search_preview, ui.get_search_preview_at(), by));
    }

    /// Enter in the search field: the preview's line walked to, or the search page.
    fn search_accepted(&mut self) {
        let ui = self.ui();
        let at = ui.get_search_preview_at();
        if ui.get_search_preview_open() && at >= 0 {
            self.search_pick(at as usize);
        } else {
            ui.set_search_preview_open(false);
            self.trail.left();
            self.go(View::Search);
        }
    }

    /// Line `i` of the search field's preview picked: its page, its song played, or every result.
    fn search_pick(&mut self, i: usize) {
        use nori_core::search::PreviewLine;
        let ui = self.ui();
        ui.set_search_preview_open(false);
        if let Some(top) = &self.top {
            top.invoke_end_search();
        }
        let id = |m: &ModelRc<Card>| m.row_data(i).map(|c| c.id.to_string()).unwrap_or_default();
        match self.search_preview.get(i).copied() {
            Some(PreviewLine::Artist { .. }) => self.open_page(PageAsk::Artist(id(&ui.get_search_preview()))),
            Some(PreviewLine::Album { .. }) => self.open_page(PageAsk::Album(id(&ui.get_search_preview()))),
            Some(PreviewLine::Playlist { .. }) => self.open_page(PageAsk::Playlist(id(&ui.get_search_preview()))),
            Some(PreviewLine::Song { at }) => self.song(2, at as usize, -1),
            Some(PreviewLine::All) => {
                self.trail.left();
                self.go(View::Search);
            }
            Some(PreviewLine::Head { .. }) | None => {}
        }
    }

    fn play_fetch(&self, what: Fetch) {
        self.on_session(|s| s.play_later(what, false));
    }

    /// Card `id` of `kind` (`Card::kind`, or 4 a mix) next or at the end of the queue; a song card, the
    /// song alone.
    fn queue_card(&self, kind: i32, id: String, next: bool) {
        if kind == 3 {
            if let Some(song) = self.feed_songs.iter().flatten().find(|s| s.id == id) {
                self.on_session(|s| s.enqueue(vec![song.clone()], next));
            }
            return;
        }
        if let Some(what) = card_fetch(kind, id) {
            self.on_session(|s| s.enqueue_later(what, next));
        }
    }

    fn list_song(&self, list: i32, i: usize) -> Option<Song> {
        match list {
            0 => self.page_songs.get(i),
            1 => self.songs.get(i),
            3 => self.queue.as_ref().and_then(|q| q.songs.get(i)),
            4 => self.preview_song(i).and_then(|at| self.search_songs.get(at)),
            _ => self.search_songs.get(i),
        }
        .cloned()
    }

    /// The search song at line `i` of the preview, by its place in the search's songs.
    fn preview_song(&self, i: usize) -> Option<usize> {
        match self.search_preview.get(i) {
            Some(nori_core::search::PreviewLine::Song { at }) => Some(*at as usize),
            _ => None,
        }
    }

    /// Song `i` of `list`'s menu, made as it opens. A song of a playlist the profile may change also offers
    /// to take it out, after Add to playlist.
    fn song_menu(&self, list: i32, i: usize) -> ModelRc<SongGo> {
        let Some(one) = self.list_song(list, i) else { return ModelRc::default() };
        // The page's own songs are list 0: their menu leaves out the page's album or artist.
        let ui = self.ui();
        let page = (list == 0).then(|| ui.get_page_id().to_string()).filter(|id| !id.is_empty()).and_then(|id| match ui.get_page_kind() {
            PageLayout::Album => Some(nori_core::PageOrigin::new(nori_core::OriginKind::Album, id)),
            PageLayout::Artist => Some(nori_core::PageOrigin::new(nori_core::OriginKind::Artist, id)),
            PageLayout::List => None,
        });
        let mut lines = self.menu_lines(one, false, page);
        if list == 0 && self.page_playlist().is_some() {
            let at = lines.iter().position(|l| l.kind == 7).map_or(lines.len(), |p| p + 1);
            lines.insert(at, SongGo { title: words::REMOVE_FROM_PLAYLIST.into(), kind: 18, id: SharedString::default(), place: SongPlace::Main, enabled: true, filled: false });
        }
        ModelRc::new(VecModel::from(lines))
    }

    /// The menu of the song playing, with the player's lines.
    fn player_menu(&self) -> ModelRc<SongGo> {
        self.song.clone().map_or_else(ModelRc::default, |one| ModelRc::new(VecModel::from(self.menu_lines(one, true, None))))
    }

    /// `one`'s menu as the core lists it (menus.rs's `song_menu`), its rows first in their groups' order and
    /// then its glyphs (`SongPlace`); `player`: opened from the player, `page` the album or artist page it is
    /// opened on.
    fn menu_lines(&self, one: Song, player: bool, page: Option<nori_core::PageOrigin>) -> Vec<SongGo> {
        use nori_core::menus::{song_menu, SongAction as A, SongDownload, SongMenuPlace};
        let Some(s) = self.session.as_ref() else { return Vec::new() };
        let starred = self.heart(&one);
        let download = if s.rules().downloads { s.download_state(&one.id) } else { SongDownload::None };
        let lyrics = player && self.lyrics.as_ref().is_some_and(|l| !l.is_empty());
        let excluded = s.core.mix_excluded_songs().is_ok_and(|ids| ids.contains(&one.id));
        let quality = words::quality(&one);
        let items = song_menu(one, starred.unwrap_or_default(), excluded, download, player, lyrics, starred.is_some(), s.rules().account, s.rules().downloads, s.rules().radio, s.rules().asks, page);
        let lines = items.into_iter().map(|m| {
            let (kind, id) = match &m.action {
                A::Favourite { .. } => (0, String::new()),
                A::GoToAlbum { id } => (1, id.clone()),
                A::GoToArtist { id, .. } => (2, id.clone()),
                A::Download => (3, String::new()),
                A::Radio => (4, String::new()),
                A::PlayNext => (5, String::new()),
                A::AddToQueue => (6, String::new()),
                A::AddToPlaylist => (7, String::new()),
                A::RemoveDownload => (8, String::new()),
                A::StopDownload => (9, String::new()),
                A::AddToLibrary => (10, String::new()),
                A::ExcludeFromMixes { on } => (if *on { 12 } else { 19 }, String::new()),
                A::Share => (13, String::new()),
                A::Details => (14, String::new()),
                A::Lyrics => (16, String::new()),
                A::SleepTimer => (17, String::new()),
            };
            let filled = matches!(m.action, A::Favourite { on: false });
            let place = match m.place {
                SongMenuPlace::Title => SongPlace::Title,
                SongMenuPlace::Names => SongPlace::Names,
                SongMenuPlace::Glyphs => SongPlace::Glyphs,
                SongMenuPlace::Line if m.more => SongPlace::Rest,
                SongMenuPlace::Line => SongPlace::Main,
            };
            SongGo { title: words::song_action(&m.action).into(), kind, id: id.into(), place, enabled: m.enabled, filled }
        });
        let plugins = self.plugin_menus[3].iter().enumerate().map(|(i, e)| SongGo { title: e.label.as_str().into(), kind: PLUGIN_MENU + i as i32, id: SharedString::default(), place: SongPlace::Rest, enabled: true, filled: false });
        let note = quality.map(|q| SongGo { title: q.into(), place: SongPlace::Note, ..Default::default() });
        let mut lines: Vec<SongGo> = lines.chain(plugins).chain(note).collect();
        lines.sort_by_key(|l| [SongPlace::Main, SongPlace::Names, SongPlace::Rest, SongPlace::Title, SongPlace::Note, SongPlace::Glyphs].iter().position(|p| *p == l.place));
        lines
    }

    /// Plays song `i` of `list` (`how` -1), or does its menu's line `how` (`SongGo::kind`).
    fn song(&mut self, list: i32, i: usize, how: i32) {
        if list == 4 {
            if let Some(at) = self.preview_song(i) {
                self.song(2, at, how);
            }
            return;
        }
        let Some(one) = self.list_song(list, i) else { return };
        let (songs, origin) = match list {
            0 => (self.page_songs.clone(), self.page_fetch.as_ref().map(Fetch::origin)),
            1 => (self.songs.clone(), None),
            3 => (self.queue.as_ref().map(|q| q.songs.to_vec()).unwrap_or_default(), None),
            _ => (self.search_songs.clone(), None),
        };
        match how {
            -1 if list == 3 => self.on_session(|s| s.jump(i)),
            -1 => self.on_session(|s| s.play(songs, i, false, origin)),
            18 => self.remove_from_playlist(i),
            _ => self.song_act(one, how),
        }
    }

    /// The player's menu line `how` (`SongGo::kind`), for the song playing.
    fn player_act(&mut self, how: i32) {
        match how {
            16 => self.lyrics_sheet(true),
            17 => {
                let running = self.session.as_ref().is_some_and(|s| s.sleep() != nori_core::sleep::Sleep::Off);
                self.sleep_choices = nori_core::menus::sleep_choices(running);
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
                ui.set_picker_name("".into());
                ui.set_picker_naming(false);
                ui.set_picker_open(true);
                self.picker_shown(Vec::new());
                self.ask_choices(one.id.clone());
                self.menu_song = Some(one);
            }
            8 => self.ask_remove(vec![one.id]),
            9 => self.on_session(|s| s.download_remove(&one.id)),
            10 => self.on_session(|s| s.star(nori_core::client::Starrable::Song, one.id, true)),
            12 | 19 => {
                let done = self.session.as_ref().is_some_and(|s| s.core.mix_excluded_set(one.id, how == 12).is_ok());
                if done {
                    self.say(if how == 12 { words::EXCLUDED_FROM_MIXES } else { words::BACK_IN_MIXES }, false);
                }
            }
            13 => self.share(one.id),
            14 => {
                let ui = self.ui();
                let rows = words::details(&one).into_iter().map(|(label, value)| Detail { label: label.into(), value: value.into(), link: SharedString::default() });
                ui.set_details(ModelRc::new(VecModel::from(rows.collect::<Vec<_>>())));
                ui.set_details_open(true);
                self.details_song = Some(one.id.clone());
                if self.plugins_fill(nori_plugins::Place::SongInfo) {
                    let (host, tx) = (self.plugin_host().clone(), self.tx.clone());
                    nori_host::spawn("nori-plugin-info", move || tx.send(Msg::SongInfo { song: one.id.clone(), groups: nori_core::transport::block_on(host.song_info(one, "en".into())) }));
                }
            }
            k if k >= PLUGIN_MENU => self.plugin_menu_run(3, (k - PLUGIN_MENU) as usize, nori_plugins::Items::Songs(vec![one])),
            _ => {}
        }
        if matches!(how, 3 | 8 | 9 | 10) {
            self.mark_playing();
        }
    }

    /// The sleep timer's choice `i`.
    fn sleep(&mut self, i: usize) {
        let Some(c) = self.sleep_choices.get(i).copied() else { return };
        self.on_session(|s| s.sleep_pick(c));
    }

    /// Fills the lyrics' sheet from the lyrics shown, opening it when `open`.
    fn lyrics_sheet(&self, open: bool) {
        let Some(l) = &self.lyrics else { return };
        let pick = l.pick();
        let ui = self.ui();
        ui.set_lyrics_from(words::lyrics_from(&pick.origin, pick.lyrics.synced, pick.lyrics.word_timed, &|id| self.plugin_name(id)).into());
        ui.set_lyrics_timing(words::lyrics_timing(pick.lyrics.timing).into());
        ui.set_lyrics_shift(words::lyrics_shift(pick.lyrics.timing.offset_ms()).into());
        ui.set_lyrics_yours(matches!(pick.lyrics.timing, nori_core::LyricsTiming::Yours { .. }));
        if open {
            ui.set_lyrics_sheet_open(true);
        }
    }

    /// The lyrics shown a step sooner (`dir` > 0), later (< 0) or back to the check's timing (0); they
    /// come back as [`Msg::Lyrics`].
    fn lyrics_nudge(&mut self, dir: i32) {
        if let Some(s) = &self.session {
            s.lyrics_nudge(dir);
        }
    }

    /// Shows what the session's lyrics view shows for the song heard; another song's is let go.
    fn lyrics_now(&mut self, now: LyricsNow) {
        if now.song() != self.heard.as_deref() {
            return;
        }
        let ui = self.ui();
        match now {
            LyricsNow::Shown { pick, .. } => {
                let l = crate::lyrics::SongLyrics::new(pick, self.position_now(), self.lyric_faces.clone());
                ui.set_lyrics_lines(l.lines());
                ui.set_lyrics_lane(l.lane());
                ui.set_lyrics_synced(l.synced());
                ui.set_lyrics_note(if l.is_empty() { "No lyrics for this song".into() } else { l.credit(&|id| self.plugin_name(id)).into() });
                ui.set_lyrics_waiting(false);
                self.lyrics = Some(l);
                self.lyrics_sheet(false);
                self.lyrics_step(true);
            }
            // The last song's lines stay (faded out while waiting) until this one's come.
            LyricsNow::Looking { .. } => {
                self.lyrics = None;
                ui.set_lyrics_active(-1);
                ui.set_lyrics_note("Looking for lyrics…".into());
                ui.set_lyrics_waiting(true);
            }
            LyricsNow::Failed { .. } | LyricsNow::NoSong => {
                self.lyrics = None;
                ui.set_lyrics_lines(ModelRc::default());
                ui.set_lyrics_active(-1);
                ui.set_lyrics_note(if self.heard.is_some() { "No lyrics for this song".into() } else { "".into() });
                ui.set_lyrics_waiting(false);
            }
        }
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

    /// Asks the core which playlists song `id` may go into and what each has of it, shown when they come.
    fn ask_choices(&self, id: String) {
        let Some(client) = self.session.as_ref().map(|s| s.client.clone()) else { return };
        let tx = self.tx.clone();
        nori_host::spawn("nori-choices", move || {
            let choices = nori_core::transport::block_on(client.playlist_choices(vec![id.clone()])).unwrap_or_default();
            tx.send(Msg::Choices(id, choices));
        });
    }

    /// The picker's playlists, each saying what it has of the song.
    fn picker_shown(&mut self, choices: Vec<nori_core::menus::PlaylistChoice>) {
        let shown = choices.iter().map(|c| PlaylistChoice {
            title: c.playlist.name.as_str().into(),
            sub: words::playlist_holds(c.holds, 1, c.playlist.song_count).into(),
            art: c.playlist.cover_art.clone().unwrap_or_default().into(),
            full: c.adds.is_empty(),
        });
        self.ui().set_picker_choices(ModelRc::new(VecModel::from(shown.collect::<Vec<_>>())));
        self.picker = choices;
    }

    /// Whether the profile may change `p` (the core's `playlist_editable`), signed in as its user.
    fn editable(&self, p: &nori_core::Playlist) -> bool {
        let user = self.root.app.settings.current().and_then(|prefs| prefs.active_server().map(|sv| sv.user.clone())).unwrap_or_default();
        nori_core::menus::playlist_editable(p, &user, self.account())
    }

    /// Adds the song whose menu asked to the picker's playlist `i`, or to a new one named `name`.
    fn add_to_playlist(&mut self, to: Result<usize, String>) {
        self.ui().set_picker_open(false);
        let Some(song) = self.menu_song.take() else { return };
        match to {
            Ok(i) => {
                let Some(c) = self.picker.get(i) else { return };
                let id = c.playlist.id.clone();
                self.playlist_write(nori_core::client::Write::AddToPlaylist { id: id.clone(), song_ids: c.adds.clone() }, words::added_to_playlist(&c.playlist.name), PlaylistChange::Changed(id));
            }
            Err(name) => {
                let name = name.trim().to_string();
                if !name.is_empty() {
                    self.playlist_write(nori_core::client::Write::CreatePlaylist { name: name.clone(), song_ids: vec![song.id] }, words::playlist_created(&name), PlaylistChange::Made);
                }
            }
        }
    }

    /// The playlist shown, while the profile may change it.
    fn page_playlist(&self) -> Option<&nori_core::Playlist> {
        match &self.page {
            Some(PageShown::Playlist(p)) => p.as_ref(),
            _ => None,
        }
    }

    /// Takes entry `index` out of the playlist shown.
    fn remove_from_playlist(&self, index: usize) {
        let Some(p) = self.page_playlist() else { return };
        let write = nori_core::client::Write::RemoveFromPlaylist { id: p.id.clone(), index: index as i32 };
        self.playlist_write(write, words::removed_from_playlist(&p.name), PlaylistChange::Changed(p.id.clone()));
    }

    /// Names the playlist shown `name`.
    fn rename_playlist(&self, name: &str) {
        let (Some(p), name) = (self.page_playlist(), name.trim()) else { return };
        if name.is_empty() {
            return;
        }
        self.ui().set_rename_open(false);
        let write = nori_core::client::Write::RenamePlaylist { id: p.id.clone(), name: name.to_string() };
        self.playlist_write(write, words::playlist_renamed(name), PlaylistChange::Changed(p.id.clone()));
    }

    /// Deletes the playlist shown, once its sheet asked.
    fn delete_playlist(&self) {
        self.ui().set_delete_open(false);
        let Some(p) = self.page_playlist() else { return };
        self.playlist_write(nori_core::client::Write::DeletePlaylist { id: p.id.clone() }, words::playlist_deleted(&p.name), PlaylistChange::Deleted(p.id.clone()));
    }

    /// Sends `write` to the server; `done` is said once it took it.
    fn playlist_write(&self, write: nori_core::client::Write, done: String, change: PlaylistChange) {
        let Some(client) = self.session.as_ref().map(|s| s.client.clone()) else { return };
        let tx = self.tx.clone();
        nori_host::spawn("nori-playlist-write", move || {
            let note = match nori_core::transport::block_on(client.write(write)) {
                Ok(()) => Msg::Playlist { text: done, change },
                Err(e) => Msg::Note { text: words::playlist_failed(&words::net_error(&e)), error: true },
            };
            tx.send(note);
        });
    }

    /// A playlist write the server took: the playlists are read again, and the page of the one it changed,
    /// or Back from it once deleted.
    fn playlist_changed(&mut self, text: &str, change: PlaylistChange) {
        self.say(text, false);
        self.on_session(|s| s.load(Req::Page(PageAsk::Playlists)));
        let on_page = self.ui().get_view() == View::Page;
        let shown = |id: &str| on_page && self.page_playlist().is_some_and(|p| p.id == id);
        match change {
            PlaylistChange::Changed(id) if shown(&id) => self.load(Req::Page(PageAsk::Playlist(id))),
            PlaylistChange::Deleted(id) if shown(&id) => self.back(),
            _ => {}
        }
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

    /// Flips the album's or artist's seal on the page shown, or pins its playlist.
    fn star_page(&self) {
        let ui = self.ui();
        let kind = match ui.get_page_kind() {
            PageLayout::Album => nori_core::client::Starrable::Album,
            PageLayout::Artist => nori_core::client::Starrable::Artist,
            PageLayout::List => {
                if ui.get_page_pinnable() {
                    let on = !ui.get_page_starred();
                    ui.set_page_starred(on);
                    nori_core::browse::pin_playlist(&self.root.app.settings, ui.get_page_id().to_string(), on);
                }
                return;
            }
        };
        let on = !ui.get_page_starred();
        ui.set_page_starred(on);
        let id = ui.get_page_id().to_string();
        self.on_session(|s| s.star(kind, id, on));
    }

    /// Flips `s`'s heart, once: through the device playing while it has the song, else here.
    fn star(&mut self, s: &Song) {
        if self.session.as_ref().is_some_and(|x| !x.rules().stars && x.rules().stars_after_sign_in) {
            return self.ask_sign_in();
        }
        let on = !self.starred(s);
        self.on_session(|x| x.star_song(s.id.clone(), on, self.there()));
        self.mark_playing();
    }

    /// Moves the seek bar on, and again when it next moves a step on screen or its labels a second, as
    /// Android's does (`nori_look::motion`).
    fn seek_step(&mut self) {
        let ui = self.ui();
        let at = self.position_now();
        position_shown(&ui, self.player.as_ref(), at as i32);
        ui.global::<crate::Playing>().set_clock(self.started.elapsed().as_secs_f32());
        let width = match self.player.as_ref().filter(|_| !ui.get_full_player()) {
            Some(p) => p.get_seek_width() * p.window().scale_factor(),
            None => ui.get_full_seek_width() * ui.window().scale_factor(),
        };
        let step = nori_look::motion::step_px(ui.global::<Motion>().get_reduced());
        let wait = nori_look::motion::wait_ms(at, ui.get_duration_ms() as i64, width, self.pace, step, 0.0);
        let me = self.me.clone();
        self.tick.start(TimerMode::SingleShot, Duration::from_millis(wait as u64), move || me.with(|a| a.seek_step()));
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
        if self.session.is_none() || answer.trim().is_empty() || !self.sign_in_on(SignInEvent::Sent) {
            return;
        }
        let Some(s) = &self.session else { return };
        let (h, tx) = ((***s).clone(), self.tx.clone());
        nori_host::spawn("nori-sign-in", move || tx.send(Msg::SignedIn(h.sign_in(answer))));
    }

    /// The account sheet of the profile in use, as its server's Edit and a seal tapped while signed out open
    /// it: who is signed in, or Sign in (the provider's own page in a window) with pasting folded under it.
    /// A server row's Edit: the profile in use's account sheet where its provider signs in to one, else
    /// the profile in the sign-in form, saved over itself.
    fn edit_profile(&mut self, id: String) {
        let prefs = self.root.app.settings.current().unwrap_or_default();
        let Some(profile) = prefs.servers.iter().find(|s| s.id == id).cloned() else { return };
        let providers = self.kinds().providers();
        if id == prefs.active_server_id && providers.iter().any(|i| i.kind == profile.kind && !i.sign_in.is_empty()) {
            return self.ask_sign_in();
        }
        let ui = self.ui();
        if let Some(info) = providers.iter().find(|i| i.kind == profile.kind).or(providers.first()) {
            // A saved profile is renamed from its menu.
            let form = LoginKind { asks_name: false, ..login_form(self.kinds(), info, self.plugged.about_of(&info.kind), nori_core::providers::FirstStep::Form) };
            ui.set_login_form(form);
            self.add_on(AddEvent::Edit { kind: info.kind.clone() });
        }
        ui.set_login_adding(false);
        ui.set_login_name(profile.name.as_str().into());
        ui.set_login_url(profile.url.as_str().into());
        ui.set_login_user(profile.user.as_str().into());
        ui.set_login_password("".into());
        self.editing = Some(profile);
        self.go(View::Login);
    }

    /// The account changed under the open profile: its seals and downloads follow, and a session the
    /// source ended asks to sign in again.
    fn account_changed(&mut self) {
        let Some(s) = self.session.as_ref() else { return };
        let Some(state) = s.account_changed() else { return };
        let rules = s.rules();
        let offer = s.client.sign_in_due(None);
        let ui = self.ui();
        ui.set_downloads(rules.downloads);
        ui.set_page_can_star(rules.account && rules.stars);
        ui.set_favorites_offered(rules.stars);
        ui.set_library_tabs(library_tabs(&rules));
        self.mirror();
        match offer {
            Some(SignInEvent::Close) => {
                self.sign_in_on(SignInEvent::Close);
            }
            Some(offer) if state == nori_core::providers::AccountState::Ended => self.offer_sign_in(offer),
            _ => {}
        }
    }

    /// The first screen: what a new profile plays from.
    fn choose_provider(&mut self) {
        let ui = self.ui();
        ui.set_provider_top(crate::settings::provider_top());
        self.go(View::Login);
        self.add_on(AddEvent::Start);
        self.plugins_look();
        self.plugins_shown();
    }

    /// `kind`'s page, or, with nothing to ask, the kind added at once from the first screen; a kind that
    /// can't be added now stays there.
    fn provider_chosen(&mut self, kind: nori_core::settings::ServerKind) {
        if self.add.busy() {
            return;
        }
        let Some(info) = self.kinds().providers().into_iter().find(|i| i.kind == kind) else { return };
        let then = self.add_on(AddEvent::Chose { info: info.clone() });
        // Asking nothing, its page shows only if adding it at once fails.
        let step = self.add.step().unwrap_or(nori_core::providers::FirstStep::AddNow);
        let ui = self.ui();
        ui.set_login_form(login_form(self.kinds(), &info, self.plugged.about_of(&kind), step));
        ui.set_login_adding(true);
        if then == AddDo::Connect {
            self.login(AfterAdding::Remind);
        }
    }

    /// Turns on plugin `id`, chosen from the first screen, and opens its form, unless the profile waiting
    /// for it opened instead.
    fn provider_turned_on(&mut self, id: String) {
        self.plugin_host().turn(id.clone(), true);
        self.plugins_read();
        if self.plugged.choosing {
            self.provider_chosen(nori_core::settings::ServerKind::Plugin { id });
        }
    }

    /// Opens the waiting profile once its plugin runs.
    fn open_waiting(&mut self) {
        match self.waiting.clone() {
            Some(profile) if self.kinds().waits(profile.clone()).is_none() => self.open(profile, None),
            _ => {}
        }
    }

    /// Why the waiting profile doesn't open, in its plugin's name.
    fn waiting_line(&self) -> Option<String> {
        let profile = self.waiting.as_ref()?;
        let nori_core::settings::ServerKind::Plugin { id } = &profile.kind else { return None };
        let wait = self.kinds().waits(profile.clone())?;
        let name = nori_plugins::plugin_display_name(id.clone(), self.plugged.cards.clone(), self.plugged.offers().to_vec());
        Some(words::profile_waits(&name, wait))
    }

    /// Takes down the sheet over the first screen.
    fn close_plugins(&mut self) {
        if let AddPage::Offered { .. } = self.add.page {
            self.add_on(AddEvent::Back);
        }
        self.plugged.stack.clear();
        self.ui().set_plugins_open(false);
        self.plugins_shown();
    }

    /// The kinds of profile this computer opens: nori's own and its plugins'.
    fn kinds(&self) -> &Arc<nori_core::providers::Kinds> {
        &self.root.plugins().kinds
    }

    /// The sign-in page of the profile in use's provider, in the browser.
    fn open_sign_in_page(&self) {
        let Some(kind) = self.root.app.settings.current().and_then(|p| p.active_server().map(|sv| sv.kind.clone())) else { return };
        let page = self.kinds().providers().into_iter().find(|i| i.kind == kind).and_then(|i| i.page().map(str::to_string));
        if let Some(url) = page {
            open_in_browser(&url);
        }
    }

    fn plugin_host(&self) -> &Arc<nori_plugins::Plugins> {
        &self.root.plugins().host
    }

    /// Reads the plugins and repos again.
    fn plugins_read(&mut self) {
        let host = self.plugin_host().clone();
        let mut cards = host.cards("en".into());
        // What was installed since the repos were asked shows as installed.
        if let crate::settings::Shelf::Read(offers) = &mut self.plugged.shelf {
            host.offers_stand(&mut offers.offers);
        }
        let watched = self.root.plugins().watched.clone();
        self.plugged.off = watched.as_said(&mut cards);
        self.plugged.assets = cards.iter().map(|c| (c.id.clone(), host.assets(c.id.clone()))).filter(|(_, a)| !a.is_empty()).collect();
        self.plugged.cards = cards;
        self.plugged.repos = host.repos();
        self.plugin_menus = MENU_KINDS.map(|k| host.menu_entries(k, "en".into()));
        let ui = self.ui();
        let prefs = self.root.app.settings.current().unwrap_or_default();
        let today = crate::look::Today::now();
        self.plugged.controlled = host.controlled("en".into());
        for d in &mut self.plugged.controlled {
            self.plugged.off.extend(watched.as_said(std::slice::from_mut(&mut d.card)));
        }
        let mut shown_here = host.device_cards("en".into());
        self.plugged.off.extend(watched.as_said(&mut shown_here));
        ui.set_device_plugin_rows(crate::settings::device_rows(&shown_here, &self.plugged.controlled, &self.plugged.off, self.plugged.device_page.as_deref(), &prefs, &self.facts, today));
        let menus = ui.global::<crate::PluginMenus>();
        let labels = |k: usize| ModelRc::new(VecModel::from(self.plugin_menus[k].iter().map(|e| SharedString::from(e.label.as_str())).collect::<Vec<_>>()));
        menus.set_album(labels(0));
        menus.set_artist(labels(1));
        menus.set_playlist(labels(2));
        self.devices_shown();
        self.plugins_shown();
        self.open_waiting();
    }

    /// Draws Settings › Plugins and the page over it as they stand, asking what they show for the first time.
    fn plugins_shown(&mut self) {
        self.plugged.all_off = self.root.app.settings.current().unwrap_or_default().plugins_off;
        let host = self.plugin_host().clone();
        if let Some((id, offer)) = self.plugged.readme_wanted() {
            let tx = self.tx.clone();
            let host = host.clone();
            nori_host::spawn("nori-plugin-readme", move || {
                let blocks = nori_core::transport::block_on(host.readme(id.clone(), offer));
                tx.send(Msg::PluginReadme { id, blocks });
            });
        }
        let icons = self.plugged.icons_wanted();
        if !icons.is_empty() {
            let tx = self.tx.clone();
            nori_host::spawn("nori-plugin-icons", move || {
                let side = session::ICON_SIDE as usize;
                for (id, offer) in icons {
                    let png = nori_core::transport::block_on(host.icon(id.clone(), offer));
                    let pixels = png.and_then(|png| nori_covers::decode::Decoder::new().decode(&png, side, side, nori_covers::Alpha::Straight).ok());
                    tx.send(Msg::PluginIcon { id, pixels });
                }
            });
        }
        let ui = self.ui();
        let prefs = self.root.app.settings.current().unwrap_or_default();
        ui.set_plugin_rows(crate::settings::plugin_rows(&self.plugged, &prefs, &self.facts, crate::look::Today::now()));
        ui.set_plugin_top(crate::settings::plugin_top(&self.plugged, prefs.plugins_off));
        if self.plugged.choosing {
            ui.set_provider_rows(crate::settings::provider_rows(&self.plugged, self.kinds(), &prefs, &self.facts, crate::look::Today::now()));
            if let Some(line) = self.waiting_line() {
                ui.set_login_error(line.into());
            }
        }
        self.settings_shown();
    }

    /// Asks the repos what they offer, once: a plugin page needs it.
    fn plugins_look(&mut self) {
        if matches!(self.plugged.shelf, crate::settings::Shelf::Unasked) {
            self.plugin_act(crate::settings::PluginAct::Look);
        }
    }

    /// `change` to the pages open over Settings › Plugins, shown from their top.
    fn plugin_pages(&mut self, change: impl FnOnce(&mut Vec<crate::settings::PluginPage>)) {
        change(&mut self.plugged.stack);
        let ui = self.ui();
        ui.set_plugin_page_seen(ui.get_plugin_page_seen() + 1);
        self.plugins_look();
        self.plugins_shown();
    }

    /// Whether a running plugin fills `place`: only then are the plugins asked for it.
    fn plugins_fill(&self, place: nori_plugins::Place) -> bool {
        self.plugin_host().running().iter().any(|i| i.manifest.places.contains(&place))
    }

    /// Runs plugin menu action `i` of the menus of `Card::kind` `kind` on `items`, off this thread; what
    /// it says arrives as a notice.
    fn plugin_menu_run(&self, kind: usize, i: usize, items: nori_plugins::Items) {
        let Some(entry) = self.plugin_menus.get(kind).and_then(|m| m.get(i)).cloned() else { return };
        let host = self.plugin_host().clone();
        nori_host::spawn("nori-plugin-menu", move || nori_core::transport::block_on(host.run_menu(entry, items)));
    }

    /// Runs plugin menu action `i` of an album's, artist's or playlist's menu (`Card::kind` `kind`) on
    /// item `id` named `name`.
    fn plugin_card_menu(&self, kind: i32, i: usize, id: String, name: String) {
        use nori_plugins::Items;
        let items = match kind {
            0 => Items::Albums(vec![self.lists.albums.iter().find(|a| a.id == id).cloned().unwrap_or(nori_core::Album { id, name, ..Default::default() })]),
            1 => Items::Artists(vec![nori_core::Artist { id, name, ..Default::default() }]),
            2 => Items::Playlists(vec![self.lists.playlists.iter().find(|p| p.id == id).cloned().unwrap_or(nori_core::Playlist { id, name, ..Default::default() })]),
            _ => return,
        };
        self.plugin_menu_run(kind as usize, i, items);
    }

    /// Asks the plugins for their lines under the player's output, off this thread: on a new song, and as
    /// Now Playing opens.
    fn ask_player_status(&self) {
        if !self.plugins_fill(nori_plugins::Place::PlayerStatus) {
            return self.ui().set_plugin_status(ModelRc::default());
        }
        let (host, tx, song) = (self.plugin_host().clone(), self.tx.clone(), self.heard.clone());
        nori_host::spawn("nori-plugin-status", move || tx.send(Msg::PlayerStatus { song, lines: nori_core::transport::block_on(host.player_status()) }));
    }

    /// Asks the plugins for `song`'s sleeve, off this thread; a still one is fetched and drawn as its
    /// cover in Now Playing, and so is a loop's first frame, until the loop plays over it.
    fn ask_sleeve(&mut self, song: Song) {
        self.sleeve = None;
        self.moving = None;
        self.ui().set_sleeved(false);
        if !self.plugins_fill(nori_plugins::Place::Artwork) {
            return;
        }
        let (host, tx) = (self.plugin_host().clone(), self.tx.clone());
        nori_host::spawn("nori-plugin-artwork", move || {
            let Some(artwork) = nori_core::transport::block_on(host.artwork(song.clone())) else { return };
            let picture = match artwork.looping {
                false => Some(artwork.url.clone()),
                true => {
                    tx.send(Msg::Loop { song: song.clone(), artwork: artwork.clone() });
                    artwork.poster.as_ref().map(|p| p.replace("{size}", &cover_px(CoverSize::Large).to_string()))
                }
            };
            if let Some(url) = picture {
                tx.send(Msg::Artwork { song: song.id, url });
            }
        });
    }

    /// The moving cover plays only while its sleeve shows: the full player open on a window in sight,
    /// music playing and Motion not off. Otherwise its thread goes and the last picture stays.
    fn motion_follow(&mut self) {
        let ui = self.ui();
        let shows = !self.hidden && ui.get_full_player() && ui.get_playing() && !ui.global::<Motion>().get_reduced();
        let Some(moving) = &mut self.moving else { return };
        if !shows {
            moving.playing = None;
            return;
        }
        if moving.playing.is_some() {
            return;
        }
        let (song, artwork) = (moving.song.clone(), moving.artwork.clone());
        let (frames, fails) = (self.tx.clone(), self.tx.clone());
        let id = song.id.clone();
        moving.playing = Some(crate::motion::play(
            artwork.url.clone(),
            moving.kept.clone(),
            move || frames.send(Msg::LoopFrame { song: id.clone() }),
            move |failed| {
                nori_core::alog::info(&format!("moving cover: {failed:?}"));
                let status = match failed {
                    crate::motion::Failed::Status(s) => Some(s),
                    crate::motion::Failed::Unplayable(_) => None,
                };
                fails.send(Msg::LoopFailed { song: song.clone(), artwork: artwork.clone(), status });
            },
        ));
    }

    /// Asks for a line of text: the sheet sends it back by `name` (`setting_typed`).
    fn ask_field(&self, title: &str, name: &str, secret: bool) {
        let ui = self.ui();
        ui.set_ask_title(title.into());
        ui.set_ask_note("".into());
        ui.set_ask_name(name.into());
        ui.set_ask_button(words::SAVE.into());
        ui.set_ask_field(true);
        ui.set_ask_secret(secret);
        ui.set_ask_text("".into());
        ui.set_ask_open(true);
    }

    /// What the ask sheet took for a backup: its file, then its password, which makes or restores it.
    fn backup_typed(&mut self, act: Act, text: String) {
        match act {
            Act::Backup { restore } if !text.trim().is_empty() => self.ask_field(words::BACKUP_PASSWORD, &crate::settings::backup_password(restore, text.trim()), true),
            Act::BackupPassword { restore: false, path } => {
                let tx = self.tx.clone();
                self.root.make_backup(path.into(), text, move |r| tx.send(Msg::BackedUp(r)));
            }
            Act::BackupPassword { restore: true, path } => {
                let tx = self.tx.clone();
                self.root.restore_backup(path.into(), text, move |r| tx.send(Msg::Restored(r.map(Box::new))));
            }
            _ => {}
        }
    }

    /// Asks before downloads `ids` are removed: the sheet's button sends it back to [`App::remove_asked`].
    fn ask_remove(&mut self, ids: Vec<String>) {
        let ui = self.ui();
        let (title, note) = words::remove_downloads(ids.len());
        ui.set_ask_title(title.into());
        ui.set_ask_note(note.into());
        ui.set_ask_name(words::REMOVE_DOWNLOADS.into());
        ui.set_ask_button(words::REMOVE.into());
        ui.set_ask_field(false);
        ui.set_ask_text("".into());
        ui.set_ask_open(true);
        self.removing = ids;
    }

    /// The removal [`App::ask_remove`] asked about, confirmed.
    fn remove_asked(&mut self) {
        let ids = std::mem::take(&mut self.removing);
        self.on_session(|s| ids.iter().for_each(|id| s.download_remove(id)));
        if self.page.is_some() {
            self.page_head();
        }
        self.mark_playing();
    }

    /// Asks before `act` is done: the sheet's button sends it back to [`App::plugin_typed`].
    fn confirm(&self, title: &str, note: &str, button: &str, act: crate::settings::PluginAct) {
        let ui = self.ui();
        ui.set_ask_title(title.into());
        ui.set_ask_note(note.into());
        ui.set_ask_name(act.name().into());
        ui.set_ask_button(button.into());
        ui.set_ask_field(false);
        ui.set_ask_text("".into());
        ui.set_ask_open(true);
    }

    fn plugin_name(&self, id: &str) -> String {
        self.plugged.cards.iter().find(|c| c.id == id).map_or_else(|| id.to_string(), |c| c.name.clone())
    }

    /// Updates the plugins once a day, from the open profile's connection. Called once the window is up
    /// with its session (or the sign-in), which ends the start for safe mode.
    fn update_plugins(&self) {
        self.root.plugins().host.started();
        let tx = self.tx.clone();
        self.root.update_plugins(move |updated| tx.send(Msg::PluginsUpdated(updated)));
    }

    /// Sets a plugin's row, or switches the plugin or its version's hold, to `v`.
    fn plugin_set(&mut self, act: crate::settings::PluginAct, v: nori_plugins::Value) {
        use crate::settings::PluginAct;
        if let (PluginAct::AllOff, nori_plugins::Value::Switch(off)) = (&act, &v) {
            self.setting("pluginsOff", &off.to_string());
            self.plugin_host().switched();
            return self.plugins_read();
        }
        if let (PluginAct::Chip, nori_plugins::Value::Choice(i)) = (&act, &v) {
            self.plugged.repo_shown = (*i as usize).checked_sub(1).and_then(|at| self.plugged.repos.get(at)).map(|r| r.url.clone());
            return self.plugins_shown();
        }
        let host = self.plugin_host();
        match (act, v) {
            (PluginAct::Turn(id), nori_plugins::Value::Switch(on)) => host.turn(id, on),
            (PluginAct::Hold(id), nori_plugins::Value::Switch(on)) => host.hold(id, on),
            (PluginAct::Row(id, row), v) => host.set(id, row, v),
            (PluginAct::Flags(id, row, bits), _) => host.set(id, row, nori_plugins::Value::Flags(bits)),
            (PluginAct::Noise(device), nori_plugins::Value::Choice(i)) => {
                let Some(&mode) = self.plugged.controlled.iter().find(|d| d.id == device).and_then(|d| d.noise_modes.get(i as usize)) else { return };
                return host.set_noise(device, mode);
            }
            _ => return,
        }
        self.plugins_read();
    }

    /// A button of Settings › Plugins.
    fn plugin_act(&mut self, act: crate::settings::PluginAct) {
        use crate::settings::PluginAct;
        if matches!(act, PluginAct::DeviceOpen(_) | PluginAct::DeviceBack) {
            self.plugged.device_page = if let PluginAct::DeviceOpen(device) = act { Some(device) } else { None };
            return self.plugins_read();
        }
        let host = self.plugin_host().clone();
        let ask = |title: &str, name: String, secret: bool| self.ask_field(title, &name, secret);
        match act {
            PluginAct::Row(id, row) => {
                let Some(r) = self.plugged.cards.iter().find(|c| c.id == id).and_then(|c| c.rows.iter().find(|r| r.id == row)) else { return };
                match r.kind {
                    nori_plugins::RowKind::Text | nori_plugins::RowKind::Secret => {
                        let (title, secret) = (r.label.clone(), r.kind == nori_plugins::RowKind::Secret);
                        return ask(&title, PluginAct::Row(id, row).name(), secret);
                    }
                    _ => host.act(id, row),
                }
            }
            PluginAct::Move(id, by) => {
                let Some(at) = self.plugged.cards.iter().position(|c| c.id == id) else { return };
                host.place(id, (at as i32 + by).max(0) as u32);
            }
            PluginAct::Remove(id) => return self.confirm(&words::remove_plugin(&self.plugin_name(&id)), words::REMOVE_PLUGIN_NOTE, words::REMOVE, PluginAct::Remove(id)),
            PluginAct::Install => return ask(words::PLUGIN_PATH, PluginAct::Install.name(), false),
            PluginAct::AddRepo => return ask(words::REPO_ADDRESS, PluginAct::AddRepo.name(), false),
            PluginAct::RemoveRepo(url) => return self.confirm(&words::remove_plugin(&url), "", words::REMOVE, PluginAct::RemoveRepo(url)),
            PluginAct::Open(url) => return open_in_browser(&url),
            PluginAct::Browser => return self.plugin_pages(|s| s.push(crate::settings::PluginPage::Browser)),
            PluginAct::Repos => return self.plugin_pages(|s| s.push(crate::settings::PluginPage::Repos)),
            PluginAct::Page(id) => return self.plugin_pages(|s| s.push(crate::settings::PluginPage::Plugin(id))),
            PluginAct::Back => {
                self.plugged.stack.pop();
                // Back from the page of the plugin the first screen offered closes its sheet.
                if self.plugged.stack.is_empty() && matches!(self.add.page, AddPage::Offered { .. }) {
                    return self.close_plugins();
                }
                return self.plugin_pages(|_| {});
            }
            PluginAct::Choose(kind) => return self.provider_chosen(kind),
            PluginAct::TurnOn(id) => return self.provider_turned_on(id),
            PluginAct::OfferPage(id) => {
                self.add_on(AddEvent::OfferOpened { plugin: id.clone() });
                self.ui().set_plugins_open(true);
                return self.plugin_pages(|s| *s = vec![crate::settings::PluginPage::Plugin(id)]);
            }
            PluginAct::Sheet => {
                self.plugged.stack.clear();
                self.plugins_look();
                self.ui().set_plugins_open(true);
            }
            PluginAct::ChooseAgain => return self.choose_provider(),
            PluginAct::AddSigningIn => return self.login(AfterAdding::SignIn),
            PluginAct::AddAsGuest => return self.login(AfterAdding::Guest),
            PluginAct::Look => {
                self.plugged.shelf = crate::settings::Shelf::Asking;
                let tx = self.tx.clone();
                nori_host::spawn("nori-plugin-offers", move || tx.send(Msg::PluginOffers(nori_core::transport::block_on(host.offers()))));
            }
            PluginAct::Get(offer, version) => {
                let crate::settings::Shelf::Read(offers) = &self.plugged.shelf else { return };
                let Some(v) = offers.offers.get(offer as usize).and_then(|o| o.versions.get(version as usize)).cloned() else { return };
                let tx = self.tx.clone();
                nori_host::spawn("nori-plugin-get", move || tx.send(Msg::PluginPreview(nori_core::transport::block_on(host.preview_version(v, "en".into())))));
                return;
            }
            PluginAct::AllowMobile(id) => host.allow_mobile_assets(id),
            PluginAct::Turn(_) | PluginAct::Hold(_) | PluginAct::AllOff | PluginAct::TrustRepo(_) | PluginAct::InstallPreview | PluginAct::Noise(_) | PluginAct::Flags(..) | PluginAct::DeviceOpen(_) | PluginAct::DeviceBack | PluginAct::Search | PluginAct::Chip => return,
        }
        self.plugins_read();
    }

    /// What the ask sheet took for `act`: a plugin's text, a path to install, or a repo's address,
    /// which is added only once the warning is answered.
    fn plugin_typed(&mut self, act: crate::settings::PluginAct, text: String) {
        use crate::settings::PluginAct;
        let host = self.plugin_host().clone();
        match act {
            PluginAct::Row(id, row) => host.set(id, row, nori_plugins::Value::Text(text)),
            PluginAct::Install => {
                let (tx, path) = (self.tx.clone(), text.trim().to_string());
                nori_host::spawn("nori-plugin-preview", move || tx.send(Msg::PluginPreview(host.preview_file(path, "en".into()))));
                return;
            }
            PluginAct::InstallPreview => {
                let Some(p) = self.preview.take() else { return };
                let tx = self.tx.clone();
                nori_host::spawn("nori-plugin-install", move || tx.send(Msg::PluginInstalled(host.install_preview(p))));
                return;
            }
            PluginAct::AddRepo if !text.trim().is_empty() => {
                let ui = self.ui();
                let url = text.trim().to_string();
                ui.set_ask_title(words::ADD_REPO.into());
                ui.set_ask_note(format!("{}\n\n{url}", words::TRUST_REPO).into());
                ui.set_ask_name(PluginAct::TrustRepo(url).name().into());
                ui.set_ask_button(words::ADD.into());
                ui.set_ask_field(false);
                ui.set_ask_open(true);
                return;
            }
            PluginAct::TrustRepo(url) => {
                host.add_repo(url);
                self.plugins_read();
                return self.plugin_act(PluginAct::Look);
            }
            // What the sheets asked, answered.
            PluginAct::Remove(id) => {
                self.plugged.gone(&id);
                host.uninstall(id);
            }
            PluginAct::RemoveRepo(url) => {
                host.remove_repo(url);
                self.plugins_read();
                return self.plugin_act(PluginAct::Look);
            }
            PluginAct::Search => {
                self.plugged.query = text;
                return self.plugins_shown();
            }
            _ => return,
        }
        self.plugins_read();
    }

    /// The ended session's sheet no longer comes back at start.
    fn sign_in_quiet(&mut self) {
        self.on_session(|s| s.client.stop_reminding());
        self.sign_in_on(SignInEvent::Close);
    }

    /// The account sheet, as asked for: the account's row, a seal tapped while signed out.
    fn ask_sign_in(&mut self) {
        if let Some(offer) = self.session.as_ref().map(|s| s.client.sign_in_offer()) {
            self.offer_sign_in(offer);
        }
    }

    /// The account sheet, opened by `offer`.
    fn offer_sign_in(&mut self, offer: SignInEvent) {
        let Some(profile) = self.root.app.settings.current().and_then(|p| p.active_server().cloned()) else { return };
        self.sign_in_on(offer);
        let ended = self.sign_in.shown == SignInShown::Offered { ended: true };
        let user = if ended { String::new() } else { profile.user.clone() };
        let info = self.kinds().providers().into_iter().find(|i| i.kind == profile.kind);
        let service = words::provider_of(self.kinds(), &profile.kind).name;
        let ui = self.ui();
        ui.set_signin_user(user.into());
        ui.set_signin_code_first(info.as_ref().is_some_and(|i| i.sign_in.first() == Some(&nori_core::providers::SignIn::Code)));
        ui.set_signin_window_too(info.as_ref().is_some_and(|i| i.page().is_some()));
        ui.set_signin_code_words(words::code_words(&service));
        ui.set_signin_service(service.into());
        ui.set_signin_pasting(false);
        self.identities.clear();
        self.identities_shown();
        if !ended && !profile.user.is_empty() && info.is_some_and(|i| i.identities.is_some()) {
            self.read_identities();
        }
    }

    /// Asks who the signed-in account may act as: [`Msg::Identities`] answers.
    fn read_identities(&self) {
        let Some(s) = &self.session else { return };
        let (h, tx) = ((***s).clone(), self.tx.clone());
        nori_host::spawn("nori-identities", move || tx.send(Msg::Identities(h.identities().unwrap_or_default())));
    }

    /// The account sheet's channels, each with its picture once it came, the one the account acts as checked.
    fn identities_shown(&mut self) {
        self.want_pictures(self.identities.iter().filter_map(|i| i.picture.clone()).collect());
        let lines: Vec<IdentityLine> = self
            .identities
            .iter()
            .map(|i| IdentityLine {
                name: i.name.as_str().into(),
                detail: i.detail.clone().unwrap_or_default().into(),
                picture: i.picture.as_ref().and_then(|p| self.plugged.pictures.get(p)).cloned().unwrap_or_default(),
                letter: i.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default().into(),
                chosen: i.chosen,
            })
            .collect();
        let kind = self.root.app.settings.current().and_then(|p| p.active_server().map(|s| s.kind.clone()));
        let kind = kind.and_then(|k| self.kinds().providers().into_iter().find(|i| i.kind == k)).and_then(|i| i.identities);
        let ui = self.ui();
        ui.set_signin_identities_title(kind.map_or("", words::identities_title).into());
        ui.set_signin_identities(ModelRc::new(VecModel::from(lines)));
    }

    /// The account acts as its `index`th identity from now on, checked at once; the profile opens again as it.
    fn choose_identity(&mut self, index: i32) {
        let Some(s) = &self.session else { return };
        let (h, tx) = ((***s).clone(), self.tx.clone());
        for (i, id) in self.identities.iter_mut().enumerate() {
            id.chosen = i as i32 == index;
        }
        self.identities_shown();
        nori_host::spawn("nori-identity", move || tx.send(Msg::IdentityChosen(h.choose_identity(index.max(0) as u32).map_err(|e| words::net_error(&e)))));
    }

    /// Asks for the account pictures in `addresses` not yet decoded; each shows where it is drawn once it came.
    fn want_pictures(&mut self, addresses: Vec<String>) {
        for a in addresses {
            if !self.plugged.pictures.contains_key(&a) {
                let image = cover_image(&self.art, &self.me, a.as_str().into(), 0);
                if image.size().width > 0 {
                    self.plugged.pictures.insert(a, image);
                }
            }
        }
    }

    /// Whether `id` is a saved profile's or an identity's picture.
    fn is_picture(&self, id: &str) -> bool {
        let saved = self.root.app.settings.current().is_some_and(|p| p.servers.iter().any(|s| s.picture.as_deref() == Some(id)));
        saved || self.identities.iter().any(|i| i.picture.as_deref() == Some(id))
    }

    /// Follows what the downloads are doing: the card shows it now, then each second until they are done.
    fn follow_work(&mut self) {
        self.work_shown();
        if self.ui().get_working() && !self.work.running() {
            let me = self.me.clone();
            self.work.start(TimerMode::Repeated, Duration::from_secs(1), move || me.with(App::work_shown));
        }
    }

    /// The progress card as the downloads stand now; gone, and no longer followed, once they are done.
    fn work_shown(&mut self) {
        let progress = self.session.as_ref().and_then(|s| s.progress());
        let ui = self.ui();
        match progress {
            Some(p) => {
                ui.set_work(work_card(&p));
                ui.set_working(true);
            }
            None => {
                self.work.stop();
                ui.set_working(false);
            }
        }
    }

    /// Saved profile `id` renamed; blank, it is called after its provider again.
    fn rename_profile(&mut self, id: &str, name: &str) {
        nori_host::profile_renamed(&self.root.app.settings, id, name);
        self.ui().set_profile_renaming("".into());
        self.profiles_shown();
        self.settings_shown();
    }

    /// The provider's own sign-in page in a window of its own (this program again, `--sign-in`: the system's
    /// web view); where that cannot start, the sheet unfolds pasting with why.
    fn sign_in_window(&mut self) {
        let Some(kind) = self.root.app.settings.current().and_then(|p| p.active_server().map(|sv| sv.kind.clone())) else { return };
        self.sign_in_page(kind, None);
    }

    /// Opens `kind`'s sign-in window, at `start` where its plugin asked for that page.
    fn sign_in_page(&mut self, kind: nori_core::settings::ServerKind, start: Option<String>) {
        let Some(s) = &self.session else { return };
        if self.sign_in_window.as_mut().is_some_and(|w| w.open_now()) {
            return;
        }
        let (h, tx) = ((***s).clone(), self.tx.clone());
        let title = format!("Sign in to {}", words::provider_of(self.kinds(), &kind).name);
        let done = move |id, cookies| tx.send(Msg::SignInWindowDone(id, h.sign_in(cookies)));
        let window = match start {
            Some(start) => nori_sign_in_window::SignInWindow::open_at(self.kinds().clone(), kind, title, start, done),
            None => nori_sign_in_window::SignInWindow::open(self.kinds().clone(), kind, title, done),
        };
        let Some(window) = window else {
            let ui = self.ui();
            ui.set_signin_pasting(true);
            ui.set_signin_error(words::SIGN_IN_WINDOW_FAILED.into());
            return;
        };
        self.sign_in_on(SignInEvent::Sent);
        self.sign_in_window = Some(window);
    }

    fn sign_out(&mut self) {
        self.on_session(|s| s.sign_out());
        self.identities.clear();
        self.identities_shown();
        self.sign_in_on(SignInEvent::SignedOut);
        self.reopen();
    }

    /// Opens the profile in use again, as the settings now keep it (its account signed in or out), on the
    /// page it was on.
    fn reopen(&mut self) {
        if let Some(p) = self.root.app.settings.current().and_then(|p| p.active_server().cloned()) {
            let view = self.ui().get_view();
            self.open(p, None);
            self.go(view);
        }
    }

    /// Pairs with the device whose code `link` is, on the pairing sheet: [`Msg::Paired`] says how it went.
    fn pair(&mut self, link: String) {
        let ui = self.ui();
        let Some(remote) = self.session.as_ref().and_then(|s| s.remote()) else { return };
        if !nori_core::remote::pairing_valid(link.clone()) {
            return ui.set_pair_error(words::pair_failed(nori_core::remote::PairError::NotACode, None).into());
        }
        ui.set_pair_busy(true);
        ui.set_pair_error("".into());
        let tx = self.tx.clone();
        nori_host::spawn("nori-pair", move || {
            let name = nori_core::remote::pairing_name(link.clone());
            let paired = nori_core::transport::block_on(remote.pair(link)).map(|d| d.name).map_err(|e| words::pair_failed(e, name.as_deref()));
            tx.send(Msg::Paired(paired))
        });
    }

    fn join_jam(&mut self, link: String) {
        let ui = self.ui();
        // A pairing code pasted to join pairs, on the pairing sheet.
        if nori_core::remote::pairing_valid(link.clone()) {
            ui.set_join_open(false);
            ui.set_pair_link(link.as_str().into());
            ui.set_pairing_open(true);
            return self.pair(link);
        }
        if !nori_core::remote::is_invite(&link) {
            return ui.set_join_error(words::NOT_AN_INVITE.into());
        }
        ui.set_join_busy(true);
        ui.set_join_error("".into());
        let (http, tx, remote, settings) = (self.http.clone(), self.tx.clone(), self.session.as_ref().and_then(|s| s.remote()), self.root.app.settings.clone());
        nori_host::spawn("nori-jam-join", move || {
            let joined = nori_host::jam_join(http, &settings, link, &nori_host::device_name(), remote);
            tx.send(Msg::Joined(joined.map_err(|e| words::jam_join_failed(&e))))
        });
    }

    /// The guest profile joined with: opened in place of the user's own, which is opened again on leaving.
    fn joined(&mut self, pass: nori_core::remote::JamPass) {
        let ui = self.ui();
        ui.set_join_busy(false);
        ui.set_join_open(false);
        ui.set_join_link("".into());
        let guest = nori_host::jam_joined(&self.root.app.settings, pass, words::JAM_GUEST);
        self.open(guest, None);
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
        let back = nori_host::jam_left(&self.root.app.settings);
        self.playback = Playback::Here;
        self.asked.clear();
        match back {
            Some(p) => self.open(p, None),
            None => {
                if let Some(s) = self.session.take() {
                    s.close_for(None);
                }
                self.choose_provider();
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
                        if self.ui().get_view() == View::Equalizer {
                            self.eq_extras();
                        }
                    }
                    Event::Buffering(b) if matches!(self.playback, Playback::Here) => {
                        self.ui().set_buffering(*b);
                        self.advancing();
                    }
                    Event::Error { message, .. } => self.say(&format!("Could not play: {message}"), true),
                    Event::Waiting(true) => self.say("Waiting for the server…", true),
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
            // The search page and the top pill's preview both show it; the core already dropped a stale one.
            Msg::Search(v) => self.show_search(v),
            Msg::Lyrics(now) => self.lyrics_now(now),
            Msg::Facts(f) => {
                self.facts = *f;
                self.settings_shown();
            }
            Msg::Note { text, error } => self.say(&text, error),
            Msg::Playlist { text, change } => self.playlist_changed(&text, change),
            Msg::Shared(Some(link)) => self.copy(link, words::LINK_COPIED),
            Msg::Choices(id, choices) => {
                if self.ui().get_picker_open() && self.menu_song.as_ref().is_some_and(|s| s.id == id) {
                    self.picker_shown(choices);
                }
            }
            Msg::Offer(offer) => {
                let (playing, here) = (self.ui().get_playing(), self.heard.clone());
                self.offer.found(offer, here, playing);
                self.offer_shown();
            }
            Msg::Shared(None) => self.say(words::SHARE_FAILED, true),
            Msg::Ask { text, action } => self.ask(&text, &action),
            Msg::Reachable(Err(e)) => self.say(&e, true),
            Msg::Reachable(Ok(())) | Msg::From(..) => {}
            Msg::Cast => {
                self.devices_shown();
                self.output_shown();
            }
            Msg::Remote => {
                self.playback_read();
                self.devices_shown();
                self.jam_shown();
                // Another device or the jam may have changed the queue (a song accepted, added from afar).
                self.follow();
            }
            Msg::Jam(Ok(())) => {
                // The jam shows in the queue panel.
                self.ui().set_inspector(Panel::Queue);
                self.jam_shown();
            }
            Msg::Jam(Err(e)) => self.say(&format!("{} ({e})", words::JAM_FAILED), true),
            Msg::Joined(Ok(pass)) => self.joined(pass),
            Msg::Joined(Err(e)) => {
                let ui = self.ui();
                ui.set_join_busy(false);
                ui.set_join_error(e.into());
            }
            Msg::Paired(paired) => {
                let ui = self.ui();
                ui.set_pair_busy(false);
                match paired {
                    Ok(name) => {
                        ui.set_pairing_open(false);
                        ui.set_pair_link("".into());
                        self.say(&words::paired_with(&name), false);
                    }
                    Err(e) => ui.set_pair_error(e.into()),
                }
                self.devices_shown();
                self.jam_shown();
            }
            Msg::JamEnded(host) => self.left(&words::jam_ended(host.as_deref())),
            Msg::Working => self.follow_work(),
            Msg::Pictured => {
                self.profiles_shown();
                self.settings_shown();
            }
            Msg::Identities(v) => {
                self.identities = v;
                self.identities_shown();
            }
            Msg::IdentityChosen(Ok(())) => {
                self.reopen();
                self.read_identities();
            }
            Msg::IdentityChosen(Err(e)) => {
                self.say(&format!("{} ({e})", words::IDENTITY_FAILED), true);
                self.read_identities();
            }
            Msg::Starred => {
                self.on_session(|s| s.mpris_changed());
                self.follow();
                self.mark_playing();
                // The Favorites tab lists what is liked now: a like from the player or a row joins or leaves it.
                if self.ui().get_view() == View::Favorites {
                    self.load(Req::Page(PageAsk::Favorites));
                    // Read again in place: the list stays while it comes.
                    self.ui().set_loading(false);
                }
                if let Some(s) = &self.session {
                    let core = s.core.clone();
                    let taste = self.root.app.settings.prefs(|p| p.taste_model);
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
                    Ok(Some(text)) => match self.root.app.settings.sound_tool(nori_core::settings_store::SoundTool::Import { text }) {
                        Ok(change) => {
                            if let Some(change) = change {
                                self.on_session(|s| s.applied(change.effect));
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
            Msg::PluginInstalled(r) => {
                self.plugins_read();
                match r {
                    Ok(id) => {
                        self.say(words::PLUGIN_INSTALLED, false);
                        if self.add.page == (AddPage::Offered { plugin: id.clone() }) {
                            self.close_plugins();
                            self.provider_turned_on(id);
                        }
                    }
                    Err(e) => self.say(&words::plugin_refused(&e), true),
                }
            }
            // What it is and may reach, before it is installed.
            Msg::PluginPreview(Ok(p)) => {
                self.confirm(&words::install_version(&p.card.name, &p.card.version), &words::preview_note(&p, self.add.page == (AddPage::Offered { plugin: p.card.id.clone() }) || !self.plugged.cards.iter().any(|c| c.id == p.card.id)).join("\n\n"), words::INSTALL, crate::settings::PluginAct::InstallPreview);
                self.preview = Some(p);
            }
            Msg::PluginPreview(Err(e)) => self.say(&words::plugin_refused(&e), true),
            Msg::SongInfo { song, groups } => {
                let ui = self.ui();
                if !ui.get_details_open() || self.details_song.as_ref() != Some(&song) {
                    return;
                }
                let mut rows: Vec<Detail> = ui.get_details().iter().collect();
                rows.extend(groups.into_iter().flat_map(|g| {
                    let name = SharedString::from(g.name);
                    g.lines.into_iter().map(move |l| Detail { label: name.clone(), value: l.words.into(), link: l.link.unwrap_or_default().into() })
                }));
                ui.set_details(ModelRc::new(VecModel::from(rows)));
            }
            Msg::PlayerStatus { song, lines } if song == self.heard => {
                self.ui().set_plugin_status(ModelRc::new(VecModel::from(lines.into_iter().map(SharedString::from).collect::<Vec<_>>())));
            }
            Msg::PlayerStatus { .. } => {}
            Msg::Artwork { song, url } if Some(&song) == self.heard.as_ref() => {
                let Some(covers) = self.session.as_ref().and_then(|s| s.covers.clone()) else { return };
                let (tx, px) = (self.tx.clone(), cover_px(CoverSize::Large));
                self.sleeve = Some(covers.request(&url, px, px, move |r| {
                    if let Ok(image) = r {
                        tx.send(Msg::Sleeve { song, image });
                    }
                }));
            }
            Msg::Artwork { .. } => {}
            Msg::Sleeve { song, image } if Some(&song) == self.heard.as_ref() => {
                let ui = self.ui();
                ui.set_sleeve(self.compositor.picture(&image.pixels, image.width, image.height));
                ui.set_sleeved(true);
            }
            Msg::Sleeve { .. } => {}
            Msg::Loop { song, artwork } if Some(&song.id) == self.heard.as_ref() => {
                self.moving = Some(Moving { song, artwork, kept: Default::default(), playing: None });
                self.motion_follow();
            }
            Msg::Loop { .. } => {}
            Msg::LoopFrame { song } => {
                let Some(playing) = self.moving.as_ref().and_then(|m| m.playing.as_ref()) else { return };
                if Some(&song) == self.heard.as_ref() {
                    let image = playing.shown.lock().expect("not poisoned");
                    let ui = self.ui();
                    ui.set_sleeve(self.compositor.picture(&image.pixels, image.width, image.height));
                    ui.set_sleeved(true);
                }
                playing.busy.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            Msg::LoopFailed { song, artwork, status } => {
                // A failure of the song before leaves the new one's loop playing.
                if Some(&song.id) == self.heard.as_ref() {
                    self.moving = None;
                }
                if let Some(status) = status {
                    let host = self.plugin_host().clone();
                    nori_host::spawn("nori-plugin-artwork", move || host.artwork_failed(artwork, song, status));
                }
            }
            Msg::PluginReadme { id, blocks } => {
                self.plugged.readme_came(&id, blocks);
                self.plugins_shown();
            }
            Msg::PluginIcon { id, pixels } => {
                let side = session::ICON_SIDE;
                let icon = pixels.map(|px| slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(&px, side, side)));
                self.plugged.icons.read.insert(id, icon);
                self.plugins_shown();
            }
            Msg::PluginOffers(o) => {
                self.plugged.shelf = crate::settings::Shelf::Read(o);
                self.plugins_read();
            }
            Msg::PluginsChanged => {
                self.plugins_read();
                self.output_shown();
                self.devices_shown();
            }
            Msg::BackedUp(r) => match r {
                Ok(()) => self.say(words::BACKUP_SAVED, false),
                Err(f) => self.say(&words::backup_failed(&f), true),
            },
            Msg::Restored(Ok(r)) => {
                self.on_session(|s| s.backup_restored(&r.settings));
                self.settings_shown();
                self.jam_shown();
                self.plugins_read();
                self.say(&words::restored(&r), false);
            }
            Msg::Restored(Err(f)) => self.say(&words::backup_failed(&f), true),
            Msg::PluginsUpdated(updated) => {
                let named: Vec<(String, String)> = updated.into_iter().map(|(id, to)| (self.plugged.cards.iter().find(|c| c.id == id).map_or(id, |c| c.name.clone()), to)).collect();
                self.say(&words::plugins_updated(&named), false);
                self.plugins_read();
            }
            Msg::PluginSignIn { plugin, url } => self.sign_in_page(nori_core::settings::ServerKind::Plugin { id: plugin }, Some(url)),
            Msg::CodeStep { code, event } => {
                if self.code.as_ref().is_some_and(|c| c.0 == code) {
                    self.sign_in_on(event);
                }
            }
            Msg::SignInWindowDone(pid, r) => {
                self.sign_in_window.take_if(|w| w.id() == pid);
                let signed_in = matches!(r, nori_core::providers::SignInResult::SignedIn { .. });
                self.sign_in_on(SignInEvent::Answered { result: r });
                if signed_in {
                    self.reopen();
                }
            }
            Msg::SignedIn(r) => {
                let signed_in = matches!(r, nori_core::providers::SignInResult::SignedIn { .. });
                self.sign_in_on(SignInEvent::Answered { result: r });
                if signed_in {
                    self.ui().set_signin_text("".into());
                    self.reopen();
                }
            }
            Msg::LoggedIn(r) => match r {
                Ok(p) => {
                    let added = match self.add_on(AddEvent::Accepted) {
                        AddDo::Open { added } => added,
                        _ => None,
                    };
                    self.editing = None;
                    self.ui().set_login_password("".into());
                    self.open(p, added);
                }
                Err(said) => {
                    self.add_on(AddEvent::Refused { said });
                }
            },
        }
    }

    /// Requests the covers found missing while drawing.
    fn ask_covers(&mut self) {
        let (session, mixes) = (self.session.as_ref(), &self.mixes);
        self.art.borrow_mut().ask(Instant::now(), |k| {
            // A mix card's first cover brings its colors, which the card wears.
            let colours = k.size != CoverSize::Card || mixes.iter().any(|m| m.covers.first() == Some(&k.id));
            session?.cover(k.clone(), cover_px(k.size), colours)
        });
    }

    fn cover(&mut self, key: CoverKey, image: &Picture, colours: Option<Box<CoverColours>>) {
        let id = key.id.clone();
        let large = key.size != CoverSize::Card;
        let picture = self.compositor.picture(&image.pixels, image.width, image.height);
        let small = key.size == CoverSize::Card;
        self.art.borrow_mut().insert(key, picture.clone(), colours, Instant::now());
        if small && self.is_picture(&id) {
            self.plugged.pictures.insert(id.clone(), picture);
            self.profiles_shown();
            self.settings_shown();
            self.identities_shown();
        }
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
            if ui.get_view() == View::Page && ui.get_page_art() == id.as_str() {
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
            p.set_noise_on(ui.get_noise_on());
            p.set_noise_mode(ui.get_noise_mode());
            p.set_noise_label(ui.get_noise_label());
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
            t.set_tabs(ui.get_library_tabs());
            t.set_previewing(ui.get_search_preview_shown());
        }
    }

    /// Positions the pills; hidden over Now Playing and the login page.
    fn place_player(&self) {
        let ui = self.ui();
        let shown = !ui.get_full_player() && ui.get_view() != View::Login;
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
        let full = ui.get_full_player() && ui.get_full_panel() == Panel::Lyrics;
        let side = ui.get_inspector() == Panel::Lyrics;
        let now = l.advance(at, full || side, force);
        ui.set_lyrics_active(now.active);
        ui.set_lyric_sweeping(now.sweeping);
        let width = ui.window().size().to_logical(ui.window().scale_factor()).width;
        let mut pieces = |view, part, on: bool, size, w, lit| if on && now.sweeping { l.pieces(view, part, &now, size, w, lit) } else { Vec::new() };
        // Every line is drawn in the rows the sung pieces take, a hair narrower than the view so the
        // window's own wrapping never breaks them again. Backing vocals are lit to 70 %, as app.slint draws them.
        let (side_w, full_w) = (320.0 - 44.0 - 2.0, width / 2.0 - 140.0 - 2.0);
        renew(&self.pieces[0], pieces(0, Part::Lead, side, 22.0, side_w, 1.0));
        renew(&self.pieces[1], pieces(1, Part::Lead, full, 44.0, full_w, 1.0));
        renew(&self.backing_pieces[0], pieces(0, Part::Backing, side, 22.0, side_w, 0.7));
        renew(&self.backing_pieces[1], pieces(1, Part::Backing, full, 44.0, full_w, 0.7));
        if let Some([lead, backing]) = side.then(|| l.rows(0, 22.0, side_w)).flatten() {
            ui.set_lyrics_side_rows(lead);
            ui.set_lyrics_side_backing_rows(backing);
        }
        if let Some([lead, backing]) = full.then(|| l.rows(1, 44.0, full_w)).flatten() {
            ui.set_lyrics_full_rows(lead);
            ui.set_lyrics_full_backing_rows(backing);
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
        let song = self.queue.as_ref().and_then(|q| q.songs.get(index).cloned());
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
        self.offer_undo(words::removed(&song.title), Taken::Song(song.id));
    }

    /// Takes the songs after the one playing out of the queue shown (here, or on the device playing), and
    /// offers their undo for a few seconds.
    fn clear_queue(&mut self) {
        let count = self.queue.as_ref().map_or(0, |q| q.upcoming().count());
        if count == 0 {
            return;
        }
        self.on_session(|s| s.clear_upcoming());
        self.offer_undo(words::cleared(count), Taken::Cleared);
    }

    /// Shows `text` with Undo beside it for as long as the core offers to put `taken` back.
    fn offer_undo(&mut self, text: String, taken: Taken) {
        self.ui().set_queue_undo(text.into());
        self.taken = Some(taken);
        let weak = self.me.clone();
        self.undo_timer.start(TimerMode::SingleShot, Duration::from_millis(nori_core::undo::UNDO_MS as u64), move || {
            weak.with(|a| {
                a.taken = None;
                a.ui().set_queue_undo("".into());
            })
        });
        self.follow();
    }

    /// Follows the other devices while their panel is open (remote control, plugins' devices), and
    /// lists them.
    fn devices_watched(&self) {
        let ui = self.ui();
        let open = devices_open(&ui);
        if let Some(s) = &self.session {
            s.output_picker(open);
        }
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
        let cast = self.cast();
        // A jam guest's music is the jam's; it moves to no other device.
        let host = &self.root.plugins().host;
        let targets = host.cast_targets();
        ui.set_devices_on((remote.is_some() || cast.is_some() || host.finds_outputs() || ui.get_device_plugin_rows().row_count() > 0) && !self.guest());
        if !devices_open(&ui) || (remote.is_none() && cast.is_none() && targets.is_empty()) {
            return;
        }
        let active = self.there().map(|e| e.mirror.id.as_str());
        let here = matches!(nori_core::destination::destination(remote.clone(), cast.as_ref().map(Into::into)), nori_core::destination::Destination::Here { .. });
        let mut rows = vec![words::device_row(None, here)];
        rows.extend(cast.as_ref().map(words::cast_row));
        rows.extend(targets.iter().map(words::target_row));
        if let Some(r) = remote {
            let mut devices = r.devices();
            let names = nori_core::remote::device_names(devices.clone(), r.me(), words::kind_words());
            devices.iter_mut().zip(names).for_each(|(d, name)| d.name = name);
            rows.extend(devices.iter().map(|d| words::device_row(Some(d), active == Some(d.id.as_str()))));
            rows.extend(r.outputs().iter().map(words::reached_row));
        }
        devices_set(&ui, rows);
    }

    /// The controlled device this computer's output plays on.
    fn output_device(&self) -> Option<&nori_plugins::ControlledDevice> {
        nori_plugins::output_device(&self.plugged.controlled, (!self.output.is_empty()).then_some(self.output.as_str()))
    }

    /// The headphones this computer plays on, when a plugin controls their noise and the music is this
    /// computer's own: not another device's, a cast's or a jam's.
    fn noise_headphones(&self) -> Option<&nori_plugins::ControlledDevice> {
        let remote = self.session.as_ref().and_then(|s| s.remote());
        let cast = self.cast();
        let here = matches!(nori_core::destination::destination(remote, cast.as_ref().map(Into::into)), nori_core::destination::Destination::Here { hosting: false });
        self.output_device().filter(|d| here && !d.noise_modes.is_empty())
    }

    /// The noise button: the headphones on to their next mode.
    fn noise_next(&self) {
        let Some(d) = self.noise_headphones() else { return };
        if let Some(next) = nori_plugins::next_noise(d.noise_cycle.clone(), d.noise) {
            self.root.plugins().host.set_noise(d.id.clone(), next);
        }
    }

    /// A plugin's cast, while one runs.
    fn cast(&self) -> Option<nori_plugins::Cast> {
        self.root.plugins().host.cast()
    }

    /// Where the sound goes, for the output buttons: the device playing while it is another one, else this
    /// computer's output as the core's `output_look` reads it (this computer until the first song opens one).
    fn output_shown(&self) {
        use nori_core::outputs::{output_look, OutputGlyph};
        use nori_core::destination::Destination;
        let remote = self.session.as_ref().and_then(|s| s.remote());
        let cast = self.cast();
        let (elsewhere, glyph, name) = match (nori_core::destination::destination(remote, cast.as_ref().map(Into::into)), self.there(), cast) {
            (Destination::Mirroring { .. }, Some(e), _) => (true, words::device_glyph(e.mirror.kind), e.mirror.name.clone()),
            (Destination::Cast { .. }, _, Some(c)) => (true, if c.look == nori_plugins::CastLook::Headphones { 1 } else { 3 }, c.device),
            // Headphones a plugin controls by their own name, with their battery.
            _ if self.output_device().is_some() => {
                let d = self.output_device().expect("just found");
                (false, 2, d.level.map_or_else(|| d.name.clone(), |l| words::with_level(&d.name, l)))
            }
            _ if self.output.is_empty() => (false, 0, words::THIS_COMPUTER.to_string()),
            _ => {
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
        self.noise_shown();
    }

    /// The noise button in both players. Gone, it keeps its last mode to fade out with.
    fn noise_shown(&self) {
        use nori_plugins::Noise;
        let ui = self.ui();
        let d = self.noise_headphones();
        ui.set_noise_on(d.is_some());
        if let Some(d) = d {
            ui.set_noise_mode(match d.noise {
                None | Some(Noise::Off) => 0,
                Some(Noise::Canceling) => 1,
                Some(Noise::Transparency) => 2,
                Some(Noise::Adaptive) => 3,
            });
            ui.set_noise_label(d.noise.map_or("", words::noise).into());
        }
        if let Some(p) = &self.player {
            p.set_noise_on(ui.get_noise_on());
            p.set_noise_mode(ui.get_noise_mode());
            p.set_noise_label(ui.get_noise_label());
        }
    }

    /// Moves the music to device `id`, or here ("").
    fn pick_device(&self, id: String) {
        if id == words::CAST_ROW {
            return;
        }
        let host = &self.root.plugins().host;
        if let Some(target) = host.cast_targets().into_iter().find(|t| words::target_id(t) == id) {
            return host.cast_to(target);
        }
        if let Some(r) = self.session.as_ref().and_then(|s| s.remote()) {
            if let Some(output) = r.outputs().into_iter().find(|o| words::reached_id(o) == id) {
                if self.cast().is_some() {
                    host.cast_button(nori_plugins::CastButton::Stop);
                }
                return r.cast_on(output);
            }
        }
        // The music goes to one place: wherever it goes now, a cast's device stops.
        if self.cast().is_some() {
            self.root.plugins().host.cast_button(nori_plugins::CastButton::Stop);
        }
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
        if moved {
            self.on_session(|s| s.mpris_changed());
        }
    }

    /// The gear menu's profiles in their saved order, the one in use checked where it stands, and its mark
    /// on the top pill.
    fn profiles_shown(&mut self) {
        let prefs = self.root.app.settings.current().unwrap_or_default();
        self.want_pictures(prefs.servers.iter().filter_map(|s| s.picture.clone()).collect());
        let ui = self.ui();
        let profiles = crate::settings::profile_items(&self.plugged, &prefs, self.kinds());
        if let (Some(t), Some(on)) = (&self.top, profiles.iter().find(|p| p.on)) {
            t.set_profile_mark(on.mark.clone());
        }
        ui.set_profiles(ModelRc::new(VecModel::from(profiles)));
        ui.set_profiles_editable(self.facts.rules.settings.contains(&nori_core::settings_pages::SettingsPart::ServerOptions));
    }

    /// Redraws the settings or equalizer page from the current settings.
    fn settings_shown(&mut self) {
        self.repaint();
        let ui = self.ui();
        let prefs = self.root.app.settings.current().unwrap_or_default();
        self.devices_shown();
        self.profiles_shown();
        let today = crate::look::Today::now();
        let view = ui.get_view();
        if view == View::Settings {
            // The core's pages, then Plugins.
            let mut tabs: Vec<SharedString> = crate::settings::tabs(&self.facts.rules.settings).into_iter().map(|p| SharedString::from(crate::settings::tab_name(p))).collect();
            let plugins_tab = tabs.len() as i32;
            tabs.push(words::PLUGINS.into());
            ui.set_settings_tabs(ModelRc::new(VecModel::from(tabs)));
            let tab = ui.get_settings_tab();
            ui.set_plugins_tab(tab == plugins_tab);
            ui.set_plugin_top(crate::settings::plugin_top(&self.plugged, prefs.plugins_off));
            ui.set_settings(if tab == plugins_tab {
                crate::settings::plugin_rows(&self.plugged, &prefs, &self.facts, today)
            } else {
                crate::settings::rows(&self.root.app.settings.model, &prefs, &self.facts, self.kinds(), &self.plugged, tab, today)
            });
        }
        if view == View::Equalizer {
            crate::eq::fill(&ui, &prefs);
            self.eq_extras();
        }
    }

    fn setting(&mut self, name: &str, value: &str) {
        // The first screen has no session, and its plugins' switch still works.
        let changed = match &self.session {
            Some(s) => s.setting(name, value),
            None => self.root.app.settings.edit_by_name(name, value),
        };
        if changed.is_none() {
            self.say(&format!("{name}: not a setting"), true);
        }
        self.settings_shown();
        self.jam_shown();
    }

    /// The `Jam` global from the jam this computer hosts; nothing is asked of a remote while jams are off.
    fn jam_shown(&mut self) {
        let ui = self.ui();
        let g = ui.global::<crate::Jam>();
        let guest = self.guest();
        let jams = guest || self.root.app.settings.prefs(|p| p.remote_control);
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
        let paired: Vec<_> = remote.as_ref().filter(|_| !pairing.is_empty()).map(|r| r.paired().iter().map(words::paired_row).collect()).unwrap_or_default();
        show_in(ui.get_paired(), paired, |m| ui.set_paired(m));
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
            let home = nori_core::remote::parse_invite(&link).and_then(|(servers, _)| servers.into_iter().next()).filter(|s| nori_core::remote::is_home_only(s));
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
        self.copy(self.jam_link.clone(), words::LINK_COPIED);
    }

    fn copy(&self, text: String, said: &str) {
        let copied = arboard::Clipboard::new().and_then(|mut c| c.set_text(text));
        match copied {
            Ok(()) => self.say(said, false),
            Err(e) => self.say(&e.to_string(), true),
        }
    }

    /// Tells the session when the sound page in sight changes as `view` shows; the new one first, so
    /// the buffer holds from one to the next.
    fn follow_sound_page(&mut self, view: View) {
        let now = self.sound_page_of(view);
        if now == self.sound_page {
            return;
        }
        let was = std::mem::replace(&mut self.sound_page, now);
        self.on_session(|s| {
            if let Some(page) = now {
                s.sound_page(page, true);
            }
            if let Some(page) = was {
                s.sound_page(page, false);
            }
        });
    }

    /// The equalizer page (with its AutoEQ search), Settings on its Sound tab, or the plugins' tab.
    fn sound_page_of(&self, view: View) -> Option<nori_core::tuning::SoundPage> {
        use nori_core::tuning::SoundPage;
        match view {
            View::Equalizer => Some(SoundPage::Equalizer),
            View::Settings => {
                let tabs = crate::settings::tabs(&self.facts.rules.settings);
                let tab = self.ui().get_settings_tab() as usize;
                if tab == tabs.len() { Some(SoundPage::Plugin) } else { tabs.get(tab).copied().and_then(SoundPage::of_settings) }
            }
            _ => None,
        }
    }

    /// Edits a level in place; redraws the page when the slider is released.
    fn slid(&mut self, name: &str, v: f32, last: bool) {
        let Some(level) = crate::settings::level_of(name) else { return };
        if let Some((effect, _)) = self.root.app.settings.edit_level(level, v) {
            self.on_session(|s| s.applied(effect));
        }
        if last {
            self.settings_shown();
        }
    }

    /// Sets band `i` as the curve or its editor left it; the curve updates at once, the page on release.
    /// `pos` is its place across the curve (a graphic band keeps its own).
    fn eq_band(&mut self, i: u32, point: nori_core::settings_store::BandPoint, last: bool) {
        let settings = self.root.app.settings.clone();
        if let Some(effect) = settings.edit_point(i, point) {
            self.on_session(|s| s.applied(effect));
        }
        if last {
            self.settings_shown();
        } else if let Some(p) = settings.current() {
            crate::eq::curve_only(&self.ui(), &p);
        }
    }

    /// What was typed for `field` of band `i` (`settings::band_typed`); the page shows the band as kept, or
    /// as it was when the words read as nothing.
    fn eq_typed(&mut self, i: u32, field: nori_core::settings::BandField, text: &str) {
        if let Some(effect) = self.root.app.settings.edit_typed(i, field, text) {
            self.on_session(|s| s.applied(effect));
        }
        self.settings_shown();
    }

    /// Adds a parametric band where the curve was clicked.
    fn eq_add(&mut self, pos: f32, gain_db: f32) {
        let freq = nori_core::dsp::band_freq_at(pos);
        self.sound_tool(nori_core::settings_store::SoundTool::AddBand { freq, gain_db });
    }

    fn eq_tool(&mut self, name: &str, i: usize) {
        use nori_core::settings_store::SoundTool;
        let tool = match name {
            "preset" => nori_core::dsp::eq_presets().get(i).cloned().map(|preset| SoundTool::Preset { preset }),
            "auto-preamp" => Some(SoundTool::AutoPreamp { automatic: true }),
            "manual-preamp" => Some(SoundTool::AutoPreamp { automatic: false }),
            "remove-band" => Some(SoundTool::RemoveBand { index: i as u32 }),
            "reset" => Some(SoundTool::ResetBands),
            _ => None,
        };
        if let Some(tool) = tool {
            self.sound_tool(tool);
        }
    }

    fn sound_tool(&mut self, tool: nori_core::settings_store::SoundTool) {
        match self.root.app.settings.sound_tool(tool) {
            Ok(Some(change)) => {
                self.on_session(|s| s.applied(change.effect));
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
        let store = &self.root.app.settings;
        let Some(prefs) = store.current() else { return };
        let effect = store.put(prefs.with_sound(sound));
        self.on_session(|s| s.applied(effect));
        self.settings_shown();
    }

    fn profile_save(&mut self, name: &str) {
        let name = name.trim();
        let (Some(s), Some(prefs)) = (&self.session, self.root.app.settings.current()) else { return };
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

    /// Row `id` of a ranked list (`kind`: a lyrics service, a profile) dragged `by` places (down +).
    fn row_placed(&mut self, kind: i32, id: &str, by: i32) {
        let Some(p) = self.root.app.settings.current() else { return };
        let (name, order): (&str, Vec<String>) = if kind == crate::settings::SERVER {
            ("profilePlace", p.servers.iter().map(|s| s.id.clone()).collect())
        } else {
            let s = nori_core::settings_model::state(&p, nori_core::settings_model::Output::default(), &self.root.app.settings.model);
            ("lyricsPlace", s.lyrics_sources.into_iter().map(|x| x.id).collect())
        };
        let Some(at) = order.iter().position(|x| x == id) else { return };
        let to = (at as i32 + by).clamp(0, order.len() as i32 - 1) as usize;
        if to != at {
            self.setting(name, &format!("{id}:{to}"));
        }
    }

    /// The icon picker of profile `id`'s circle.
    fn icon_picker(&self, id: &str) -> crate::IconPicker {
        let prefs = self.root.app.settings.current().unwrap_or_default();
        let Some(p) = prefs.servers.iter().find(|s| s.id == id) else { return crate::IconPicker::default() };
        crate::settings::icon_picker(&self.plugged, p, &words::profile_label(p, self.kinds()), &words::provider_of(self.kinds(), &p.kind).name)
    }

    /// Profile `id`'s circle shows the icon at `at` among those offered.
    fn profile_icon_picked(&mut self, id: &str, at: usize) {
        let Some(&icon) = nori_core::settings::ProfileIcon::ALL.get(at) else { return };
        nori_host::profile_look(&self.root.app.settings, id, |p| p.icon = icon);
        self.settings_shown();
    }

    fn setting_action(&mut self, name: &str) {
        match Act::of(name) {
            Some(Act::Equalizer) => self.go(View::Equalizer),
            Some(Act::AddServer) => {
                self.editing = None;
                let ui = self.ui();
                for set in [AppWindow::set_login_name, AppWindow::set_login_user, AppWindow::set_login_password] {
                    set(&ui, "".into());
                }
                ui.set_login_url("https://".into());
                self.choose_provider();
            }
            Some(Act::Server(id)) => {
                let Some(p) = nori_host::profile_picked(&self.root.app.settings, &id) else { return };
                self.open(p, None);
            }
            Some(Act::Edit(id)) => self.edit_profile(id),
            Some(Act::Rename(id)) => {
                let prefs = self.root.app.settings.current().unwrap_or_default();
                let Some(p) = prefs.servers.iter().find(|s| s.id == id) else { return };
                let ui = self.ui();
                ui.set_profile_rename_name(p.name.as_str().into());
                ui.set_profile_rename_blank(words::profile_label(&SavedServer { name: String::new(), ..p.clone() }, self.kinds()).into());
                ui.set_profile_renaming(id.as_str().into());
            }
            Some(Act::Backup { restore }) => self.ask_field(words::backup_path(restore), name, false),
            Some(Act::BackupPassword { .. }) => {}
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
        let changed = self.heard != song.as_ref().map(|s| s.id.clone());
        self.heard = song.as_ref().map(|s| s.id.clone());
        self.song = song;
        if changed {
            self.ask_player_status();
            match self.song.clone() {
                Some(one) => self.ask_sleeve(one),
                None => {
                    self.sleeve = None;
                    self.moving = None;
                    ui.set_sleeved(false);
                }
            }
        }
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
        if changed {
            let now = self.session.as_ref().map_or(LyricsNow::NoSong, |s| s.lyrics_heard(self.heard.clone()));
            self.lyrics_now(now);
        }
    }

    /// Fetches lyrics only while they are on screen: the song's as they come in sight, and the core warms
    /// the songs ahead meanwhile.
    fn lyrics_seen(&mut self) {
        let ui = self.ui();
        let viewed = !self.hidden && (ui.get_full_player() && ui.get_full_panel() == Panel::Lyrics || ui.get_inspector() == Panel::Lyrics);
        if viewed == self.lyrics_viewed {
            return;
        }
        self.lyrics_viewed = viewed;
        if let Some(s) = &self.session {
            s.lyrics_view(viewed);
        }
    }

    /// The back button: the page this one was opened from, else the tab it was opened on; Home from a
    /// page of its own.
    fn back(&mut self) {
        if self.ui().get_view() == View::Settings && !self.plugged.stack.is_empty() {
            return self.plugin_act(crate::settings::PluginAct::Back);
        }
        if self.ui().get_view() != View::Page {
            self.trail.left();
            return self.go(View::Home);
        }
        match self.trail.back(self.ui().global::<crate::Scroll>().get_at()) {
            (Back::Page(page), at) => self.show_page(page, at),
            (Back::Tab(tab), at) => self.go_at(tab, at),
        }
    }

    /// The mouse's forward button: the page Back last left, scrolled where it was.
    fn forward(&mut self) {
        if let Some((page, at)) = self.trail.forward(self.ui().global::<crate::Scroll>().get_at()) {
            self.show_page(page, at);
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
        self.motion_follow();
        let Some(s) = &self.session else { return };
        let ui = self.ui();
        let now = s.now();
        // Copy the queue only when it changed.
        let queue = s.queue_stale(self.queue.as_ref()).then(|| s.queue_shown(self.queue.as_ref()));
        let playing = now.playing;
        ui.set_playing(playing);
        ui.set_mixing(now.mixing && now.state == State::Playing);
        if ui.get_view() == View::Page {
            self.hero_shown();
        }
        position_shown(&ui, self.player.as_ref(), now.position_now() as i32);
        self.lyrics_seen();
        if now.song.as_ref().map(|s| &s.id) != self.heard.as_ref() {
            self.song_shown(now.song.clone());
        }
        // Music started, or the player came to show the offered song: Home lets the offer go.
        if self.offer.shown().is_some() && !self.offer.still(self.heard.clone(), playing) {
            self.offer_shown();
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
                let s = q.songs.get(i as usize)?;
                let at = at as i32;
                Some(SongRow { by: q.by.get(&s.id).cloned().unwrap_or_default().into(), played: at < now, ..row(s, i as usize, at == now) })
            }).collect();
            self.queue_shown(rows);
            ui.set_queue_from(queue_from(q.upcoming().map(|(_, s)| s)).into());
            self.queue = Some(q);
        }
        // The seek bar steps only while playing.
        self.pace = now.pace;
        let ticks = playing && !self.hidden;
        if ticks && !self.tick.running() {
            self.seek_step();
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

/// `fresh` as shelf `old`'s newer answer: each card's move from its old place and whether it is new, so
/// the cards it kept slide to their places and the others fade in. A first fill moves nothing.
fn moved(old: Option<&Shelf>, fresh: Vec<Card>) -> Vec<Card> {
    let Some(old) = old.filter(|o| o.loaded && o.cards.row_count() > 0) else { return fresh };
    let was: HashMap<(i32, SharedString), usize> = old.cards.iter().enumerate().map(|(i, c)| ((c.kind, c.id), i)).collect();
    fresh
        .into_iter()
        .enumerate()
        .map(|(i, c)| match was.get(&(c.kind, c.id.clone())) {
            Some(&from) => Card { shift: from as i32 - i as i32, new: false, ..c },
            None => Card { shift: 0, new: true, ..c },
        })
        .collect()
}

/// Puts `fresh` in `model` row by row, so the shelves already drawn stay and their cards can move.
fn put_shelves(model: &VecModel<Shelf>, fresh: Vec<Shelf>) {
    let n = fresh.len();
    for (i, shelf) in fresh.into_iter().enumerate() {
        if i < model.row_count() { model.set_row_data(i, shelf) } else { model.push(shelf) }
    }
    while model.row_count() > n {
        model.remove(model.row_count() - 1);
    }
}

fn cards(it: impl Iterator<Item = Card>) -> ModelRc<Card> {
    ModelRc::new(VecModel::from(it.collect::<Vec<_>>()))
}

fn album_card(a: &nori_core::Album) -> Card {
    let sub_artist = a.artist_id.clone().unwrap_or_default().into();
    Card { id: a.id.as_str().into(), title: a.name.as_str().into(), sub: nori_core::lines::album_subtitle(&a.artist, a.year).into(), art: a.cover_art.clone().unwrap_or_default().into(), sub_artist, kind: 0, ..Default::default() }
}

/// The songs card `id` of `Card::kind` (or 4, a mix) stands for, fetched whole; a song card's are at hand.
fn card_fetch(kind: i32, id: String) -> Option<Fetch> {
    match kind {
        0 => Some(Fetch::Album(id)),
        1 => Some(Fetch::Artist(id)),
        2 => Some(Fetch::Playlist(id)),
        4 => Some(Fetch::Mix(id)),
        _ => None,
    }
}

/// The search field's preview `lines` of `found` as the main window draws them: a group's head as kind -1,
/// every result as kind 5, the rest as their cards.
fn preview_cards(lines: &[nori_core::search::PreviewLine], found: &nori_core::SearchResult) -> Vec<Card> {
    use nori_core::search::PreviewLine;
    let words_card = |kind: i32, title: &str| Card { id: SharedString::default(), title: title.into(), sub: SharedString::default(), art: SharedString::default(), sub_artist: SharedString::default(), kind, ..Default::default() };
    lines
        .iter()
        .map(|l| match *l {
            PreviewLine::Head { kind } => words_card(-1, words::preview_group(kind)),
            PreviewLine::Artist { at } => artist_card(&found.artists[at as usize]),
            PreviewLine::Album { at } => album_card(&found.albums[at as usize]),
            PreviewLine::Playlist { at } => playlist_card(&found.playlists[at as usize]),
            PreviewLine::Song { at } => song_card(&found.songs[at as usize]),
            PreviewLine::All => words_card(5, words::ALL_RESULTS),
        })
        .collect()
}

/// The preview's line `by` lines on from `at` (-1 for none), past the groups' heads and round its ends.
fn preview_step(lines: &[nori_core::search::PreviewLine], at: i32, by: i32) -> i32 {
    let stops: Vec<i32> = lines.iter().enumerate().filter(|(_, l)| !matches!(l, nori_core::search::PreviewLine::Head { .. })).map(|(i, _)| i as i32).collect();
    if stops.is_empty() {
        return -1;
    }
    let n = stops.len() as i32;
    let from = stops.iter().position(|&s| s == at).map(|k| k as i32).unwrap_or(if by > 0 { -1 } else { 0 });
    stops[(from + by).rem_euclid(n) as usize]
}

fn artist_card(a: &nori_core::Artist) -> Card {
    Card { id: a.id.as_str().into(), title: a.name.as_str().into(), sub: a.album_count.filter(|&n| n > 0).map(words::albums).unwrap_or_default().into(), art: a.cover_art.clone().unwrap_or_default().into(), sub_artist: SharedString::default(), kind: 1, ..Default::default() }
}

/// A playlist's card: its song count, or its own line (a provider's, who made it) when not counted.
fn playlist_card(p: &nori_core::Playlist) -> Card {
    let sub = match &p.comment {
        Some(line) if p.song_count == 0 => line.clone(),
        _ => words::songs(p.song_count as usize),
    };
    Card { id: p.id.as_str().into(), title: p.name.as_str().into(), sub: sub.into(), art: p.cover_art.clone().unwrap_or_default().into(), sub_artist: SharedString::default(), kind: 2, ..Default::default() }
}

/// A song's card on a feed's shelf: played from it, with the shelf's songs after.
fn song_card(s: &Song) -> Card {
    let sub_artist = s.artist_id.clone().unwrap_or_default().into();
    Card { id: s.id.as_str().into(), title: s.title.as_str().into(), sub: s.artist.as_str().into(), art: s.cover_art.clone().unwrap_or_default().into(), sub_artist, kind: 3, ..Default::default() }
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
        mark: 0,
        sealed: false,
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

/// Whether a devices panel shows: the full player's, or the side sheet's beside the pages.
fn devices_open(ui: &AppWindow) -> bool {
    if ui.get_full_player() { ui.get_full_panel() == Panel::Devices } else { ui.get_inspector() == Panel::Devices }
}

fn devices_set(ui: &AppWindow, rows: Vec<crate::DeviceRow>) {
    show_in(ui.get_devices(), rows, |m| ui.set_devices(m));
}

/// Shows `rows` in a list whose model is `shown`: in place where it is a [`VecModel`], so the rows drawn
/// stay (and a menu open on one stays open), else as a new model through `set`.
fn show_in<T: Clone + PartialEq + 'static>(shown: ModelRc<T>, rows: Vec<T>, set: impl FnOnce(ModelRc<T>)) {
    match shown.as_any().downcast_ref::<VecModel<T>>() {
        Some(m) => renew(m, rows),
        None => set(ModelRc::new(VecModel::from(rows))),
    }
}

/// What the `code`th code sign-in says, as the app's messages; signed in, the session finishes it as any
/// sign-in.
struct CodeHeard {
    session: nori_host::session::Handle,
    tx: Tx,
    code: u32,
}

impl nori_core::providers::code::CodeHeard for CodeHeard {
    fn heard(&self, event: SignInEvent) {
        self.tx.send(match event {
            SignInEvent::Answered { result } => {
                self.session.signed_in(&result);
                Msg::SignedIn(result)
            }
            event => Msg::CodeStep { code: self.code, event },
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test plugin's song action is on the songs' menus alone, and its page's rows on the devices panel.
    #[test]
    fn a_plugins_menu_actions_and_device_rows_come_from_its_manifest() {
        let dir = nori_testdir::TempDir::new("desktop-plugin-slots");
        let host = nori_plugins::open(dir.path().join("plugins"), nori_core::settings_store::Settings::new(), Arc::new(nori_host::desktop::Watched::default()));
        let id = host.install_from(nori_plugins::testing::fixture("").to_string_lossy().into_owned()).unwrap();
        host.turn(id, true);
        let labels: Vec<Vec<String>> = MENU_KINDS.iter().map(|k| host.menu_entries(*k, "en".into()).into_iter().map(|e| e.label).collect()).collect();
        assert_eq!(labels, [vec![], vec![], vec![], vec!["Count letters".to_string()]]);
        let rows = crate::settings::device_rows(&host.device_cards("en".into()), &[], &Default::default(), None, &Default::default(), &Default::default(), crate::look::Today { month: 10, day: 19, southern: false });
        let titles: Vec<String> = rows.iter().map(|r| r.title.to_string()).collect();
        assert_eq!(titles, ["TEST", "Gain: 0.5"]);
    }

    fn dressed() -> crate::look::Look {
        crate::look::plain(&Default::default(), &nori_core::settings::StoredPrefs::default(), true, crate::look::Today { month: 10, day: 19, southern: false })
    }

    /// A window in the theme's colors, as the app paints it.
    fn window() -> AppWindow {
        let ui = AppWindow::new().unwrap();
        dressed().paint(&ui);
        ui
    }

    #[test]
    fn back_retraces_pages_then_the_tab() {
        let (artist, album) = (PageAsk::Artist("ar".into()), PageAsk::Album("al".into()));
        let mut trail = PageTrail::default();
        trail.opened(artist.clone(), View::Favorites, 300.0);
        trail.opened(album.clone(), View::Page, 120.0);
        assert_eq!(trail.back(40.0), (Back::Page(artist.clone()), 120.0), "the album opened from the artist goes back to it, scrolled as it was");
        assert_eq!(trail.back(0.0), (Back::Tab(View::Favorites), 300.0), "then where the artist was opened from");
        assert_eq!(trail.forward(10.0), Some((artist, 0.0)), "forward retraces the pages back left");
        assert_eq!(trail.forward(0.0), Some((album, 40.0)));
        assert_eq!(trail.forward(0.0), None);
        trail.opened(PageAsk::Album("x".into()), View::Home, 0.0);
        assert_eq!(trail.back(0.0).0, Back::Tab(View::Home), "a page opened from a tab forgets the pages before");
        trail.left();
        assert_eq!(trail.forward(0.0), None, "a tab chosen forgets the pages ahead");
        trail.opened(PageAsk::Artist("ar".into()), opened_from(View::Search), 0.0);
        assert_eq!(trail.back(0.0).0, Back::Tab(View::Search), "an artist found by search goes back to the search");
    }

    #[test]
    fn a_long_continue_title_moves_nothing_under_it() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::LogicalSize::new(1280.0, 820.0));
        ui.set_view(View::Home);
        ui.set_picks_loaded(true);
        ui.set_offer_shown(true);
        ui.set_offer_line("The Weeknd · 9 of 22".into());
        ui.show().unwrap();
        // The rows from the card's line down, under where a two-line title ends.
        let under_title = |title: &str| {
            ui.set_offer_title(title.into());
            ui.window().take_snapshot().unwrap();
            slint::platform::update_timers_and_animations();
            let shot = ui.window().take_snapshot().unwrap();
            shot.as_bytes()[245 * 1280 * 4..].to_vec()
        };
        let two_lines = under_title("Reflections Laughing (feat. Florence + The Machine)");
        let too_long = under_title("Reflections Laughing (feat. Florence + The Machine, Travis Scott) and a good deal more words than fit");
        assert!(two_lines == too_long, "the card's line and button moved");
    }

    #[test]
    fn back_returns_to_where_home_was_scrolled() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::LogicalSize::new(1280.0, 820.0));
        let scroll = ui.global::<crate::Scroll>();
        scroll.set_want(600.0);
        scroll.set_asked(1);
        ui.set_view(View::Home);
        ui.set_picks_loaded(true);
        ui.show().unwrap();
        ui.window().take_snapshot().unwrap();
        slint::platform::update_timers_and_animations();
        assert_eq!(scroll.get_at(), 0.0, "an empty Home cannot be scrolled that far yet");
        ui.set_shelves(ModelRc::new(VecModel::from((0..12).map(|_| Shelf {
            loaded: true, cards: cards((0..8).map(|i| Card { id: i.to_string().into(), ..Default::default() })), ..Default::default()
        }).collect::<Vec<_>>())));
        ui.window().take_snapshot().unwrap();
        slint::platform::update_timers_and_animations();
        ui.window().take_snapshot().unwrap();
        slint::platform::update_timers_and_animations();
        assert_eq!(scroll.get_at(), 600.0, "once its shelves come, Home is where it was left");
    }

    fn pill() -> crate::PlayerBar {
        let bar = crate::PlayerBar::new().unwrap();
        dressed().paint(&bar);
        bar
    }

    #[test]
    fn an_edited_profile_keeps_its_id_and_password() {
        let info = nori_core::providers::Kinds::builtin().providers().into_iter().find(|i| i.kind == nori_core::settings::ServerKind::Subsonic).unwrap();
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
        let fresh = || {
            let mut pages = Paged::new(0);
            pages.ask();
            (Vec::new(), pages)
        };
        let (mut list, mut pages) = fresh();
        assert_eq!(albums_paged(&mut list, &mut pages, AlbumSort::ByName, false, 0, &page(0, full)), Some(full as u32));
        assert_eq!(albums_paged(&mut list, &mut pages, AlbumSort::ByName, false, 0, &page(0, full)), None, "the server's copy of the first page");
        assert_eq!(albums_paged(&mut list, &mut pages, AlbumSort::ByName, false, full as u32, &page(full, 100)), None, "the last page");
        assert_eq!(list.len(), full + 100);
        assert_eq!(list.last().map(|c| c.id.to_string()), Some((full + 99).to_string()));
        let (mut list, mut pages) = fresh();
        albums_paged(&mut list, &mut pages, AlbumSort::ByName, false, 0, &page(0, full));
        assert_eq!(albums_paged(&mut list, &mut pages, AlbumSort::ByName, false, full as u32, &page(0, full)), None, "a server that ignores the offset");
        assert_eq!(list.len(), full);
        assert!(!pages.more());
        let (mut list, mut pages) = fresh();
        albums_paged(&mut list, &mut pages, AlbumSort::ByName, false, 0, &page(0, full));
        pages.failed(&(full as u32));
        assert_eq!(pages.ask(), Some(full as u32), "a page that failed is asked again");
        let (mut list, mut pages) = fresh();
        assert_eq!(albums_paged(&mut list, &mut pages, AlbumSort::Random, false, 0, &page(0, full)), None, "a random order is one page");
        let (mut list, mut pages) = fresh();
        assert_eq!(albums_paged(&mut list, &mut pages, AlbumSort::ByName, true, 0, &page(0, full)), None, "the favorites come whole");
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
    fn a_cover_with_no_ticket_is_asked_again_when_drawn() {
        let mut art = Art::default();
        let now = Instant::now();
        let key = CoverKey { id: "a".into(), size: CoverSize::Card };
        art.asked.insert(key.clone());
        art.wanted.push(key.clone());
        art.missing.insert(key.clone(), now);
        art.ask(now, |_| None);
        assert!(!art.asked.contains(&key), "not fetched, so the next draw asks for it again");
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
    fn new_tiles_keep_the_mosaics_still_shown() {
        let mut art = Art::default();
        let image = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(4, 4));
        let covers = |c: &str| (0..4).map(|i| format!("{c}{i}")).collect::<Vec<_>>();
        for c in ["a", "b"] {
            art.mosaics.insert(covers(c), Mosaic::Ready(image.clone()));
        }
        let tile = |c: &str| nori_core::mixes::board::MixTile { id: c.into(), name: nori_core::mixes::board::MixName::Discover, covers: covers(c), favourites: false };
        keep_shown_mosaics(&mut art, &[tile("a"), tile("c")]);
        assert!(art.mosaics.contains_key(&covers("a")));
        assert!(!art.mosaics.contains_key(&covers("b")));
    }

    #[test]
    fn lyrics_are_sharp_only_at_the_line_aimed_at_from_the_intro_on() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_full_player(true);
        ui.set_full_panel(Panel::Lyrics);
        ui.set_lyrics_synced(true);
        ui.set_lyrics_lines(ModelRc::new(VecModel::from((0..6).map(|i| crate::SungLine { text: format!("line {i}").into(), ..Default::default() }).collect::<Vec<_>>())));
        ui.show().unwrap();
        let band = |active: i32| {
            ui.set_lyrics_active(active);
            for _ in 0..2 {
                ui.window().take_snapshot().unwrap();
                i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
                slint::platform::update_timers_and_animations();
            }
            lyrics_focus(&ui).map(|f| (f.band_top, f.band_h))
        };
        let sung = |active| band(active).unwrap_or_else(|| panic!("line {active} sung: the others are blurred"));
        let intro = band(-1).expect("before the first line, the lines are blurred");
        let first = sung(0);
        assert_eq!(intro, first, "through the intro the first line, where it will be sung, is the sharp one");
        assert_eq!(sung(2), first, "each line sung is scrolled to the same place");
        assert_eq!(band(6), Some(first), "once all are over, the last line is aimed at and the rest stay blurred");
        ui.set_details_open(true);
        assert_eq!(band(2), Some(first), "under a dialog the lyrics stay in focus");
        let raised = lyrics_focus(&ui).unwrap().raised;
        // Details' pane: 440 px wide, centered, 80 px down, with its 30 px corners.
        assert!(matches!(raised[..], [Raised { rect: [420.0, 80.0, 440.0, h], radius: 30.0 }] if h > 0.0), "the dialog's pane stays sharp: {raised:?}");
        ui.set_details_open(false);
        assert_eq!(lyrics_focus(&ui).unwrap().raised, [], "a closed dialog leaves nothing sharp");
    }

    #[test]
    fn lyric_rows_are_drawn_as_broken() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_lyrics_synced(true);
        ui.set_inspector(Panel::Lyrics);
        ui.show().unwrap();
        // How many pixel rows of the panel a lyric line takes: drawn, against the line with no words.
        let tall = |text: &str| {
            let shot = |text: &str| {
                ui.set_lyrics_lines(ModelRc::new(VecModel::from(vec![crate::SungLine { text: text.into(), ..Default::default() }])));
                ui.set_lyrics_side_rows(ModelRc::new(VecModel::from(vec![SharedString::from(text)])));
                i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
                slint::platform::update_timers_and_animations();
                ui.window().take_snapshot().unwrap()
            };
            let (bare, drawn) = (shot("."), shot(text));
            let w = bare.width() as usize;
            let rows: Vec<usize> = (0..bare.height() as usize)
                .filter(|y| (w - 300..w).any(|x| bare.as_slice()[y * w + x] != drawn.as_slice()[y * w + x]))
                .collect();
            rows.last().unwrap() - rows.first().unwrap()
        };
        // A row wider than the panel, as one measured a hair differently from the window can be.
        let long = tall("every word of this row stays on it however wide it runs");
        let short = tall("every word");
        assert!(short > 10 && long <= short + 4, "one row each, never wrapped again: {long} px against {short} px");
    }

    #[test]
    fn resizing_does_not_animate_layout() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        for (view, full, panel) in [(View::Home, false, Panel::None), (View::Albums, false, Panel::Queue), (View::Home, true, Panel::Queue), (View::Home, true, Panel::Lyrics)] {
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
                assert_eq!(changed, 0, "resize {width}x{height}, view {view:?}, full {full}, panel {panel:?} must be laid out immediately");
            }
            if full {
                continue;
            }
            ui.set_inspector(if panel == Panel::None { Panel::Queue } else { Panel::None });
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
            ui.set_view(View::Page);
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

    /// A song's menu does what its entries say: the seal beside the title stars the song and the menu stays
    /// open on it while the row takes its seal, the album's line opens it, Play plays it, and Details beside
    /// the seal is told by its kind.
    #[test]
    fn a_songs_menu_entries_act() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_view(View::Page);
        let sealed = Rc::new(std::cell::Cell::new(false));
        let menu = {
            let sealed = sealed.clone();
            move || {
                vec![
                    crate::SongGo { kind: 1, title: "Go to album".into(), id: "al".into(), place: SongPlace::Names, enabled: true, ..Default::default() },
                    crate::SongGo { kind: 0, title: "Favorite".into(), place: SongPlace::Title, enabled: true, filled: sealed.get(), ..Default::default() },
                    crate::SongGo { kind: 14, title: "Details".into(), place: SongPlace::Title, enabled: true, ..Default::default() },
                ]
            }
        };
        ui.on_song_menu(move |_, _| ModelRc::new(VecModel::from(menu())));
        let row = |sealed| vec![SongRow { title: "Glass Harbour".into(), sealed, ..Default::default() }];
        ui.set_page_songs(ModelRc::new(VecModel::from(row(false))));
        let did = Rc::new(RefCell::new(Vec::new()));
        let seen = did.clone();
        let weak = ui.as_weak();
        // As the app does: the seal flips and the rows are shown again.
        ui.on_song(move |list, i, how| {
            seen.borrow_mut().push(format!("song {list} {i} {how}"));
            if how == 0 {
                sealed.set(!sealed.get());
                let ui = weak.unwrap();
                show_in(ui.get_page_songs(), row(sealed.get()), |m| ui.set_page_songs(m));
            }
        });
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
        // The menu opens where the row is pressed: the song at its head with its two glyphs at the end, then
        // Play, and the album's line a group below.
        let row = (400.0, 470.0);
        // Its 6 px inset, the song's head (46 px, the glyphs 32 px at its end), its hairline, 32 px rows, and
        // 7 px between groups.
        let glyph = |n: f32| (row.0 + 260.0 - 6.0 - (2.0 - n) * 32.0 + 16.0, row.1 + 6.0 + 23.0);
        let play = row.1 + 6.0 + 53.0 + 16.0;
        let album = play + 32.0 + 7.0;
        press(row.0, row.1, Right);
        press(glyph(0.0).0, glyph(0.0).1, Left);
        press(glyph(0.0).0, glyph(0.0).1, Left);
        press(row.0 + 40.0, play, Left);
        assert_eq!(*did.borrow(), ["song 0 0 0", "song 0 0 0", "song 0 0 -1"], "the menu stays open on the seal");
        did.borrow_mut().clear();
        press(row.0, row.1, Right);
        press(row.0 + 40.0, album, Left);
        press(row.0, row.1, Right);
        press(glyph(1.0).0, glyph(1.0).1, Left);
        assert_eq!(*did.borrow(), ["album al", "song 0 0 14"]);
    }

    /// A song menu's head keeps its glyphs a 32 px column at its end, each centered on the head, whatever the
    /// song, its lines and where the menu opens.
    #[test]
    fn a_songs_menu_head_keeps_its_glyphs_at_the_end() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let seal = |filled| crate::SongGo { kind: 0, title: "Favorite".into(), place: SongPlace::Title, enabled: true, filled, ..Default::default() };
        let details = crate::SongGo { kind: 14, title: "Details".into(), place: SongPlace::Title, enabled: true, ..Default::default() };
        let album = crate::SongGo { kind: 1, title: "Go to album".into(), id: "al".into(), place: SongPlace::Names, enabled: true, ..Default::default() };
        let artist = crate::SongGo { kind: 2, title: "Go to artist".into(), id: "ar".into(), place: SongPlace::Names, enabled: true, ..Default::default() };
        let long = "Reflections Laughing (feat. Florence + The Machine, Travis Scott) and a good deal more";
        // The song, its line under the title, the menu's lines, where the row is pressed, the window's height.
        let cases = [
            ("Glass Harbour", ("Mira", "Tides"), vec![album.clone(), seal(false), details.clone()], 400.0, 820),
            ("Glass Harbour", ("", ""), vec![seal(true), details.clone()], 400.0, 820),
            (long, ("Mira", "Tides"), vec![album.clone(), artist.clone(), seal(true), details.clone()], 400.0, 820),
            ("Up", ("Mira", ""), vec![album.clone(), seal(false)], 400.0, 820),
            (long, ("", ""), vec![artist.clone(), details.clone()], 400.0, 820),
            ("Glass Harbour", ("Mira", "Tides"), vec![album.clone(), seal(false), details.clone()], 1180.0, 820),
            (long, ("Mira", "Tides"), vec![seal(true), details.clone(), album.clone()], 1180.0, 560),
        ];
        let mut off = Vec::new();
        for (case, (title, (by, on), lines, x, height)) in cases.into_iter().enumerate() {
            let ui = window();
            ui.window().set_size(slint::PhysicalSize::new(1280, height));
            ui.set_view(View::Page);
            let glyphs = lines.iter().filter(|m| m.place == SongPlace::Title).count();
            ui.on_song_menu(move |_, _| ModelRc::new(VecModel::from(lines.clone())));
            let row = SongRow { title: title.into(), artist: by.into(), album: on.into(), ..Default::default() };
            ui.set_page_songs(ModelRc::new(VecModel::from(vec![row])));
            ui.show().unwrap();
            let settle = || {
                i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
                slint::platform::update_timers_and_animations();
                ui.window().take_snapshot().unwrap()
            };
            settle();
            let point = |x: f32, y: f32| slint::LogicalPosition::new(x, y);
            let button = slint::platform::PointerEventButton::Right;
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position: point(x, 470.0), button });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: point(x, 470.0), button });
            settle();
            let (left, top) = {
                let inner = i_slint_core::window::WindowInner::from_pub(ui.window());
                let popups = inner.active_popups();
                let i_slint_core::window::PopupWindowLocation::ChildWindow(at) = popups[0].location else { panic!("the menu is in the window") };
                (at.x, at.y)
            };
            // The head's 40 px within its 6 px inset; the glyphs end at the inset.
            let centre = top + 6.0 + 20.0;
            for n in 0..glyphs {
                let box_left = left + 260.0 - 6.0 - (glyphs - n) as f32 * 32.0;
                let mid = (box_left + 16.0, centre);
                ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: point(mid.0, mid.1) });
                let shot = settle();
                let w = shot.width() as usize;
                let px = |x: isize, y: isize| shot.as_slice()[y as usize * w + x as usize];
                let panel = px((left + 260.0 - 3.0) as isize, centre as isize);
                // From the middle out, as far as the hovered glyph's plate reaches.
                let reach = |dx: isize, dy: isize| {
                    let (mut x, mut y) = (mid.0 as isize, mid.1 as isize);
                    while px(x + dx, y + dy) != panel { x += dx; y += dy; }
                    (x as f32, y as f32)
                };
                let plate = [reach(-1, 0).0, reach(1, 0).0, reach(0, -1).1, reach(0, 1).1];
                let want = [box_left, box_left + 31.0, centre - 16.0, centre + 15.0];
                if plate.iter().zip(want).any(|(a, b)| (a - b).abs() > 1.0) {
                    off.push(format!("case {case}, glyph {n}: its plate spans {plate:?}, not {want:?}"));
                }
            }
        }
        assert!(off.is_empty(), "{off:#?}");
    }

    /// The player's menu opens above the pill with the lines it is given, and a line reaches the app by
    /// its kind.
    #[test]
    fn a_right_press_on_the_full_players_cover_opens_the_songs_menu() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_has_song(true);
        ui.set_now_title("Song".into());
        ui.set_full_player(true);
        let asked = Rc::new(std::cell::Cell::new(0));
        let seen = asked.clone();
        ui.on_player_menu(move || { seen.set(seen.get() + 1); ModelRc::default() });
        ui.show().unwrap();
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        ui.window().take_snapshot().unwrap();
        // On the cover, in the middle of the window's left half.
        use slint::platform::PointerEventButton::{Left, Right};
        let at = slint::LogicalPosition::new(320.0, 300.0);
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position: at, button: Left });
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: at, button: Left });
        assert_eq!(asked.get(), 0, "a click on the cover opens no menu");
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position: at, button: Right });
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: at, button: Right });
        assert_eq!(asked.get(), 1);
    }

    /// A profile row's circle opens its icon picker where the profile may be changed, and the icon picked
    /// reaches the app by its place; a profile without Edit opens none.
    #[test]
    fn a_profiles_circle_picks_its_icon() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_view(View::Settings);
        let profile = |options: &[&str]| {
            let options: Vec<slint::SharedString> = options.iter().map(|o| (*o).into()).collect();
            crate::SettingRow { kind: 8, name: "a".into(), title: "Home".into(), options: ModelRc::new(VecModel::from(options)), letter: "Z".into(), enabled: true, first: true, last: true, ..Default::default() }
        };
        let sv = nori_core::settings::SavedServer { user: "zoe".into(), ..Default::default() };
        let picker = crate::settings::icon_picker(&crate::settings::Plugged::default(), &sv, "Home", "OpenSubsonic");
        let did = Rc::new(RefCell::new(Vec::new()));
        let icons = ui.global::<crate::ProfileIcons>();
        let seen = did.clone();
        icons.on_picker(move |id| {
            seen.borrow_mut().push(format!("picker {id}"));
            picker.clone()
        });
        let seen = did.clone();
        icons.on_picked(move |id, at| seen.borrow_mut().push(format!("picked {id} {at}")));
        ui.show().unwrap();
        let settle = || {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
        };
        let click = |at: slint::LogicalPosition| {
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position: at, button: slint::platform::PointerEventButton::Left });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: at, button: slint::platform::PointerEventButton::Left });
            settle();
        };
        let middle = |e: &i_slint_backend_testing::ElementHandle| {
            let (at, size) = (e.absolute_position(), e.size());
            slint::LogicalPosition::new(at.x + size.width / 2.0, at.y + size.height / 2.0)
        };
        let circle = || i_slint_backend_testing::ElementHandle::find_by_element_type_name(&ui, "ProfileCircle").next().expect("the profile's circle");
        for (options, want) in [(&[][..], Vec::<String>::new()), (&["Rename"][..], vec!["picker a".into(), "picked a 2".into()])] {
            did.borrow_mut().clear();
            ui.set_settings(ModelRc::new(VecModel::from(vec![profile(options)])));
            settle();
            click(middle(&circle()));
            // A popup's elements give where they are in it, not in the window: the choice is picked as a
            // screen reader would.
            if let Some(singing) = i_slint_backend_testing::ElementHandle::find_by_accessible_label(&ui, "Singing").next() {
                singing.invoke_accessible_default_action();
                settle();
            }
            assert_eq!(*did.borrow(), want, "with {options:?}");
        }
    }

    /// The main window and its player pill as the app joins them, the pill at x 140 and 1000 wide.
    fn window_and_pill() -> (AppWindow, crate::PlayerBar) {
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        let chrome = ui.global::<crate::Chrome>();
        chrome.set_window_width(1280.0);
        chrome.set_window_height(820.0);
        chrome.set_player_x(140.0);
        chrome.set_player_w(1000.0);
        ui.set_has_song(true);
        let bar = player(&ui, |_, _| Image::default()).unwrap();
        dressed().paint(&bar);
        bar.window().set_size(slint::PhysicalSize::new(1000, 64));
        bar.set_room(1000.0);
        bar.set_has_song(true);
        bar.set_now_title("Song".into());
        bar.set_now_artist("Artist".into());
        bar.set_now_artist_id("a1".into());
        ui.show().unwrap();
        bar.show().unwrap();
        (ui, bar)
    }

    fn press(window: &slint::Window, x: f32, y: f32, button: slint::platform::PointerEventButton) {
        let at = slint::LogicalPosition::new(x, y);
        window.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at });
        window.dispatch_event(slint::platform::WindowEvent::PointerPressed { position: at, button });
        window.dispatch_event(slint::platform::WindowEvent::PointerReleased { position: at, button });
    }

    #[test]
    fn a_right_press_on_the_pills_title_opens_the_songs_menu() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let (ui, bar) = window_and_pill();
        ui.on_player_menu(|| ModelRc::new(VecModel::from(vec![crate::SongGo { kind: 17, title: "Sleep timer".into(), enabled: true, ..Default::default() }])));
        let did = Rc::new(RefCell::new(Vec::new()));
        let seen = did.clone();
        ui.on_player_act(move |kind, _| seen.borrow_mut().push(kind));
        let settle = || {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
            bar.window().take_snapshot().unwrap();
        };
        settle();
        use slint::platform::PointerEventButton::{Left, Right};
        press(bar.window(), 75.0, 24.0, Right);
        settle();
        // The menu's one line, its foot on the pill's room, beside where the pill was pressed.
        press(ui.window(), 140.0 + 75.0 + 60.0, 820.0 - 96.0 - 6.0 - 16.0, Left);
        settle();
        assert_eq!(*did.borrow(), [17]);
    }

    #[test]
    fn a_right_press_on_an_artists_name_offers_to_play_the_artist() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let (ui, bar) = window_and_pill();
        let played = Rc::new(RefCell::new(Vec::new()));
        let seen = played.clone();
        ui.on_play_artist(move |id| seen.borrow_mut().push(id.to_string()));
        let settle = || {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
            bar.window().take_snapshot().unwrap();
        };
        settle();
        use slint::platform::PointerEventButton::{Left, Right};
        press(bar.window(), 75.0, 41.0, Right);
        settle();
        // Play, the first of its three lines over the pill: Play, Play next, Add to queue.
        press(ui.window(), 140.0 + 75.0 + 60.0, 820.0 - 96.0 - 6.0 - 16.0 - 2.0 * 32.0, Left);
        settle();
        assert_eq!(*played.borrow(), ["a1"]);
    }

    #[test]
    fn the_queue_follows_the_next_song_to_its_top() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_has_song(true);
        ui.set_inspector(Panel::Queue);
        let row = |i: i32, playing: i32| SongRow { title: format!("Song {i}").into(), index: i, playing: i == playing, played: i < playing, ..Default::default() };
        let queue = Rc::new(VecModel::from((0..40).map(|i| row(i, 0)).collect::<Vec<_>>()));
        ui.set_queue(ModelRc::from(queue.clone()));
        let jumped = Rc::new(RefCell::new(Vec::new()));
        let seen = jumped.clone();
        ui.on_jump(move |i| seen.borrow_mut().push(i));
        ui.show().unwrap();
        let settle = || {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
        };
        settle();
        // A double click plays the row under it.
        let play_at = || {
            use slint::platform::PointerEventButton::Left;
            press(ui.window(), 1100.0, 271.0, Left);
            press(ui.window(), 1100.0, 271.0, Left);
            settle();
        };
        play_at();
        for i in 0..40 {
            queue.set_row_data(i, row(i as i32, 10));
        }
        settle();
        play_at();
        let j = jumped.borrow();
        assert_eq!(j.len(), 2, "{j:?}");
        assert_eq!(j[1], j[0] + 10, "{j:?}");
    }

    /// A queue row's right press opens the song's menu there: Play jumps to the row's song, and a line
    /// reaches the app as the queue's (list 3) by the row's list index.
    #[test]
    fn a_right_press_on_a_queue_row_opens_its_songs_menu() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_has_song(true);
        ui.set_inspector(Panel::Queue);
        let rows: Vec<SongRow> = (0..6).map(|i| SongRow { title: format!("Song {i}").into(), index: 100 + i, playing: i == 0, ..Default::default() }).collect();
        ui.set_queue(ModelRc::new(VecModel::from(rows)));
        let menu = vec![crate::SongGo { kind: 5, title: "Play next".into(), enabled: true, ..Default::default() }];
        let asked = Rc::new(RefCell::new(Vec::new()));
        let seen = asked.clone();
        ui.on_song_menu(move |list, i| {
            seen.borrow_mut().push(format!("menu {list} {i}"));
            ModelRc::new(VecModel::from(menu.clone()))
        });
        let seen = asked.clone();
        ui.on_song(move |list, i, how| seen.borrow_mut().push(format!("song {list} {i} {how}")));
        ui.show().unwrap();
        let settle = || {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
        };
        settle();
        use slint::platform::PointerEventButton::{Left, Right};
        let row = (1100.0, 271.0);
        // Its 6 px inset, the song's head (53 px with its hairline), then 32 px rows, the menu flipped left
        // of the pointer near the window's edge.
        let entry = |n: f32| row.1 + 6.0 + 53.0 + n * 32.0 + 16.0;
        let x = row.0 - 60.0;
        press(ui.window(), row.0, row.1, Right);
        settle();
        press(ui.window(), x, entry(1.0), Left);
        settle();
        press(ui.window(), row.0, row.1, Right);
        settle();
        press(ui.window(), x, entry(0.0), Left);
        settle();
        let asked = asked.borrow();
        assert_eq!(asked.len(), 4, "{asked:?}");
        let i: i32 = asked[0].trim_start_matches("menu 3 ").parse().unwrap_or_else(|_| panic!("the queue's menu: {asked:?}"));
        assert!((100..106).contains(&i), "{asked:?}");
        assert_eq!(asked[1..], [format!("song 3 {i} 5"), format!("menu 3 {i}"), format!("song 3 {i} -1")]);
    }

    /// The search preview's lines open their menus on a right press: an album's plays next by its id, a
    /// song's acts as the preview's (list 4) by its line.
    #[test]
    fn a_right_press_on_the_search_preview_opens_its_menus() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.global::<crate::Chrome>().set_top_x(190.0);
        ui.global::<crate::Chrome>().set_top_w(900.0);
        // The field as the top pill reports it: 360 px, ending 84 px in from the pill's end.
        ui.global::<crate::Chrome>().set_search_end(84.0);
        ui.global::<crate::Chrome>().set_search_w(360.0);
        let card = |kind: i32, id: &str| Card { id: id.into(), title: id.into(), kind, ..Default::default() };
        ui.set_search_preview(ModelRc::new(VecModel::from(vec![card(-1, "Albums"), card(0, "al1"), card(3, "s1")])));
        ui.set_search_preview_open(true);
        let did = Rc::new(RefCell::new(Vec::new()));
        let seen = did.clone();
        ui.on_queue_card(move |kind, id, next| seen.borrow_mut().push(format!("card {kind} {id} {next}")));
        let plugins = ui.global::<crate::PluginMenus>();
        plugins.set_album(ModelRc::new(VecModel::from(vec![SharedString::from("Count tracks")])));
        let seen = did.clone();
        plugins.on_run(move |kind, i, id, title| seen.borrow_mut().push(format!("plugin {kind} {i} {id} {title}")));
        let menu = vec![crate::SongGo { kind: 6, title: "Add to queue".into(), enabled: true, ..Default::default() }];
        ui.on_song_menu(move |_, _| ModelRc::new(VecModel::from(menu.clone())));
        let seen = did.clone();
        ui.on_song(move |list, i, how| seen.borrow_mut().push(format!("song {list} {i} {how}")));
        ui.show().unwrap();
        let settle = || {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
        };
        settle();
        use slint::platform::PointerEventButton::{Left, Right};
        // The preview's lines under its 6 px inset, 64 px down at the top pill's right: the 24 px head,
        // then 48 px lines.
        let (x, album, song) = (864.0, 64.0 + 6.0 + 24.0 + 24.0, 64.0 + 6.0 + 24.0 + 48.0 + 24.0);
        let entry = |y: f32, n: f32| y + 6.0 + 53.0 + n * 32.0 + 16.0;
        press(ui.window(), x, album, Right);
        settle();
        press(ui.window(), x + 60.0, entry(album, 1.0), Left);
        settle();
        // A plugin's action follows the album's own lines.
        press(ui.window(), x, album, Right);
        settle();
        press(ui.window(), x + 60.0, entry(album, 3.0), Left);
        settle();
        press(ui.window(), x, song, Right);
        settle();
        press(ui.window(), x + 60.0, entry(song, 1.0), Left);
        settle();
        assert_eq!(*did.borrow(), ["card 0 al1 true", "plugin 0 0 al1 al1", "song 4 2 6"]);
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
        ui.set_inspector(Panel::Queue);
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
        ui.set_view(View::Settings);
        ui.set_settings(crate::settings::rows(&Default::default(), &nori_core::settings::StoredPrefs::default(), &facts, &nori_core::providers::Kinds::builtin(), &Default::default(), tab, crate::look::Today { month: 10, day: 19, southern: false }));
        let moved = Rc::new(RefCell::new(Vec::new()));
        let seen = moved.clone();
        ui.on_row_placed(move |kind, id, by| seen.borrow_mut().push((kind, id.to_string(), by)));
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
        assert_eq!(*moved.borrow(), [(6, "LYRICS_PLUS".to_string(), 2)]);
    }

    /// A profile dragged by its handle above the one before it moves one place up, once let go; a jam
    /// guest's profiles have no handle.
    #[test]
    fn a_profile_is_dragged_to_its_place() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.global::<crate::Chrome>().set_top_x(190.0);
        ui.global::<crate::Chrome>().set_top_w(900.0);
        ui.set_view(View::Settings);
        let sv = |id: &str| nori_core::settings::SavedServer { id: id.into(), name: id.into(), user: "ann".into(), ..Default::default() };
        let prefs = nori_core::settings::StoredPrefs { servers: vec![sv("a"), sv("b"), sv("c")], active_server_id: "a".into(), ..Default::default() };
        let rows = |guest: bool| {
            let facts = crate::settings::Facts { rules: nori_core::browse::profile_rules(guest), ..Default::default() };
            let tab = crate::settings::tabs(&facts.rules.settings).iter().position(|p| *p == nori_core::settings_pages::SettingsPage::Servers).unwrap() as i32;
            crate::settings::rows(&Default::default(), &prefs, &facts, &nori_core::providers::Kinds::builtin(), &Default::default(), tab, crate::look::Today { month: 10, day: 19, southern: false })
        };
        let moved = Rc::new(RefCell::new(Vec::new()));
        let seen = moved.clone();
        ui.on_row_placed(move |kind, id, by| seen.borrow_mut().push((kind, id.to_string(), by)));
        ui.show().unwrap();
        let pass = |ms: u64| {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(ms));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
        };
        use slint::platform::{PointerEventButton::Left, WindowEvent::*};
        // Up the handles' column from the foot of the page: each press dragged 70 px up, until one moves a row.
        let drag = || {
            let at = |y: f32| slint::LogicalPosition::new(978.0, y);
            for y in (100..=780).rev().step_by(6) {
                let y = y as f32;
                ui.window().dispatch_event(PointerMoved { position: at(y) });
                ui.window().dispatch_event(PointerPressed { position: at(y), button: Left });
                ui.window().dispatch_event(PointerMoved { position: at(y - 70.0) });
                pass(16);
                ui.window().dispatch_event(PointerReleased { position: at(y - 70.0), button: Left });
                pass(16);
                if !moved.borrow().is_empty() {
                    break;
                }
            }
        };
        ui.set_settings(rows(false));
        pass(1000);
        drag();
        assert_eq!(moved.borrow().first().map(|m| (m.0, m.2)), Some((8, -1)), "{:?}", moved.borrow());
        moved.borrow_mut().clear();
        ui.set_settings(rows(true));
        pass(1000);
        drag();
        assert!(moved.borrow().is_empty(), "{:?}", moved.borrow());
    }

    /// A card opens on a click with jams on too, where a right press opens its menu.
    #[test]
    fn a_card_opens_on_a_click_with_jams_on() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = window();
        ui.window().set_size(slint::LogicalSize::new(1280.0, 820.0));
        ui.global::<crate::Jam>().set_on(true);
        ui.set_view(View::Albums);
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
        ui.set_view(View::Albums);
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
        ui.set_inspector(Panel::Queue);
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
        // Where each of the side sheet's tabs is laid out, left edge and width.
        let tabs = |panel: Panel| {
            ui.set_inspector(panel);
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            i_slint_backend_testing::ElementHandle::find_by_element_type_name(&ui, "PanelTabs")
                .flat_map(|t| t.query_descendants().match_inherits("TabButton").find_all())
                .map(|b| (b.absolute_position().x, b.size().width))
                .collect::<Vec<_>>()
        };
        let devices = tabs(Panel::Devices);
        assert!(devices.len() >= 3, "{devices:?}");
        for panel in [Panel::Lyrics, Panel::Queue] {
            assert_eq!(tabs(panel), devices, "panel {panel:?}");
        }
    }

    #[test]
    fn clear_shows_only_with_songs_after_the_one_playing() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.show().unwrap();
        ui.set_inspector(Panel::Queue);
        let row = |index: i32, played: bool, playing: bool| SongRow { index, title: format!("Song {index}").into(), played, playing, ..Default::default() };
        let clear = |rows: Vec<SongRow>| {
            ui.set_queue(ModelRc::new(VecModel::from(rows)));
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            i_slint_backend_testing::ElementHandle::find_by_element_type_name(&ui, "QueueView")
                .flat_map(|q| q.query_descendants().match_predicate(|e| e.accessible_label().is_some_and(|l| l == "Clear")).find_all())
                .count()
        };
        assert_eq!(clear(vec![row(0, true, false), row(1, false, true)]), 0, "nothing after the song playing");
        assert_eq!(clear(vec![row(0, true, false), row(1, false, true), row(2, false, false)]), 1);
    }

    #[test]
    fn a_sideways_scroll_leaves_the_lyrics_in_place() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        // A row wider than either view, as one measured a hair narrower than the window draws it.
        let row = "All that I want is a little bit of buh-duh-dabba-dabba-dabba";
        let line = crate::SungLine { text: row.into(), backing: SharedString::default(), right: false, background: false };
        ui.set_lyrics_lines(ModelRc::new(VecModel::from(vec![line])));
        let rows = || ModelRc::new(VecModel::from(vec![SharedString::from(row)]));
        ui.set_lyrics_side_rows(rows());
        ui.set_lyrics_full_rows(rows());
        ui.show().unwrap();
        let settle = || {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
        };
        ui.set_full_panel(Panel::Lyrics);
        for (view, inspector, full) in [("side", Panel::Lyrics, false), ("Now Playing", Panel::None, true)] {
            ui.set_inspector(inspector);
            ui.set_full_player(full);
            settle();
            let text = || {
                let found: Vec<_> = i_slint_backend_testing::ElementHandle::find_by_accessible_label(&ui, row).map(|t| t.absolute_position()).collect();
                assert_eq!(found.len(), 1, "{view}");
                found[0]
            };
            let before = text();
            let position = slint::LogicalPosition::new(before.x + 40.0, before.y + 10.0);
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled { position, delta_x: -200.0, delta_y: 0.0 });
            settle();
            assert_eq!(text(), before, "{view}");
        }
    }

    #[test]
    fn the_pills_seal_matches_the_output_beside_it() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        for dark in [false, true] {
            let bar = crate::PlayerBar::new().unwrap();
            crate::look::plain(&Default::default(), &nori_core::settings::StoredPrefs::default(), dark, crate::look::Today { month: 10, day: 19, southern: false }).paint(&bar);
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
    fn text_without_a_colour_takes_the_themes_ink() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        for dark in [false, true] {
            let ui = AppWindow::new().unwrap();
            crate::look::plain(&Default::default(), &nori_core::settings::StoredPrefs::default(), dark, crate::look::Today { month: 10, day: 19, southern: false }).paint(&ui);
            let ink = ui.global::<crate::Palette>().get_foreground().color().to_argb_u8();
            let light = nori_look::color::luminance(u32::from_be_bytes([ink.alpha, ink.red, ink.green, ink.blue])) > 0.5;
            assert_eq!(light, dark, "dark {dark}: {ink:?}");
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
        let said = Rc::new(RefCell::new(Vec::<String>::new()));
        let (st, ac, es) = (said.clone(), said.clone(), said.clone());
        bar.on_search_step(move |by| st.borrow_mut().push(format!("step {by}")));
        bar.on_search_accepted(move || ac.borrow_mut().push("enter".into()));
        bar.on_search_escaped(move || es.borrow_mut().push("escape".into()));
        key(&SharedString::from(slint::platform::Key::DownArrow));
        key(&SharedString::from(slint::platform::Key::UpArrow));
        key("\n");
        assert_eq!(bar.get_search_text(), "ab", "the arrows walk the preview, not the text");
        key(&SharedString::from(slint::platform::Key::Escape));
        settle();
        assert_eq!(*said.borrow(), ["step 1", "step -1", "enter", "escape"]);
        assert!(!typing.get(), "Escape gives the keys back to the page");
    }

    /// While the search field is open, it takes the room the pill has: the settings and the profile's mark stay
    /// inside the pill at its right end, and the field does not run over the tabs, narrow or wide.
    #[test]
    fn the_top_pill_keeps_its_ends_inside_it_while_searching() {
        use i_slint_backend_testing::ElementHandle;
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let all = LibraryTabs { albums: true, artists: true, songs: true, playlists: true, favorites: true };
        let mut off = Vec::new();
        // An 800 px window's pill on macOS, up to the widest pill; the current tab is named even while the
        // others are not.
        for room in [616.0f32, 700.0, 879.0, 880.0, 960.0, 1040.0] {
            for (focused, previewing) in [(false, false), (true, false), (true, true)] {
                let bar = TopBar::new().unwrap();
                bar.window().set_size(slint::LogicalSize::new(room, 48.0));
                bar.set_room(room);
                bar.set_tabs(all.clone());
                bar.set_view(View::Favorites);
                bar.show().unwrap();
                if focused {
                    bar.invoke_focus_search();
                }
                bar.set_previewing(previewing);
                for _ in 0..3 {
                    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(300));
                    slint::platform::update_timers_and_animations();
                    bar.window().take_snapshot().unwrap();
                }
                let case = format!("room {room}, focused {focused}, previewing {previewing}");
                let span = |e: &ElementHandle| (e.absolute_position().x, e.absolute_position().x + e.size().width);
                let last_tab = ElementHandle::find_by_element_type_name(&bar, "TabButton").map(|e| span(&e).1).fold(0.0, f32::max);
                // The field: its words start 35 px in from its left edge, and it is as wide as the bar says.
                let input_left = span(&ElementHandle::find_by_element_type_name(&bar, "TextInput").next().unwrap()).0;
                let field = (input_left - 35.0, input_left - 35.0 + bar.get_search_width());
                if (room - field.1 - bar.get_search_end()).abs() > 0.5 {
                    off.push(format!("{case}: the field ends at {}, the bar says {}", field.1, room - bar.get_search_end()));
                }
                let settings = span(&ElementHandle::find_by_element_type_name(&bar, "IconButton").next().unwrap());
                let mark = span(&ElementHandle::find_by_element_type_name(&bar, "PluginTile").next().unwrap());
                if (mark.1 - (room - 12.0)).abs() > 0.5 {
                    off.push(format!("{case}: the mark ends at {}, not {}", mark.1, room - 12.0));
                }
                if (field.1 + 2.0 - settings.0).abs() > 0.5 || settings.1 + 2.0 + 4.0 > mark.0 + 0.5 {
                    off.push(format!("{case}: field {field:?}, settings {settings:?}, mark {mark:?} are not side by side"));
                }
                if field.0 < last_tab + 2.0 - 0.5 {
                    off.push(format!("{case}: the field {field:?} runs over the tabs, which end at {last_tab}"));
                }
            }
        }
        assert!(off.is_empty(), "{off:#?}");
        // The preview under the field, in the main window, hangs where the field is.
        let ui = window();
        let bar = top(&ui).unwrap();
        bar.window().set_size(slint::LogicalSize::new(700.0, 48.0));
        bar.set_room(700.0);
        bar.set_tabs(all);
        bar.show().unwrap();
        bar.invoke_focus_search();
        bar.set_previewing(true);
        for _ in 0..3 {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(300));
            slint::platform::update_timers_and_animations();
            bar.window().take_snapshot().unwrap();
        }
        let chrome = ui.global::<crate::Chrome>();
        assert!(bar.get_search_width() > 36.0 && bar.get_search_width() < 300.0, "the field takes what room there is: {}", bar.get_search_width());
        assert_eq!((chrome.get_search_end(), chrome.get_search_w()), (bar.get_search_end(), bar.get_search_width()));
    }

    #[test]
    fn the_top_pills_tabs_are_the_profiles_sections() {
        use nori_core::settings::LibrarySection::*;
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let bar = TopBar::new().unwrap();
        bar.window().set_size(slint::PhysicalSize::new(1000, 48));
        bar.set_room(1000.0);
        bar.show().unwrap();
        let tabs = |bar: &TopBar| i_slint_backend_testing::ElementHandle::find_by_element_type_name(bar, "TabButton").count();
        let server = nori_core::browse::profile_rules(false);
        bar.set_tabs(library_tabs(&server));
        assert_eq!(tabs(&bar), 6, "Home, Albums, Artists, Songs, Playlists and Favorites");
        let provider = nori_core::browse::ProfileRules { sections: vec![Favourites, Playlists, History], ..server };
        bar.set_tabs(library_tabs(&provider));
        assert_eq!(tabs(&bar), 3, "Home, Playlists and Favorites: no empty Albums, Artists or Songs");
    }

    #[test]
    fn the_search_preview_steps_past_its_heads_and_round_its_ends() {
        use nori_core::search::{PreviewKind, PreviewLine as L};
        let lines = [L::Head { kind: PreviewKind::Artists }, L::Artist { at: 0 }, L::Head { kind: PreviewKind::Songs }, L::Song { at: 0 }, L::Song { at: 1 }, L::All];
        for (at, by, to) in [(-1, 1, 1), (-1, -1, 5), (1, 1, 3), (3, -1, 1), (5, 1, 1), (1, -1, 5)] {
            assert_eq!(preview_step(&lines, at, by), to, "from {at} by {by}");
        }
        assert_eq!(preview_step(&[], -1, 1), -1);
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

    /// The top bar's profiles menu draws them in the order given, the one in use where it stands and not
    /// picked again; a profile picked is the one switched to.
    #[test]
    fn the_profiles_menu_switches_to_the_profile_picked() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::LogicalSize::new(1280.0, 820.0));
        let chrome = ui.global::<crate::Chrome>();
        chrome.set_top_x(0.0);
        chrome.set_top_w(1280.0);
        let item = |name: &str, on: bool| crate::ProfileItem { name: name.into(), title: name.into(), detail: "elsewhere".into(), on, ..Default::default() };
        ui.set_profiles(ModelRc::new(VecModel::from(vec![item("1", false), item("2", true), item("3", false)])));
        let did = Rc::new(RefCell::new(Vec::new()));
        let seen = did.clone();
        ui.on_setting_action(move |name| seen.borrow_mut().push(name.to_string()));
        ui.show().unwrap();
        // Down the menu's column, opened again before each click.
        for y in (60..260).step_by(4) {
            ui.invoke_show_profiles();
            let position = slint::LogicalPosition::new(1100.0, y as f32);
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button: slint::platform::PointerEventButton::Left });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button: slint::platform::PointerEventButton::Left });
        }
        let mut switched: Vec<String> = did.borrow().iter().filter(|n| n.starts_with("server:")).cloned().collect();
        switched.dedup();
        assert_eq!(switched, ["server:1", "server:3"]);
    }

    /// The first screen sends what its rows and plus do, a quiet row too; the form goes back from there.
    #[test]
    fn the_first_screen_sends_what_is_chosen() {
        use crate::settings::{provider_rows, provider_top, Plugged, Shelf};
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        let repo = nori_plugins::Repo { url: nori_plugins::OFFICIAL.into(), official: true };
        let version = nori_plugins::Version { repo: repo.url.clone(), version: "1.0.0".into(), released: None, url: String::new(), sha256: String::new(), key: None, signature: None };
        let offer = nori_plugins::Offer { repo, id: "a.new".into(), name: "New".into(), about: None, source: None, readme: None, icon: None, points: vec!["provider".into()], versions: vec![version], installed: None, state: nori_plugins::OfferState::New };
        let plugged = Plugged { shelf: Shelf::Read(nori_plugins::Offers { offers: vec![offer], failed: Vec::new() }), ..Default::default() };
        let (prefs, facts) = (nori_core::settings::StoredPrefs::default(), crate::settings::Facts::default());
        ui.set_provider_rows(provider_rows(&plugged, &nori_core::providers::Kinds::builtin(), &prefs, &facts, crate::look::Today::now()));
        ui.set_provider_top(provider_top());
        ui.set_login_choosing(true);
        ui.set_view(View::Login);
        let did = Rc::new(RefCell::new(Vec::new()));
        let seen = did.clone();
        ui.on_setting_action(move |name| seen.borrow_mut().push(name.to_string()));
        ui.show().unwrap();
        let click = |x: f32, y: f32| {
            let position = slint::LogicalPosition::new(x, y);
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button: slint::platform::PointerEventButton::Left });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button: slint::platform::PointerEventButton::Left });
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
        };
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        // Two rows of 68 and the head, centered in the window.
        for y in (0..820).step_by(4) {
            click(640.0, y as f32);
        }
        for x in (840..880).step_by(4) {
            for y in (300..420).step_by(4) {
                click(x as f32, y as f32);
            }
        }
        let got = did.borrow().clone();
        assert!(got.iter().any(|n| n == "provider-own"), "{got:?}");
        assert!(got.iter().any(|n| n == "provider-offer:a.new"), "the quiet row is chosen too: {got:?}");
        assert!(got.iter().any(|n| n == "plugins-sheet"), "the plus: {got:?}");
        did.borrow_mut().clear();
        ui.set_login_form(LoginKind { asks_address: true, ..Default::default() });
        ui.set_login_choosing(false);
        ui.set_login_adding(true);
        click(32.0, 60.0);
        assert_eq!(*did.borrow(), ["provider-again"]);
        did.borrow_mut().clear();
        ui.set_login_form(LoginKind { sign_in: "Sign in".into(), guest: "Continue as guest".into(), ..Default::default() });
        for y in (0..820).step_by(4) {
            for x in (400..880).step_by(40) {
                click(x as f32, y as f32);
            }
        }
        for act in ["provider-sign-in", "provider-guest"] {
            assert!(did.borrow().iter().any(|n| n == act), "{act}: {:?}", did.borrow());
        }
        did.borrow_mut().clear();
        ui.set_login_adding(false);
        click(32.0, 60.0);
        assert!(did.borrow().is_empty(), "editing a profile, nothing to choose again");
    }

    /// The account sheet of a provider offering a code first: its button asks for the code, which shows
    /// with Open and Copy while it is checked; expired, only a new code is offered.
    #[test]
    fn the_account_sheet_shows_a_code_to_allow() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::LogicalSize::new(1280.0, 820.0));
        ui.set_signin_open(true);
        ui.set_signin_service("SoundCloud".into());
        ui.set_signin_code_first(true);
        ui.set_signin_window_too(true);
        ui.set_signin_code_words(words::code_words("SoundCloud"));
        let did = Rc::new(RefCell::new(Vec::new()));
        for (name, seen) in ["code", "open", "copy", "window"].map(|n| (n, did.clone())) {
            let heard = move || seen.borrow_mut().push(name);
            match name {
                "code" => ui.on_sign_in_code(heard),
                "open" => ui.on_open_code_page(heard),
                "copy" => ui.on_copy_code(heard),
                _ => ui.on_sign_in_window(heard),
            }
        }
        ui.show().unwrap();
        let settle = || {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
        };
        let pressed = || {
            did.borrow_mut().clear();
            settle();
            for y in (200..620).step_by(6) {
                for x in (440..840).step_by(12) {
                    let position = slint::LogicalPosition::new(x as f32, y as f32);
                    ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button: slint::platform::PointerEventButton::Left });
                    ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button: slint::platform::PointerEventButton::Left });
                }
            }
            let mut got = did.borrow().clone();
            got.sort();
            got.dedup();
            got
        };
        assert_eq!(pressed(), ["code", "window"], "a code first, a window instead");
        ui.set_signin_code_step(true);
        assert!(pressed().is_empty(), "nothing to press while the code is asked for");
        ui.set_signin_code("UJ6MYK".into());
        assert_eq!(pressed(), ["copy", "open"]);
        ui.set_signin_code_expired(true);
        assert_eq!(pressed(), ["code"], "a new code");
    }

    /// A provider with an account and nothing to fill gets two ways on; any other gets its one button.
    #[test]
    fn a_page_offers_what_its_provider_needs() {
        use nori_core::providers::{FieldSpec, FormField, Kinds, ProviderInfo, SignIn};
        use nori_core::settings::ServerKind;
        let info = |fields, sign_in| ProviderInfo { kind: ServerKind::Plugin { id: "a.b".into() }, fields, sign_in, identities: None };
        let web = || vec![SignIn::WebPage { start_url: "https://a.example".into() }];
        let page = login_form(&Kinds::builtin(), &info(Vec::new(), web()), Some("Music from A"), nori_core::providers::FirstStep::Account);
        assert_eq!((page.guest.as_str(), page.sign_in.as_str(), page.note.as_str()), (words::CONTINUE_AS_GUEST, "Sign in to a.b", "Music from A"));
        let form = login_form(&Kinds::builtin(), &info(vec![FieldSpec { what: FormField::Address, advanced: false }], web()), None, nori_core::providers::FirstStep::Form);
        assert_eq!((form.guest.as_str(), form.asks_address, form.name.as_str()), ("", true, "a.b"), "titled with the provider");
    }

    #[test]
    fn mixing_is_only_visible_in_fullscreen() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_mixing_label(words::MIXING.into());
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
        ui.set_view(View::Albums);
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
        for view in [View::Albums, View::Home, View::Shelf] {
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
            let position = slint::LogicalPosition::new(600.0, if view == View::Home { 250.0 } else { 400.0 });
            let (delta_x, delta_y) = if view == View::Home { (-6000.0, 0.0) } else { (0.0, -3000.0) };
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled { position, delta_x, delta_y });
            ui.window().take_snapshot().unwrap();
            assert!(requests.borrow().iter().any(|id| id.parse::<usize>().is_ok_and(|i| i > 20)), "later albums are drawn in view {view:?}: {:?}", requests.borrow());
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
        for view in [View::Shelf, View::ForYou] {
            let ui = window();
            ui.window().set_size(slint::PhysicalSize::new(1280, 820));
            ui.set_picks_loaded(true);
            ui.set_show_all_text(words::SHOW_ALL.into());
    ui.set_mixing_label(words::MIXING.into());
            ui.set_shelves(ModelRc::new(VecModel::from(vec![Shelf {
                title: "Recently played".into(), loaded: true,
                cards: cards((0..100).map(|i| Card { id: i.to_string().into(), art: i.to_string().into(), ..Default::default() })),
            }])));
            if view == View::ForYou {
                ui.set_picks(ModelRc::new(VecModel::from((0..100).map(|i| crate::Pick {
                    id: i.to_string().into(), covers: ModelRc::new(VecModel::from(vec![i.to_string().into()])), ..Default::default()
                }).collect::<Vec<_>>())));
            }
            let weak = ui.as_weak();
            ui.on_go(move |view| weak.unwrap().set_view(view));
            let weak = ui.as_weak();
            ui.on_back(move || weak.unwrap().set_view(View::Home));
            let requests = Rc::new(RefCell::new(Vec::new()));
            let seen = requests.clone();
            ui.on_art(move |id, _, _| { seen.borrow_mut().push(id.to_string()); Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(16, 16)) });
            let seen = requests.clone();
            ui.on_pick_art(move |ids, _| { seen.borrow_mut().push(ids.row_data(0).unwrap().to_string()); Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(16, 16)) });
            ui.show().unwrap();
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            let frame = ui.window().take_snapshot().unwrap();
            let dim = ui.global::<crate::Theme>().get_dim().to_argb_u8();
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
            assert_eq!(ui.get_view(), View::Home, "the back button returns to Home");
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
        }
    }

    #[test]
    fn a_card_opens_when_its_shelf_is_read_again_under_the_press() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = window();
        ui.window().set_size(slint::LogicalSize::new(1280.0, 820.0));
        ui.set_view(View::Home);
        ui.set_picks_loaded(true);
        let shelf = || Shelf { loaded: true, cards: cards((0..12).map(|i| Card { id: i.to_string().into(), ..Default::default() })), ..Default::default() };
        let shelves = Rc::new(VecModel::from(vec![shelf()]));
        ui.set_shelves(ModelRc::from(shelves.clone()));
        let opened = Rc::new(RefCell::new(Vec::new()));
        let seen = opened.clone();
        ui.on_open_album(move |id| seen.borrow_mut().push(id.to_string()));
        ui.show().unwrap();
        i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
        slint::platform::update_timers_and_animations();
        // The first card's cover, under Home's title and the shelf's.
        let at = slint::LogicalPosition::new(100.0, 250.0);
        let button = slint::platform::PointerEventButton::Left;
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at });
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position: at, button });
        // The scrolling shelf hands the press on after a moment.
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(200));
        slint::platform::update_timers_and_animations();
        shelves.set_row_data(0, shelf());
        slint::platform::update_timers_and_animations();
        ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: at, button });
        assert_eq!(*opened.borrow(), ["0"]);
    }

    /// A shelf's newer answer slides the cards it kept from their old places to their new ones.
    #[test]
    fn a_shelfs_newer_answer_slides_its_cards_to_their_places() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = window();
        ui.window().set_size(slint::LogicalSize::new(1280.0, 820.0));
        ui.set_view(View::Home);
        ui.set_picks_loaded(true);
        let shelf = |ids: &[&str], old: Option<&Shelf>| Shelf {
            loaded: true,
            cards: cards(moved(old, ids.iter().map(|&id| Card { id: id.into(), ..Default::default() }).collect()).into_iter()),
            ..Default::default()
        };
        let first = shelf(&["a", "b", "c"], None);
        let shelves = Rc::new(VecModel::from(vec![first.clone()]));
        ui.set_shelves(ModelRc::from(shelves.clone()));
        let opened = Rc::new(RefCell::new(Vec::new()));
        let seen = opened.clone();
        ui.on_open_album(move |id| seen.borrow_mut().push(id.to_string()));
        ui.show().unwrap();
        let wait = |ms| {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(ms));
            slint::platform::update_timers_and_animations();
        };
        wait(1000);
        let click = || {
            let at = slint::LogicalPosition::new(100.0, 250.0);
            let button = slint::platform::PointerEventButton::Left;
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position: at, button });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: at, button });
        };
        shelves.set_row_data(0, shelf(&["x", "b", "a"], Some(&first)));
        wait(16);
        click();
        wait(1000);
        click();
        assert_eq!(*opened.borrow(), ["a", "x"]);
    }

    #[test]
    fn shelves_drag_with_a_mouse() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_view(View::Home);
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
        ui.set_view(View::Home);
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
                let pending: Vec<_> = std::mem::take(&mut art.borrow_mut().wanted);
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
            ui.set_inspector(if full { Panel::None } else { Panel::Queue });
            ui.set_full_panel(Panel::Queue);
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
            ui.set_inspector(Panel::None);
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
    fn a_paired_device_is_picked_and_forgotten_only_with_its_cross() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::LogicalSize::new(1280.0, 820.0));
        ui.set_devices_on(true);
        ui.set_inspector(Panel::Devices);
        let link = format!("nori://pair#k={}&d=mac&a=10.0.0.2%3A41234&n=Mac", "ab".repeat(32));
        ui.set_devices(ModelRc::new(VecModel::from(vec![crate::DeviceRow { name: "This computer".into(), active: true, ..Default::default() }])));
        ui.set_pairing_link(link.as_str().into());
        ui.set_pairing_qr(crate::jam::qr(&link));
        use nori_core::remote::{wire::DeviceKind, PairedReach};
        let paired = [("pixel", "Pixel", PairedReach::Answers, 0, DeviceKind::Phone), ("desk", "Desk", PairedReach::Away, 3, DeviceKind::Desktop)]
            .map(|(id, name, reach, seen_days, kind)| words::paired_row(&nori_core::remote::PairedDevice { id: id.into(), name: name.into(), kind, reach, active: false, seen_days }));
        ui.set_paired(ModelRc::new(VecModel::from(paired.to_vec())));
        let forgot = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let heard = forgot.clone();
        ui.on_forget_device(move |id| heard.borrow_mut().push(id.to_string()));
        let picked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let heard = picked.clone();
        ui.on_pick_device(move |id| heard.borrow_mut().push(id.to_string()));
        let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let heard = asked.clone();
        ui.on_pair(move |link| heard.borrow_mut().push(link.to_string()));
        ui.show().unwrap();
        let settle = || {
            for _ in 0..3 {
                ui.window().take_snapshot().unwrap();
                i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
                slint::platform::update_timers_and_animations();
            }
        };
        let click = |label: &str| {
            settle();
            let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(&ui, label).last().unwrap_or_else(|| panic!("{label} shown"));
            let (at, size) = (found.absolute_position(), found.size());
            let at = slint::LogicalPosition::new(at.x + size.width / 2.0, at.y + size.height / 2.0);
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position: at, button: slint::platform::PointerEventButton::Left });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: at, button: slint::platform::PointerEventButton::Left });
            settle();
        };
        settle();
        shot(&ui, "after-devices");
        click("Nearby");
        assert_eq!(*picked.borrow(), ["pixel"], "a paired device that answers is picked, as a device above");
        assert!(!ui.get_forget_open());
        click("Not nearby · seen 3 days ago");
        assert!(!ui.get_forget_open() && picked.borrow().len() == 1, "one out of reach is neither picked nor forgotten");
        click("Forget Desk");
        assert!(ui.get_forget_open() && ui.get_forgetting().id == "desk", "asked first");
        assert!(forgot.borrow().is_empty());
        shot(&ui, "after-forget");
        click("Cancel");
        assert!(!ui.get_forget_open() && forgot.borrow().is_empty(), "kept");
        click("Forget Desk");
        click("Forget");
        assert_eq!(*forgot.borrow(), ["desk"], "that one only");
        assert!(!ui.get_forget_open());
        click("Pair a device…");
        assert!(ui.get_pairing_open());
        ui.set_pair_link(link.as_str().into());
        ui.set_pair_error("Can’t reach Mac. Is it on this network, with remote control on?".into());
        shot(&ui, "after-pairing");
        click("Pair");
        assert_eq!(*asked.borrow(), [link]);
    }

    /// Saves the window as `name`.raw (its size, then RGBA) under NORI_DESKTOP_SHOTS, when set.
    fn shot(ui: &AppWindow, name: &str) {
        let Some(dir) = std::env::var_os("NORI_DESKTOP_SHOTS") else { return };
        let shot = ui.window().take_snapshot().unwrap();
        let mut bytes = shot.width().to_le_bytes().to_vec();
        bytes.extend(shot.height().to_le_bytes());
        bytes.extend_from_slice(shot.as_bytes());
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.raw")), bytes).unwrap();
    }

    #[test]
    fn devices_open_in_the_full_player() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        for (full, panel, inspector, open) in [(false, Panel::Lyrics, Panel::Devices, true), (false, Panel::Devices, Panel::Queue, false), (true, Panel::Devices, Panel::None, true), (true, Panel::Lyrics, Panel::Devices, false)] {
            ui.set_full_player(full);
            ui.set_full_panel(panel);
            ui.set_inspector(inspector);
            assert_eq!(devices_open(&ui), open, "full {full}, panel {panel:?}, inspector {inspector:?}");
        }
    }

    #[test]
    fn pick_covers_have_no_background_seams() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.set_view(View::Home);
        ui.set_inspector(Panel::Devices);
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
        ui.set_view(View::Home);
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

    /// A playlist page the profile may change: its list head renames it with its name filled in, and
    /// deletes it only once the sheet that asks is answered.
    #[test]
    fn a_playlist_is_renamed_and_deleted_only_once_asked() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        ui.set_view(View::Page);
        ui.set_page_kind(PageLayout::List);
        ui.set_page_title("Road trip".into());
        ui.set_page_editable(true);
        let did = Rc::new(RefCell::new(Vec::new()));
        let seen = did.clone();
        ui.on_rename_playlist(move |name| seen.borrow_mut().push(format!("rename {name}")));
        let seen = did.clone();
        ui.on_delete_playlist(move || seen.borrow_mut().push("delete".to_string()));
        ui.show().unwrap();
        let tap = |x: f32, y: f32| {
            press(ui.window(), x, y, slint::platform::PointerEventButton::Left);
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
        };
        tap(0.0, 0.0);
        // The list head's pencil and trash, at its right end; the sheets' buttons at their foot.
        let (rename, delete) = ((1180.0, 405.0), (1225.0, 405.0));
        let (cancel, confirm) = ((675.0, 170.0), (765.0, 170.0));
        tap(rename.0, rename.1);
        assert_eq!(ui.get_rename_name(), "Road trip", "the field starts with the playlist's name");
        ui.set_rename_name("Rain".into());
        tap(confirm.0, 189.0);
        assert_eq!(*did.borrow(), ["rename Rain"]);
        // The app closes it once the name is sent.
        ui.set_rename_open(false);
        tap(delete.0, delete.1);
        assert!(ui.get_delete_open());
        tap(cancel.0, cancel.1);
        assert!(!ui.get_delete_open(), "Cancel closes the sheet");
        tap(delete.0, delete.1);
        tap(100.0, 600.0);
        assert!(!ui.get_delete_open(), "a press beside the sheet closes it");
        assert_eq!(*did.borrow(), ["rename Rain"], "nothing is deleted before the sheet is answered");
        tap(delete.0, delete.1);
        tap(confirm.0, confirm.1);
        assert_eq!(*did.borrow(), ["rename Rain", "delete"]);
    }

    /// Add to playlist: a playlist picked takes the song, one that has it already takes nothing, and New
    /// playlist turns into its name's field in place, made with Create.
    #[test]
    fn the_playlist_picker_adds_to_one_or_names_a_new_one() {
        slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
            i_slint_backend_testing::TestingBackendOptions { mock_time: true, renderer_name: Some("skia-software".into()), ..Default::default() },
        ))).unwrap();
        let ui = window();
        ui.window().set_size(slint::PhysicalSize::new(1280, 820));
        let choice = |title: &str, full: bool| PlaylistChoice { title: title.into(), full, ..Default::default() };
        ui.set_picker_choices(ModelRc::new(VecModel::from(vec![choice("Road trip", true), choice("Rain", false)])));
        ui.set_picker_open(true);
        let did = Rc::new(RefCell::new(Vec::new()));
        let seen = did.clone();
        ui.on_add_to_playlist(move |i| seen.borrow_mut().push(format!("add {i}")));
        let seen = did.clone();
        ui.on_add_to_new_playlist(move |name| seen.borrow_mut().push(format!("new {name}")));
        ui.show().unwrap();
        let settle = || {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_secs(1));
            slint::platform::update_timers_and_animations();
            ui.window().take_snapshot().unwrap();
        };
        let tap = |x: f32, y: f32| {
            press(ui.window(), x, y, slint::platform::PointerEventButton::Left);
            settle();
        };
        settle();
        // The sheet's rows under its title: New playlist, then the playlists; one that has the song takes nothing.
        let (new, full, other) = (151.0, 215.0, 263.0);
        tap(600.0, full);
        tap(600.0, other);
        tap(600.0, new);
        assert!(ui.get_picker_naming(), "New playlist turns into its name's field");
        for key in ["R", "a", "i", "n"] {
            ui.window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: key.into() });
            ui.window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: key.into() });
        }
        settle();
        tap(780.0, new);
        assert_eq!(*did.borrow(), ["add 1", "new Rain"]);
    }
}
