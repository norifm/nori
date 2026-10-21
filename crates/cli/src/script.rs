//! `nori-cli --script`: a non-interactive player for scripts and renders. Logs in, searches, queues
//! and plays through the core and nori-engine.
//!
//! ```text
//! nori-cli --script --url http://localhost:4533 --user admin --password admin --play "noise" --crossfade 6
//! nori-cli --script ... --search "noise" --songs 2 --start 570 --crossfade 6 --mix-albums --wav out.wav
//! nori-cli --script --provider youtubeMusic --play "daft punk" --songs 1 --wav out.wav
//! nori-cli --script --provider youtubeMusic --sign-in cookies.txt --home
//! ```
//!
//! Without `--wav` it plays on the sound card and reads commands from stdin (see `HELP`). With `--wav`
//! it renders the queue to a file and exits at the end.

use std::io::BufRead;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nori_core::cache_policy::{Page, Read};
use nori_core::client::{Client, NetProfile};
use nori_core::settings::{GainMode, SavedServer, ServerKind, StoredPrefs};
use nori_core::settings_store::{APPLY_AUDIO, APPLY_GAIN, REPLAN, SOUND};
use nori_core::{Core, Song};
use nori_engine::core::{resolving, settings, Analyses, CoreApp, CoreLibrary, CoreQueue, Downloader, Measurer};
use nori_engine::{AudioOutput, Config, Engine, Event, State, Store, WavOutput};
use nori_http::Http;
use nori_output_cpal::CpalOutput;

use crate::backend::{block_on, Audio};
use crate::text::clock;

const HELP: &str = "play, pause, next, prev, seek <s>, crossfade <s>|off, automix on|off, gain off|track|album|auto, pos, positions on|off, set <name> <value>, tuning on|off, band <i> <dB>, search <q>, queue, quit";

struct Args {
    /// The kind of profile: a server's unless another is named.
    kind: ServerKind,
    /// A file of what the profile's account signs in with (a browser session's cookies).
    sign_in: Option<PathBuf>,
    /// Print the source's own Home's shelves.
    home: bool,
    url: String,
    user: String,
    password: String,
    data: PathBuf,
    search: Option<String>,
    songs: usize,
    start_s: f64,
    crossfade: Option<i32>,
    automix: bool,
    /// Crossfade within an album played in order too (by default albums stay gapless).
    mix_albums: bool,
    replay_gain: Option<GainMode>,
    /// Float output to the device or file.
    hi_res: bool,
    /// Download the songs found before playing them.
    download: bool,
    /// No login and no audio requests; the queue is the downloaded songs.
    offline: bool,
    mpris: bool,
    wav: Option<PathBuf>,
    pace: f64,
    device: Option<String>,
}

fn usage() -> ! {
    eprintln!(
        "usage: nori-cli --script --url URL --user USER --password PASSWORD [--search|--play QUERY] [--songs N] [--start SECONDS]\n\
         \x20               [--crossfade SECONDS] [--automix] [--mix-albums] [--replay-gain off|track|album|auto] [--hi-res]\n\
         \x20               [--download] [--offline] [--mpris]\n\
         \x20               [--wav OUT.wav [--pace X]] [--device NAME] [--devices]\n\
         \x20      nori-cli --script --provider KIND [the same, without the server] [--sign-in FILE] [--home]\n\
         \x20               [--data DIR]\n\
         The url, user and password may also come from NORI_URL, NORI_USER and NORI_PASSWORD.\n\
         KIND is a profile kind as the settings store it (youtubeMusic)."
    );
    std::process::exit(2)
}

fn gain_mode(v: &str) -> Option<GainMode> {
    [("off", GainMode::Off), ("track", GainMode::Track), ("album", GainMode::Album), ("auto", GainMode::Auto)].into_iter().find(|(n, _)| *n == v).map(|(_, m)| m)
}

