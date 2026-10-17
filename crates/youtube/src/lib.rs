//! YouTube Music over InnerTube, without JavaScript: search, browse and the home feed through the
//! WEB_REMIX client (music.youtube.com), streams through the visionOS client, whose URLs need no
//! signature, n-transform or PO token. Optionally signed in with a browser session's cookies ([`auth`])
//! for the account's own feed, library and likes.
//!
//! The crate does no I/O of its own. It builds each [`Request`] and parses each [`Response`]; the
//! caller sends them with any HTTP client by implementing [`Http`], on any async runtime (or none, with a
//! blocking client behind a ready future). Answers become plain records ([`Song`], [`Album`], ...)
//! carrying YouTube's own ids.
//!
//! Best effort: YouTube changes either client without notice.

pub mod auth;
mod parse;
mod player;

use std::future::Future;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

pub use auth::Cookies;
pub use parse::{thumbnail_url, Album, AlbumLink, AlbumPage, Artist, ArtistLink, ArtistPage, Item, Playlist, PlaylistPage, SearchResults, Shelf, Song};
pub use player::{pick, Format, Stream};

/// One HTTP request: a POST of `body` as `application/json` when it is set, a GET otherwise.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

impl Request {
    /// The value of header `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// The status and whole body of an answer, and its `Set-Cookie` values (a signed-in session's rotated
/// cookies come in them; a client that signs no one in may leave them out). Error statuses are answers
/// too, not transport errors.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub set_cookies: Vec<String>,
}

/// The caller's HTTP client.
pub trait Http {
    /// Why a request did not come back.
    type Error;

    fn send(&self, request: Request) -> impl Future<Output = std::result::Result<Response, Self::Error>> + Send;
}

/// Why a call gave nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error<E> {
    /// The request did not come back.
    Transport(E),
    /// YouTube answered this error status.
    Status(u16),
    /// The answer was not what was expected.
    Parse(String),
    /// YouTube will not play the video; its reason.
    Unplayable(String),
    /// The call needs an account and there is none, or YouTube ended the session (its cookies were
    /// refused after a rotation): sign in again.
    SignedOut,
}

pub type Result<T, E> = std::result::Result<T, Error<E>>;

/// One InnerTube client as YouTube knows it.
struct Client {
    name: &'static str,
    version: &'static str,
    /// `X-YouTube-Client-Name`.
    number: &'static str,
    host: &'static str,
    /// None: the HTTP client's own.
    user_agent: Option<&'static str>,
    /// More `context.client` fields.
    device: &'static [(&'static str, &'static str)],
    /// Sent signed in when there is an account; the stream client never is (a web session's cookies on
    /// another client are what YouTube flags).
    signs_in: bool,
}

/// music.youtube.com's web client (yt-dlp `web_music`): search, browse and the account.
const WEB_REMIX: Client = Client { name: "WEB_REMIX", version: "1.20260707.12.00", number: "67", host: "music.youtube.com", user_agent: None, device: &[], signs_in: true };

/// The browser a signed-in session's requests say they come from.
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:140.0) Gecko/20100101 Firefox/140.0";

const VISIONOS_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
const VISIONOS_DEVICE: &[(&str, &str)] = &[("deviceMake", "Apple"), ("deviceModel", "RealityDevice17,1"), ("osName", "visionOS"), ("osVersion", "26.5.23O471")];

/// The visionOS builds asked for streams, in order (yt-dlp `visionos` is 1.02; 1.03 and 1.01 are its
/// siblings that zemer-app found serving whole songs when 0.1 was retired on 2026-10-07).
const VISIONOS: [Client; 3] = [
    Client { name: "VISIONOS", version: "1.02", number: "101", host: "www.youtube.com", user_agent: Some(VISIONOS_UA), device: VISIONOS_DEVICE, signs_in: false },
    Client { name: "VISIONOS", version: "1.03", number: "101", host: "www.youtube.com", user_agent: Some(VISIONOS_UA), device: VISIONOS_DEVICE, signs_in: false },
    Client { name: "VISIONOS", version: "1.01", number: "101", host: "www.youtube.com", user_agent: Some(VISIONOS_UA), device: VISIONOS_DEVICE, signs_in: false },
];