fn args(argv: Vec<String>) -> Args {
    let env = |k: &str| std::env::var(k).ok();
    let mut a = Args {
        kind: ServerKind::Subsonic,
        sign_in: None,
        home: false,
        url: env("NORI_URL").unwrap_or_default(),
        user: env("NORI_USER").unwrap_or_default(),
        password: env("NORI_PASSWORD").unwrap_or_default(),
        data: std::env::temp_dir().join("nori-cli"),
        search: None,
        songs: usize::MAX,
        start_s: 0.0,
        crossfade: None,
        automix: false,
        mix_albums: false,
        replay_gain: None,
        hi_res: false,
        download: false,
        offline: false,
        mpris: false,
        wav: None,
        pace: 16.0,
        device: None,
    };
    let mut it = argv.into_iter();
    while let Some(k) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| usage());
        match k.as_str() {
            "--provider" => a.kind = ServerKind::from_id(&v()).unwrap_or_else(|| usage()),
            "--sign-in" => a.sign_in = Some(v().into()),
            "--home" => a.home = true,
            "--url" => a.url = v(),
            "--user" => a.user = v(),
            "--password" => a.password = v(),
            "--data" => a.data = v().into(),
            "--search" | "--play" => a.search = Some(v()),
            "--songs" => a.songs = v().parse().unwrap_or_else(|_| usage()),
            "--start" => a.start_s = v().parse().unwrap_or_else(|_| usage()),
            "--crossfade" => a.crossfade = Some(v().parse().unwrap_or_else(|_| usage())),
            "--automix" => a.automix = true,
            "--mix-albums" => a.mix_albums = true,
            "--replay-gain" => a.replay_gain = Some(gain_mode(&v()).unwrap_or_else(|| usage())),
            "--hi-res" => a.hi_res = true,
            "--download" => a.download = true,
            "--offline" => a.offline = true,
            "--mpris" => a.mpris = true,
            "--wav" => a.wav = Some(v().into()),
            "--pace" => a.pace = v().parse().unwrap_or_else(|_| usage()),
            "--device" => a.device = Some(v()),
            "--devices" => {
                for d in CpalOutput::devices() {
                    println!("{d}");
                }
                std::process::exit(0)
            }
            _ => usage(),
        }
    }
    if !nori_core::providers::server_ready(a.profile()) {
        usage();
    }
    a
}

impl Args {
    /// The profile the options name.
    fn profile(&self) -> SavedServer {
        SavedServer { kind: self.kind, url: self.url.clone(), user: self.user.clone(), password: self.password.clone(), ..Default::default() }
    }
}

/// The media controls over the script's engine and its songs.
struct Keys {
    engine: Arc<Engine>,
    songs: Arc<Mutex<Vec<Song>>>,
}

impl nori_mpris::Controls for Keys {
    fn play(&self) {
        self.engine.play();
    }
    fn pause(&self) {
        self.engine.pause();
    }
    fn toggle(&self) {
        self.engine.toggle();
    }
    fn next(&self) {
        self.engine.next();
    }
    fn previous(&self) {
        self.engine.previous();
    }
    fn seek(&self, ms: i64) {
        self.engine.seek(ms);
    }
    fn now(&self) -> nori_mpris::Now {
        let s = self.engine.status();
        let song = s.index.and_then(|i| self.songs.lock().unwrap().get(i).cloned()).unwrap_or_default();
        nori_mpris::Now {
            playing: s.state == State::Playing,
            loaded: s.state == State::Paused,
            index: s.index,
            title: song.title,
            artist: song.artist,
            album: song.album,
            length_ms: song.duration as i64 * 1000,
            position_ms: s.position_now(),
            starred: song.starred,
            art: None,
            art_file: None,
        }
    }
}

struct Cli {
    core: Arc<Core>,
    client: Arc<Client>,
    prefs: StoredPrefs,
    engine: Arc<Engine>,
    /// The queue as last set, shared with the event printer and MPRIS for titles.
    songs: Arc<Mutex<Vec<Song>>>,
    downloader: Arc<Downloader>,
}

impl Cli {
    fn search(&self, query: &str) -> Result<Vec<Song>, String> {
        let read = Read::Search { query: query.into(), songs: 50, albums: 0, artists: 0 };
        let songs = match block_on(self.client.read_now(read)).map_err(|e| e.to_string())? {
            Page::Found { v } => v.songs,
            _ => Vec::new(),
        };
        // Provider songs (octo-fiesta) are downloaded by the server when requested: never queued from a search.
        Ok(songs.into_iter().filter(|s| !s.is_provider()).collect())
    }

    fn queue(&mut self, songs: Vec<Song>, start_ms: i64) {
        crate::backend::app().register(songs.clone());
        crate::backend::app().set(songs.iter().map(|s| s.id.clone()).collect(), Some(0), false, None);
        *self.songs.lock().unwrap() = songs;
        self.engine.queue_changed();
        self.engine.play_at(0, start_ms);
    }

    fn download(&self, songs: &[Song]) {
        match self.core.download_queue(songs.to_vec()) {
            Ok(q) => println!("downloading {} ({} asked again)", q.fresh.len(), q.again.len()),
            Err(e) => println!("download failed: {e}"),
        }
        self.downloader.start(nori_core::transfers::DOWNLOADS_AT_ONCE);
    }

    /// The stored settings; a device's own sound may have been loaded since the last put.
    fn kept(&self) -> StoredPrefs {
        crate::backend::app().settings.current().unwrap_or_else(|| self.prefs.clone())
    }

    fn put(&mut self, prefs: StoredPrefs) {
        self.apply(crate::backend::app().settings.put(prefs));
    }

    /// Reloads the stored settings and applies `effect` to the engine.
    fn apply(&mut self, effect: u32) {
        self.prefs = self.kept();
        if effect & (APPLY_AUDIO | SOUND) != 0 {
            self.engine.set_settings(settings(&self.prefs));
        }
        if effect & APPLY_GAIN != 0 {
            self.engine.gain_changed();
        }
        if effect & REPLAN != 0 {
            self.engine.replan();
        }
    }
}

fn title(songs: &[Song], index: usize) -> String {
    songs.get(index).map_or_else(|| format!("#{index}"), |s| format!("{} - {} ({})", s.artist, s.title, nori_core::numbers::clock(s.duration as i64, false)))
}

fn print_songs(songs: &[Song]) {
    for i in 0..songs.len() {
        println!("  {i}: {}", title(songs, i));
    }
}