/// Where a youtube.com session's cookies are rotated, and the body it expects.
const ROTATE_URL: &str = "https://accounts.youtube.com/RotateCookies";
const ROTATE_BODY: &str = r#"[000,"-0000000000000000000"]"#;

/// Statuses with which YouTube refuses a retired client build (400 "Precondition check failed", 404).
fn retired(status: u16) -> bool {
    matches!(status, 400 | 404)
}

/// The search filters (`params`) of songs, albums and artists, as music.youtube.com sends them.
const SONGS: &str = "EgWKAQIIAWoKEAkQBRAKEAMQBA==";
const ALBUMS: &str = "EgWKAQIYAWoKEAkQBRAKEAMQBA==";
const ARTISTS: &str = "EgWKAQIgAWoKEAkQBRAKEAMQBA==";

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Client {
    /// The request of `endpoint` with `fields` beside the client's context, as `visitor` if known, signed
    /// in with `cookies` when given.
    fn request(&self, endpoint: &str, fields: Value, visitor: Option<&str>, cookies: Option<&Cookies>) -> Request {
        let origin = format!("https://{}", self.host);
        let user_agent = self.user_agent.or(cookies.map(|_| BROWSER_UA));
        let mut client = json!({ "clientName": self.name, "clientVersion": self.version, "hl": "en" });
        for (k, v) in self.device {
            client[*k] = (*v).into();
        }
        if let Some(ua) = self.user_agent {
            client["userAgent"] = ua.into();
        }
        if let Some(v) = visitor {
            client["visitorData"] = v.into();
        }
        let mut body = json!({ "context": { "client": client } });
        if let (Some(b), Value::Object(f)) = (body.as_object_mut(), fields) {
            b.extend(f);
        }
        let mut headers = vec![
            ("X-YouTube-Client-Name".to_string(), self.number.to_string()),
            ("X-YouTube-Client-Version".to_string(), self.version.to_string()),
            ("Origin".to_string(), origin.clone()),
        ];
        if let Some(ua) = user_agent {
            headers.push(("User-Agent".into(), ua.into()));
        }
        if let Some(v) = visitor {
            headers.push(("X-Goog-Visitor-Id".into(), v.into()));
        }
        if let Some(c) = cookies {
            if let Some(a) = auth::authorization(c, &origin, unix_now()) {
                headers.push(("Authorization".into(), a));
            }
            headers.push(("Cookie".into(), c.header()));
            headers.push(("X-Goog-AuthUser".into(), "0".into()));
            headers.push(("X-Origin".into(), origin.clone()));
        }
        Request { url: format!("{origin}/youtubei/v1/{endpoint}?prettyPrint=false"), headers, body: Some(body.to_string()) }
    }
}

/// Polls three futures side by side until all are done.
async fn join3<A: Future, B: Future, C: Future>(a: A, b: B, c: C) -> (A::Output, B::Output, C::Output) {
    use std::task::Poll;
    let (mut a, mut b, mut c) = (std::pin::pin!(a), std::pin::pin!(b), std::pin::pin!(c));
    let (mut ra, mut rb, mut rc) = (None, None, None);
    std::future::poll_fn(move |cx| {
        if ra.is_none() {
            if let Poll::Ready(v) = a.as_mut().poll(cx) {
                ra = Some(v);
            }
        }
        if rb.is_none() {
            if let Poll::Ready(v) = b.as_mut().poll(cx) {
                rb = Some(v);
            }
        }
        if rc.is_none() {
            if let Poll::Ready(v) = c.as_mut().poll(cx) {
                rc = Some(v);
            }
        }
        match (ra.is_some() && rb.is_some() && rc.is_some(), &mut ra, &mut rb, &mut rc) {
            (true, a, b, c) => Poll::Ready((a.take().unwrap(), b.take().unwrap(), c.take().unwrap())),
            _ => Poll::Pending,
        }
    })
    .await
}

/// The signed-in session: its cookies, and whether they changed since the caller last took them.
struct Account {
    cookies: Cookies,
    changed: bool,
}

/// How a rotation went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotated {
    /// The session's cookies are fresh.
    Fresh,
    /// Asked too soon after the last one (429): nothing changed, the session stands.
    TooSoon,
}