pub fn main(argv: Vec<String>) {
    let a = args(argv);
    std::fs::create_dir_all(&a.data).unwrap_or_else(|e| panic!("{}: {e}", a.data.display()));
    let db = a.data.join("nori.db").to_string_lossy().into_owned();
    let core = Core::new(db.clone(), "cli".into(), crate::backend::app().clone()).unwrap_or_else(|e| panic!("the database: {e}"));
    let http = Http::new();
    let profile = match a.offline {
        true => a.profile(),
        false => block_on(nori_core::providers::check_profile(http.clone(), a.profile())).unwrap_or_else(|e| {
            eprintln!("login failed: {e}");
            std::process::exit(1)
        }),
    };
    core.configure_profile(profile).unwrap_or_else(|e| panic!("the server: {e}"));
    let client = Client::new(core.clone(), http.clone(), Default::default());
    client.set_profile(NetProfile { url: a.url.clone(), ..Default::default() });
    if let Some(path) = &a.sign_in {
        let answer = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        println!("{}", crate::text::sign_in(a.kind, &block_on(client.sign_in(answer))));
    }
    if a.home {
        match block_on(client.read_now(Read::HomeFeed)) {
            Ok(Page::Feed { v }) => v.iter().for_each(|f| println!("{}", crate::text::feed_shelf(f))),
            Ok(_) => {}
            Err(e) => println!("home failed: {e}"),
        }
    }
    if (a.sign_in.is_some() || a.home) && a.search.is_none() {
        return;
    }
    let mut prefs = crate::backend::app().settings.open(&db).unwrap_or_else(|e| panic!("the settings: {e}"));
    prefs.crossfade_sec = a.crossfade.unwrap_or(prefs.crossfade_sec);
    prefs.auto_mix = a.automix;
    prefs.crossfade_keep_albums = !a.mix_albums;
    prefs.replay_gain = a.replay_gain.unwrap_or(prefs.replay_gain);
    prefs.hi_res = a.hi_res;
    crate::backend::app().settings.put(prefs.clone());

    let output: Box<dyn AudioOutput> = match (&a.wav, &a.device) {
        (Some(path), _) if a.hi_res => Box::new(WavOutput::new(path, a.pace).in_float()),
        (Some(path), _) => Box::new(WavOutput::new(path, a.pace)),
        (None, Some(name)) => Box::new(CpalOutput::with_device(name)),
        (None, None) => Box::new(CpalOutput::new()),
    };
    let (tx, events): (_, Receiver<Event>) = channel();
    let store = Store::open(a.data.join("music"), prefs.cache_mb.max(0) as u64 * 1024 * 1024).unwrap_or_else(|e| panic!("the music directory: {e}"));
    let audio = Arc::new(Audio::new(http.clone(), a.offline));
    let bytes = resolving(&client, audio.clone());
    let analyses = Analyses::of(client.clone());
    let downloader = Downloader::new(client.clone(), bytes.clone(), store.clone(), analyses.clone());
    let app = CoreApp::new(core.session.clone()).measuring(Measurer::new(analyses.clone(), store.clone())).per_device(core.clone());
    let library = CoreLibrary { client: client.clone(), bytes, store: Some(store), analyses };
    let engine = Engine::start(library, app, CoreQueue(core.session.clone()), output, None, Config { memory_mb: 256, settings: settings(&prefs), ..Config::default() }, move |e| {
        let _ = tx.send(e);
    });
    let mut cli = Cli { core, client, prefs, engine: Arc::new(engine), songs: Arc::default(), downloader };
    let how = if a.offline { "offline" } else { "logged in" };
    let to = if a.url.is_empty() { crate::text::provider(a.kind).name.to_string() } else { format!("{} as {}", a.url, a.user) };
    println!("{how} to {to}; crossfade {} s, AutoMix {}", cli.prefs.crossfade_sec, if cli.prefs.auto_mix { "on" } else { "off" });

    let found = match (&a.search, a.offline) {
        (_, true) => Ok(cli.core.downloads(true).unwrap_or_default()),
        (Some(q), false) => cli.search(q),
        (None, false) => Ok(Vec::new()),
    };
    match found {
        Ok(found) if !found.is_empty() => {
            let found: Vec<Song> = found.into_iter().take(a.songs).collect();
            print_songs(&found);
            if a.download {
                cli.download(&found);
                cli.downloader.wait();
            }
            cli.queue(found, (a.start_s * 1000.0) as i64);
        }
        Ok(_) if a.search.is_some() || a.offline => {
            println!("nothing found");
            cli.engine.stop();
            return;
        }
        Ok(_) => {}
        Err(e) => println!("search failed: {e}"),
    }

    let songs = cli.songs.clone();
    let title_at = move |i: usize| title(&songs.lock().unwrap(), i);
    if a.wav.is_some() {
        // Render: follow the engine to the end of the queue.
        loop {
            match events.recv_timeout(Duration::from_secs(600)) {
                Ok(Event::Song { index, .. }) => println!("now: {}", title_at(index)),
                Ok(Event::Error { id, message }) => println!("error: {id} {message}"),
                Ok(Event::State(State::Ended)) => break,
                Ok(Event::State(State::Idle)) if cli.songs.lock().unwrap().is_empty() => break,
                Ok(_) => {}
                Err(_) => {
                    println!("timed out");
                    break;
                }
            }
        }
        let s = cli.engine.status();
        println!("ended; underruns {}; audio requests {}", s.underruns, audio.requests.load(Ordering::Relaxed));
        cli.engine.stop();
        return;
    }

    let mpris = a.mpris.then(|| {
        let songs = cli.songs.clone();
        let controls = Arc::new(Keys { engine: cli.engine.clone(), songs });
        let m = nori_mpris::Mpris::start("nori").map_err(|e| println!("no media controls: {e}")).ok()?;
        m.serve(Some(controls));
        Some(m)
    });
    let mpris = mpris.flatten();
    let shown = title_at.clone();
    std::thread::spawn(move || {
        for e in events {
            if matches!(e, Event::Song { .. } | Event::State(_)) {
                if let Some(m) = &mpris {
                    m.changed();
                }
            }
            match e {
                Event::Song { index, .. } => println!("now: {}", shown(index)),
                Event::State(s) => println!("{s:?}"),
                Event::Position { index, ms, .. } => println!("  {} at {}", shown(index), clock(ms)),
                Event::Error { id, message } => println!("error: {id} {message}"),
                Event::Output { name } => println!("output: {name}"),
                Event::Stopped { .. } => println!("stopped"),
                Event::Buffering(on) => println!("{}", if on { "buffering" } else { "playing again" }),
                Event::Looped { index, .. } => println!("again: {}", shown(index)),
                Event::Title(t) => println!("on air: {t}"),
                // No offline bridge here: the error rules stop playback.
                Event::Bridge { .. } => println!("stopped: the network is gone"),
                Event::Waiting(on) => println!("{}", if on { "waiting for the network" } else { "trying again" }),
                Event::Mixing(on) => println!("{}", if on { "mixing" } else { "mixed" }),
                Event::Placed { index, ms } => println!("  {} at {} (another path)", shown(index), clock(ms)),
                Event::Awake(_) | Event::Following(_) => {}
            }
        }
    });
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let (cmd, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
        match cmd {
            "play" => {
                cli.engine.play();
            }
            "pause" => cli.engine.pause(),
            "p" | "toggle" => cli.engine.toggle(),
            "next" | "n" => {
                cli.engine.next();
            }
            "prev" | "previous" => {
                cli.engine.previous();
            }
            "seek" => match rest.parse::<f64>() {
                Ok(s) => cli.engine.seek((s * 1000.0) as i64),
                Err(_) => println!("seek <seconds>"),
            },
            "crossfade" => {
                let secs = if rest == "off" { 0 } else { rest.parse().unwrap_or(6) };
                cli.put(StoredPrefs { crossfade_sec: secs, ..cli.kept() });
                println!("crossfade {secs} s");
            }
            "gain" => match gain_mode(rest) {
                Some(m) => {
                    cli.put(StoredPrefs { replay_gain: m, ..cli.kept() });
                    println!("ReplayGain {rest}");
                }
                None => println!("gain off|track|album|auto"),
            },
            "automix" => {
                cli.put(StoredPrefs { auto_mix: rest != "off", ..cli.kept() });
                println!("AutoMix {}", if cli.prefs.auto_mix { "on" } else { "off" });
            }
            "pos" => {
                let s = cli.engine.status();
                let name = s.index.map_or_else(|| "nothing".into(), &title_at);
                println!("{:?}: {} at {}{}; underruns {}", s.state, name, clock(s.position_now()), if s.mixing { " (mixing)" } else { "" }, s.underruns);
            }
            // Any setting by name, as the settings screen sets it: `set eq true`.
            "set" => {
                let (name, value) = rest.split_once(' ').unwrap_or((rest, ""));
                match crate::backend::app().settings.edit_by_name(name, value) {
                    Some(c) => {
                        cli.apply(c.effect);
                        println!("{name} = {value}");
                    }
                    None => println!("{name}: not a setting"),
                }
            }
            // The equalizer screen's low-latency buffer.
            "tuning" => cli.engine.set_shallow(rest != "off"),
            "band" => {
                let mut it = rest.split_whitespace();
                let (Some(Ok(i)), Some(Ok(db))) = (it.next().map(str::parse::<u32>), it.next().map(str::parse::<f32>)) else {
                    println!("band <index> <dB>");
                    continue;
                };
                let Some(b) = cli.kept().eq_bands.get(i as usize).copied() else {
                    println!("no band {i}");
                    continue;
                };
                if let Some((effect, _)) = crate::backend::app().settings.edit_band(i, nori_core::settings::SoundBand { gain_db: db, ..b }) {
                    cli.apply(effect);
                }
            }
            "positions" => cli.engine.position_updates((rest != "off").then(|| Duration::from_secs(1))),
            "search" => match cli.search(rest) {
                Ok(found) if !found.is_empty() => {
                    print_songs(&found);
                    cli.queue(found, 0);
                }
                Ok(_) => println!("nothing found"),
                Err(e) => println!("search failed: {e}"),
            },
            "download" => {
                let songs = cli.songs.lock().unwrap().clone();
                cli.download(&songs);
            }
            "queue" => print_songs(&cli.songs.lock().unwrap()),
            "q" | "quit" | "exit" => break,
            "" => {}
            _ => println!("{HELP}"),
        }
    }
    cli.engine.stop();
}