/// What a session's requests share: YouTube's visitor id, without which music videos answer "Sign in to
/// confirm you're not a bot", learned from the first answer; and the account, when signed in.
#[derive(Default)]
pub struct InnerTube {
    visitor: Mutex<Option<String>>,
    account: Mutex<Option<Account>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl InnerTube {
    fn visitor(&self) -> Option<String> {
        lock(&self.visitor).clone()
    }

    /// Signs in with a session's `cookies` (see [`auth`]); None signs out. Nothing is asked: a later
    /// call finds out whether YouTube takes them.
    pub fn set_cookies(&self, cookies: Option<Cookies>) {
        *lock(&self.account) = cookies.map(|cookies| Account { cookies, changed: false });
    }

    /// The session's cookies as they are now; None when signed out.
    pub fn cookies(&self) -> Option<Cookies> {
        lock(&self.account).as_ref().map(|a| a.cookies.clone())
    }

    /// The session's cookies when YouTube changed them since the last take: they are to be kept in
    /// place of the old ones at once, or the session may be lost.
    pub fn take_changed(&self) -> Option<Cookies> {
        lock(&self.account).as_mut().filter(|a| a.changed).map(|a| {
            a.changed = false;
            a.cookies.clone()
        })
    }

    fn take_set_cookies(&self, set_cookies: &[String]) {
        if let Some(a) = lock(&self.account).as_mut() {
            a.changed |= a.cookies.merge(set_cookies);
        }
    }

    /// `client`'s answer to `endpoint` once, parsed; an error status is [`Error::Status`]. Learns the
    /// visitor id and, signed in, the rotated cookies.
    async fn call_once<H: Http + Sync>(&self, client: &Client, http: &H, endpoint: &str, fields: Value) -> Result<Value, H::Error> {
        let cookies = self.cookies().filter(|_| client.signs_in);
        let r = http.send(client.request(endpoint, fields, self.visitor().as_deref(), cookies.as_ref())).await.map_err(Error::Transport)?;
        if cookies.is_some() {
            self.take_set_cookies(&r.set_cookies);
        }
        if !(200..300).contains(&r.status) {
            return Err(Error::Status(r.status));
        }
        let v: Value = serde_json::from_slice(&r.body).map_err(|e| Error::Parse(e.to_string()))?;
        if let Some(id) = v.pointer("/responseContext/visitorData").and_then(Value::as_str).filter(|id| !id.is_empty()) {
            lock(&self.visitor).get_or_insert_with(|| id.to_string());
        }
        Ok(v)
    }

    /// [`InnerTube::call_once`]; signed in, an answer refused (401) or given as to a visitor is asked once
    /// more after the cookies are rotated. Refused again, the session has ended.
    async fn call<H: Http + Sync>(&self, client: &Client, http: &H, endpoint: &str, fields: Value) -> Result<Value, H::Error> {
        let signed = client.signs_in && self.cookies().is_some();
        let refused = |r: &Result<Value, H::Error>| matches!(r, Err(Error::Status(401))) || matches!(r, Ok(v) if parse::logged_out(v));
        let first = self.call_once(client, http, endpoint, fields.clone()).await;
        if !signed || !refused(&first) {
            return first;
        }
        self.rotate(http).await?;
        let again = self.call_once(client, http, endpoint, fields).await;
        if refused(&again) {
            self.set_cookies(None);
            return Err(Error::SignedOut);
        }
        again
    }

    /// Asks Google to rotate the session's cookies, keeping the new ones ([`InnerTube::take_changed`]).
    /// A refusal (401) ends the session; any other failure leaves it as it was.
    pub async fn rotate<H: Http + Sync>(&self, http: &H) -> Result<Rotated, H::Error> {
        let cookies = self.cookies().ok_or(Error::SignedOut)?;
        let request = Request {
            url: ROTATE_URL.into(),
            headers: vec![("Origin".into(), "https://accounts.youtube.com".into()), ("Cookie".into(), cookies.header()), ("User-Agent".into(), BROWSER_UA.into())],
            body: Some(ROTATE_BODY.into()),
        };
        let r = http.send(request).await.map_err(Error::Transport)?;
        match r.status {
            200..=299 => {
                self.take_set_cookies(&r.set_cookies);
                Ok(Rotated::Fresh)
            }
            429 => Ok(Rotated::TooSoon),
            401 => {
                self.set_cookies(None);
                Err(Error::SignedOut)
            }
            status => Err(Error::Status(status)),
        }
    }

    /// The signed-in account's name, as YouTube shows it; [`Error::SignedOut`] when the cookies are no
    /// account's.
    pub async fn account_name<H: Http + Sync>(&self, http: &H) -> Result<String, H::Error> {
        if self.cookies().is_none() {
            return Err(Error::SignedOut);
        }
        let v = self.call(&WEB_REMIX, http, "account/account_menu", json!({})).await?;
        parse::account_name(&v).ok_or(Error::SignedOut)
    }

    /// Songs, albums and artists matching `query`: three filtered searches, side by side, each kind from
    /// its own answer (one page, about 20, of each).
    pub async fn search<H: Http + Sync>(&self, http: &H, query: &str) -> Result<SearchResults, H::Error> {
        let ask = |filter: &str| self.call(&WEB_REMIX, http, "search", json!({ "query": query, "params": filter }));
        let (s, al, ar) = join3(ask(SONGS), ask(ALBUMS), ask(ARTISTS)).await;
        let (s, al, ar) = (parse::search(&s?), parse::search(&al?), parse::search(&ar?));
        Ok(SearchResults { songs: s.songs, albums: al.albums, artists: ar.artists })
    }

    async fn browse<H: Http + Sync>(&self, http: &H, id: &str) -> Result<Value, H::Error> {
        self.call(&WEB_REMIX, http, "browse", json!({ "browseId": id })).await
    }

    /// Album `id` (`MPREb_...`) and its songs.
    pub async fn album<H: Http + Sync>(&self, http: &H, id: &str) -> Result<AlbumPage, H::Error> {
        Ok(parse::album(&self.browse(http, id).await?, id))
    }

    /// Artist `id` (`UC...`): albums, top songs, similar artists, description.
    pub async fn artist<H: Http + Sync>(&self, http: &H, id: &str) -> Result<ArtistPage, H::Error> {
        Ok(parse::artist(&self.browse(http, id).await?, id))
    }

    /// Playlist `id` (`VL...`) and its first page of songs.
    pub async fn playlist<H: Http + Sync>(&self, http: &H, id: &str) -> Result<PlaylistPage, H::Error> {
        Ok(parse::playlist(&self.browse(http, id).await?, id))
    }

    /// The home feed's shelves, its first page and the one after: the account's own when signed in,
    /// everyone's otherwise.
    pub async fn home<H: Http + Sync>(&self, http: &H) -> Result<Vec<Shelf>, H::Error> {
        let (mut shelves, next) = parse::feed(&self.browse(http, "FEmusic_home").await?);
        if let Some(token) = next {
            shelves.extend(parse::feed(&self.call(&WEB_REMIX, http, "browse", json!({ "continuation": token })).await?).0);
        }
        Ok(shelves)
    }

    /// A page of the account's library, signed in only.
    async fn library<H: Http + Sync>(&self, http: &H, id: &str) -> Result<Vec<Item>, H::Error> {
        if self.cookies().is_none() {
            return Err(Error::SignedOut);
        }
        Ok(parse::library(&self.browse(http, id).await?))
    }

    /// The playlists in the account's library ("Liked music" among them).
    pub async fn library_playlists<H: Http + Sync>(&self, http: &H) -> Result<Vec<Playlist>, H::Error> {
        let items = self.library(http, "FEmusic_liked_playlists").await?;
        Ok(items.into_iter().filter_map(|i| if let Item::Playlist(p) = i { Some(p) } else { None }).collect())
    }

    /// The albums saved to the account's library.
    pub async fn library_albums<H: Http + Sync>(&self, http: &H) -> Result<Vec<Album>, H::Error> {
        let items = self.library(http, "FEmusic_liked_albums").await?;
        Ok(items.into_iter().filter_map(|i| if let Item::Album(a) = i { Some(a) } else { None }).collect())
    }

    /// The songs the account liked, newest first (the first page of its "Liked music").
    pub async fn liked_songs<H: Http + Sync>(&self, http: &H) -> Result<Vec<Song>, H::Error> {
        if self.cookies().is_none() {
            return Err(Error::SignedOut);
        }
        Ok(self.playlist(http, "VLLM").await?.songs)
    }

    /// Likes `video` (`on`) or takes the like back.
    pub async fn like<H: Http + Sync>(&self, http: &H, video: &str, on: bool) -> Result<(), H::Error> {
        if self.cookies().is_none() {
            return Err(Error::SignedOut);
        }
        let endpoint = if on { "like/like" } else { "like/removelike" };
        self.call(&WEB_REMIX, http, endpoint, json!({ "target": { "videoId": video } })).await.map(|_| ())
    }

    /// The stream to play `video` from: the first visionOS build YouTube still serves, and its best
    /// audio ([`pick`]). Refused for want of a visitor id, it is asked once more with the one the
    /// refusal gave. Never signed in.
    pub async fn player<H: Http + Sync>(&self, http: &H, video: &str) -> Result<Stream, H::Error> {
        let fields = json!({ "videoId": video, "contentCheckOk": true, "racyCheckOk": true });
        let mut last = Error::Status(404);
        for client in &VISIONOS {
            let visitor_known = self.visitor().is_some();
            let answer = match self.call(client, http, "player", fields.clone()).await {
                Err(Error::Status(status)) if retired(status) => {
                    last = Error::Status(status);
                    continue;
                }
                Ok(v) if !visitor_known && self.visitor().is_some() && player::wants_visitor(&v) => self.call(client, http, "player", fields).await?,
                answer => answer?,
            };
            return player::stream(&answer).map_err(|e| match e {
                player::NoStream::Refused(reason) => Error::Unplayable(reason),
                player::NoStream::NoAudio => Error::Parse("no plain audio URL in the player answer".into()),
            });
        }
        Err(last)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Answers each request with the next queued answer and keeps what was asked.
    #[derive(Default)]
    pub struct Fake {
        answers: Mutex<Vec<Response>>,
        pub asked: Mutex<Vec<Request>>,
    }

    impl Fake {
        pub fn answering(answers: &[(u16, &str)]) -> Fake {
            Fake::with(answers.iter().map(|(s, b)| Response { status: *s, body: b.as_bytes().to_vec(), set_cookies: Vec::new() }).collect())
        }

        pub fn with(answers: Vec<Response>) -> Fake {
            Fake { answers: Mutex::new(answers.into_iter().rev().collect()), ..Default::default() }
        }

        pub fn asked(&self) -> Vec<Request> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl Http for Fake {
        type Error = ();

        fn send(&self, request: Request) -> impl Future<Output = std::result::Result<Response, ()>> + Send {
            self.asked.lock().unwrap().push(request);
            let answer = self.answers.lock().unwrap().pop().ok_or(());
            std::future::ready(answer)
        }
    }

    /// Runs a future whose requests are all answered at once.
    pub fn ready<F: Future>(f: F) -> F::Output {
        let mut f = std::pin::pin!(f);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        match f.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(v) => v,
            std::task::Poll::Pending => panic!("a fake request was left waiting"),
        }
    }

    pub fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    fn body(r: &Request) -> Value {
        serde_json::from_str(r.body.as_deref().unwrap()).unwrap()
    }

    fn ok(name: &str) -> Response {
        Response { status: 200, body: fixture(name).into_bytes(), set_cookies: Vec::new() }
    }

    fn status(status: u16, set_cookies: &[&str]) -> Response {
        Response { status, body: b"{}".to_vec(), set_cookies: set_cookies.iter().map(|c| c.to_string()).collect() }
    }

    const SESSION: &str = "SAPISID=sapi; __Secure-3PAPISID=sapi3; __Secure-1PSIDTS=ts1; SID=sid";

    fn signed_in() -> InnerTube {
        let yt = InnerTube::default();
        yt.set_cookies(Some(Cookies::parse(SESSION)));
        yt
    }

    /// A search asks songs, albums and artists each with its own filter, and keeps each kind from its own answer.
    #[test]
    fn search_asks_each_kind_with_its_filter() {
        let (s, al, ar) = (fixture("search_songs"), fixture("search_albums"), fixture("search_artists"));
        let fake = Fake::answering(&[(200, &s), (200, &al), (200, &ar)]);
        let found = ready(InnerTube::default().search(&fake, "daft punk")).unwrap();
        assert_eq!(found.songs.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Robot Rock (Edit)", "Da Funk", "Alive"]);
        assert_eq!(found.albums.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(), ["Random Access Memories", "Daft Punk Is Not Dead Yet", "The Answer is Daft Punk"]);
        assert_eq!(found.artists.len(), 3);
        let asked = fake.asked();
        let filters: Vec<Value> = asked.iter().map(|r| body(r)["params"].clone()).collect();
        assert_eq!(filters, [SONGS, ALBUMS, ARTISTS].map(Value::from));
        for r in &asked {
            assert_eq!(r.url, "https://music.youtube.com/youtubei/v1/search?prettyPrint=false");
            assert_eq!(body(r)["context"]["client"]["clientName"], "WEB_REMIX");
            assert_eq!(body(r)["query"], "daft punk");
            assert!(r.header("Cookie").is_none() && r.header("Authorization").is_none(), "signed out");
        }
    }

    /// The player is asked as visionOS 1.02, with its device and user agent, on www.youtube.com.
    #[test]
    fn player_asks_as_visionos() {
        let fake = Fake::answering(&[(200, &fixture("player"))]);
        let s = ready(InnerTube::default().player(&fake, "dQw4w9WgXcQ")).unwrap();
        assert_eq!(s.format.itag, 251);
        let r = &fake.asked()[0];
        assert_eq!(r.url, "https://www.youtube.com/youtubei/v1/player?prettyPrint=false");
        let c = &body(r)["context"]["client"];
        assert_eq!((c["clientName"].as_str(), c["clientVersion"].as_str(), c["deviceModel"].as_str()), (Some("VISIONOS"), Some("1.02"), Some("RealityDevice17,1")));
        assert_eq!(body(r)["videoId"], "dQw4w9WgXcQ");
        assert_eq!(r.header("x-youtube-client-name"), Some("101"));
        assert_eq!(r.header("User-Agent"), Some(VISIONOS_UA));
    }

    /// A build YouTube retired (400, 404) gives way to the next; any other failure is the answer.
    #[test]
    fn a_retired_visionos_build_gives_way_to_the_next() {
        let player = |fake: &Fake| ready(InnerTube::default().player(fake, "v"));
        let fake = Fake::answering(&[(400, "{}"), (200, &fixture("player"))]);
        assert!(player(&fake).is_ok());
        let versions: Vec<String> = fake.asked().iter().map(|r| r.header("X-YouTube-Client-Version").unwrap().to_string()).collect();
        assert_eq!(versions, ["1.02", "1.03"]);

        let fake = Fake::answering(&[(404, ""), (400, ""), (400, "")]);
        assert_eq!(player(&fake), Err(Error::Status(400)));
        assert_eq!(fake.asked().len(), 3);

        let fake = Fake::answering(&[(429, "")]);
        assert_eq!(player(&fake), Err(Error::Status(429)), "a rate limit is not retried");
        assert_eq!(fake.asked().len(), 1);
    }

    /// The first answer's visitor id goes with every later request; a player refusal for want of one is
    /// asked once more with it, a refusal with one known is the answer.
    #[test]
    fn the_visitor_id_is_learned_and_sent() {
        let yt = InnerTube::default();
        let fake = Fake::answering(&[(200, &fixture("not_a_bot")), (200, &fixture("player")), (200, &fixture("not_a_bot"))]);
        assert_eq!(ready(yt.player(&fake, "v")).unwrap().format.itag, 251);
        let asked = fake.asked();
        assert!(asked[0].header("X-Goog-Visitor-Id").is_none() && body(&asked[0])["context"]["client"].get("visitorData").is_none());
        assert_eq!(asked[1].header("X-Goog-Visitor-Id"), Some("CgtGSVhUVVJFX0lEKAA%3D"));
        assert_eq!(body(&asked[1])["context"]["client"]["visitorData"], "CgtGSVhUVVJFX0lEKAA%3D");
        assert_eq!(asked[1].header("X-YouTube-Client-Version"), Some("1.02"), "the same build again");
        match ready(yt.player(&fake, "w")) {
            Err(Error::Unplayable(reason)) => assert!(reason.contains("not a bot"), "{reason}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(fake.asked().len(), 3);
    }

    #[test]
    fn browse_asks_youtubes_id() {
        let fake = Fake::answering(&[(200, &fixture("album"))]);
        let page = ready(InnerTube::default().album(&fake, "MPREb_K8qWMWVqXGi")).unwrap();
        assert_eq!(page.album.id, "MPREb_K8qWMWVqXGi");
        assert_eq!(body(&fake.asked()[0])["browseId"], "MPREb_K8qWMWVqXGi");
    }

    /// The three searches are in flight together: none waits for another's answer.
    #[test]
    fn searches_go_out_side_by_side() {
        struct Held(Mutex<usize>);
        impl Http for Held {
            type Error = ();
            fn send(&self, _: Request) -> impl Future<Output = std::result::Result<Response, ()>> + Send {
                *self.0.lock().unwrap() += 1;
                std::future::pending()
            }
        }
        let held = Held(Mutex::new(0));
        let yt = InnerTube::default();
        let mut f = std::pin::pin!(yt.search(&held, "q"));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(f.as_mut().poll(&mut cx).is_pending());
        assert_eq!(*held.0.lock().unwrap(), 3);
    }

    /// The home feed is its first page and the one its continuation names, shelf by shelf.
    #[test]
    fn home_is_two_pages_of_shelves() {
        let fake = Fake::with(vec![ok("home"), ok("home_more")]);
        let shelves = ready(InnerTube::default().home(&fake)).unwrap();
        let titles: Vec<&str> = shelves.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Quick picks", "All hits", "Party music", "New releases"]);
        assert_eq!(body(&fake.asked()[0])["browseId"], "FEmusic_home");
        assert_eq!(body(&fake.asked()[1])["continuation"], "FIXTURE_CONTINUATION");
    }

    /// Signed in, music.youtube.com requests carry the session; streams never do.
    #[test]
    fn signed_in_requests_carry_the_session_and_streams_do_not() {
        let yt = signed_in();
        let fake = Fake::with(vec![ok("library_albums"), ok("player")]);
        assert_eq!(ready(yt.library_albums(&fake)).unwrap().len(), 2);
        ready(yt.player(&fake, "v")).unwrap();
        let asked = fake.asked();
        let r = &asked[0];
        assert_eq!(r.header("Cookie"), Some(SESSION));
        assert!(r.header("Authorization").is_some_and(|a| a.starts_with("SAPISIDHASH ") && a.contains(" SAPISID3PHASH ")));
        assert_eq!((r.header("X-Origin"), r.header("X-Goog-AuthUser"), r.header("User-Agent")), (Some("https://music.youtube.com"), Some("0"), Some(BROWSER_UA)));
        let p = &asked[1];
        assert!(p.header("Cookie").is_none() && p.header("Authorization").is_none());
    }

    /// The library, likes and the account's name need an account: nothing is asked without one.
    #[test]
    fn personal_calls_need_an_account() {
        let fake = Fake::default();
        let yt = InnerTube::default();
        assert_eq!(ready(yt.library_playlists(&fake)), Err(Error::SignedOut));
        assert_eq!(ready(yt.liked_songs(&fake)), Err(Error::SignedOut));
        assert_eq!(ready(yt.like(&fake, "v", true)), Err(Error::SignedOut));
        assert_eq!(ready(yt.account_name(&fake)), Err(Error::SignedOut));
        assert!(fake.asked().is_empty());
    }

    /// A rotation keeps the cookies it is given, for the caller to take once; music.youtube.com's own
    /// Set-Cookie values are kept too.
    #[test]
    fn rotated_cookies_are_kept_and_taken_once() {
        let yt = signed_in();
        let fake = Fake::with(vec![status(200, &["__Secure-1PSIDTS=ts2; Domain=.youtube.com; Secure"]), Response { set_cookies: vec!["SIDCC=cc; Domain=.youtube.com".into()], ..ok("account_menu") }]);
        assert_eq!(ready(yt.rotate(&fake)), Ok(Rotated::Fresh));
        let r = &fake.asked()[0];
        assert_eq!((r.url.as_str(), r.body.as_deref(), r.header("Origin")), (ROTATE_URL, Some(ROTATE_BODY), Some("https://accounts.youtube.com")));
        assert_eq!(r.header("Cookie"), Some(SESSION));
        assert_eq!(yt.take_changed().unwrap().get("__Secure-1PSIDTS"), Some("ts2"));
        assert_eq!(yt.take_changed(), None, "taken once");
        assert_eq!(ready(yt.account_name(&fake)).unwrap(), "Ann Example");
        assert_eq!(yt.take_changed().unwrap().get("SIDCC"), Some("cc"));
        assert_eq!(fake.asked()[1].header("Cookie"), Some("SAPISID=sapi; __Secure-3PAPISID=sapi3; __Secure-1PSIDTS=ts2; SID=sid"));
    }

    /// Asked too soon, a rotation changes nothing; a failure that is not a refusal keeps the session; a
    /// refusal ends it.
    #[test]
    fn only_a_refused_rotation_ends_the_session() {
        let yt = signed_in();
        let fake = Fake::with(vec![status(429, &[]), status(503, &[])]);
        assert_eq!(ready(yt.rotate(&fake)), Ok(Rotated::TooSoon));
        assert_eq!(ready(yt.rotate(&fake)), Err(Error::Status(503)));
        assert_eq!(ready(yt.rotate(&fake)), Err(Error::Transport(())), "no answer");
        assert!(yt.cookies().is_some(), "the session stands");
        let fake = Fake::with(vec![status(401, &[])]);
        assert_eq!(ready(yt.rotate(&fake)), Err(Error::SignedOut));
        assert_eq!(yt.cookies(), None);
    }

    /// A signed-in answer given as to a visitor is asked again after a rotation; given so again, the
    /// session has ended. A network failure is only that.
    #[test]
    fn a_refused_answer_rotates_and_asks_once_more() {
        let yt = signed_in();
        let fake = Fake::with(vec![ok("signed_out"), status(200, &["__Secure-1PSIDTS=ts2"]), ok("library_albums")]);
        assert_eq!(ready(yt.library_albums(&fake)).unwrap().len(), 2);
        assert_eq!(fake.asked().iter().map(|r| r.url.as_str()).collect::<Vec<_>>()[1], ROTATE_URL);
        assert!(fake.asked()[2].header("Cookie").unwrap().contains("ts2"), "asked again with the fresh cookies");

        let fake = Fake::with(vec![status(401, &[]), status(200, &[]), ok("signed_out")]);
        assert_eq!(ready(yt.library_albums(&fake)), Err(Error::SignedOut));
        assert_eq!(yt.cookies(), None);

        let yt = signed_in();
        let fake = Fake::default();
        assert_eq!(ready(yt.library_albums(&fake)), Err(Error::Transport(())));
        assert!(yt.cookies().is_some());
    }

    #[test]
    fn likes_name_the_video() {
        let yt = signed_in();
        let fake = Fake::answering(&[(200, "{}"), (200, "{}")]);
        ready(yt.like(&fake, "v1", true)).unwrap();
        ready(yt.like(&fake, "v1", false)).unwrap();
        let asked = fake.asked();
        assert!(asked[0].url.contains("/like/like?") && asked[1].url.contains("/like/removelike?"));
        assert_eq!(body(&asked[0])["target"]["videoId"], "v1");
    }

    /// The library's playlists, without the "New playlist" button; its albums.
    #[test]
    fn library_pages() {
        let fake = Fake::with(vec![ok("library_playlists"), ok("library_albums"), ok("playlist")]);
        let yt = signed_in();
        let lists = ready(yt.library_playlists(&fake)).unwrap();
        assert_eq!(lists.iter().map(|p| (p.id.as_str(), p.title.as_str())).collect::<Vec<_>>(), [("VLLM", "Liked music"), ("VLOLAK5uy_lNVBcjNtiCwq-n95uoxJ-Gd4vsfMkyZNs", "Top songs")]);
        let albums = ready(yt.library_albums(&fake)).unwrap();
        assert_eq!(albums.iter().map(|a| (a.title.as_str(), a.year, a.artists[0].id.as_str())).collect::<Vec<_>>(), [("Random Access Memories", 2013, "UCRr1xG_2WIDs18a6cIiCxeA"), ("Homework", 1997, "UCRr1xG_2WIDs18a6cIiCxeA")]);
        assert_eq!(ready(yt.liked_songs(&fake)).unwrap().len(), 3);
        assert_eq!(body(&fake.asked()[2])["browseId"], "VLLM");
    }
}
