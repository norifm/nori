//! Where a profile's music comes from, as a plug-in: a [`Provider`] answers the core's reads and writes,
//! says where songs and covers are, the rules of its profiles, and how one is added and signed in. The
//! rest of the core and every client go through it and never name one (docs/site/developers/providers.md).
//! Not octo-fiesta's provider items (`ext-` ids), which are a Subsonic server's.

pub(crate) mod subsonic;
#[cfg(feature = "youtube")]
mod youtube;
#[cfg(test)]
mod fake;
#[cfg(test)]
pub(crate) mod harness;

use std::sync::Arc;

pub use subsonic::login_check;

use crate::browse::ProfileRules;
use crate::cache_policy::{Page, Read};
use crate::client::{Client, NetResult, Write};
use crate::settings::{SavedServer, ServerKind, ServerList};
use crate::stream::{StreamQuality, StreamTarget};
use crate::transport::{NetError, Transport};
use crate::Core;

/// A kind of music source, and the state of the profile in use of it. One is made per profile opened
/// ([`Core::configure_profile`]); its methods get the client they work through.
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    /// What adding a profile asks for, and how its account signs in.
    fn info(&self) -> ProviderInfo;

    /// Whether the add form `profile` can be sent.
    fn ready(&self, profile: &SavedServer) -> bool;

    /// Checks `profile` before it is kept; the profile to keep.
    async fn check(&self, transport: Arc<dyn Transport>, profile: SavedServer) -> NetResult<SavedServer>;

    /// What the app offers over saved `profile`; `guest` when it is a jam guest's.
    fn rules(&self, profile: &SavedServer, guest: bool) -> ProfileRules;

    /// Points `core` at `profile`: what its library is kept under, its rules and its account.
    fn configure(&self, core: &Core, profile: &SavedServer) -> nori_model::Result<()>;

    /// Whether the profile has a Subsonic server for the core's account work (scrobbles, playlists,
    /// queue pushes, remote control). Without one, those requests fail at once.
    fn subsonic_api(&self) -> bool {
        false
    }

    /// `read` asked of the source now; None when it has nothing for such a read (the empty page). How
    /// its answer is kept, how long it is fresh and what the session lays over it are the core's
    /// (cache_policy.rs).
    async fn read(&self, client: &Client, read: Read) -> NetResult<Option<Answer>>;

    /// A raw answer of `read` ([`Answer::Raw`]) as its page.
    fn parse(&self, _client: &Client, read: &Read, _raw: &[u8]) -> NetResult<Page> {
        Err(NetError::Parse { reason: format!("no raw answer to {read:?}") })
    }

    /// What `read`'s answer depends on beyond the read (a server's music folders), part of its key.
    fn scope(&self, _client: &Client, _read: &Read) -> String {
        String::new()
    }

    /// Whether `e` says the item read is gone, so its kept page goes too.
    fn gone(&self, _e: &NetError) -> bool {
        false
    }

    /// Sends one change.
    async fn write(&self, client: &Client, w: Write) -> NetResult<()>;

    /// Where song `id` streams from at quality `q`, and the key its bytes are kept under.
    fn stream(&self, client: &Client, id: String, q: StreamQuality) -> StreamTarget;

    /// How many songs past the next one may be fetched ahead; None: as many as the settings say.
    fn ahead(&self) -> Option<usize> {
        None
    }

    /// The URL of cover `cover` at `size` px.
    fn cover_address(&self, core: &Core, cover: &str, size: u32) -> String;

    /// What every cover address starts with, the cover's id and size appended ([`crate::covers::cover_url_into`]);
    /// None when each is the provider's own.
    fn cover_prefix(&self, _core: &Core) -> Option<String> {
        None
    }

    /// What turns its stream addresses into the URLs they open at; None when they are URLs already.
    fn resolver(&self, _client: &Arc<Client>) -> Option<Arc<dyn ResolveStream>> {
        None
    }

    /// Whether a [`SignIn::WebPage`] showing `address`, holding `cookies` for it, is done.
    fn sign_in_done(&self, _address: &str, _cookies: &str) -> bool {
        false
    }

    /// Signs the profile's account in with `answer` (a sign-in page's cookies, or what was pasted).
    async fn sign_in(&self, _client: &Client, _answer: String) -> SignInResult {
        SignInResult::Refused
    }

    /// Signs the account out here.
    fn sign_out(&self, _client: &Client) {}

    /// Where the profile's account stands.
    fn account(&self, _client: &Client) -> AccountState {
        AccountState::SignedOut
    }

    /// The account's session ended and the client reminds to sign in again at start.
    fn reminds_to_sign_in(&self, _client: &Client) -> bool {
        false
    }

    /// No reminder until the account is signed in, and its session ends, again.
    fn stop_reminding(&self, _client: &Client) {}
}

/// A source's answer to a read.
pub enum Answer {
    /// Its page, kept in memory for the session.
    Page(Box<Page>),
    /// Its answer as it came, kept on disk and parsed again on every read ([`Provider::parse`]), so a
    /// page reads as the current parser makes it.
    Raw(Vec<u8>),
}

/// Turns a provider's stream addresses into the URLs they open at, on the opening thread (a loader's,
/// never the engine's).
pub trait ResolveStream: Send + Sync {
    /// The URL to open `address` at; None when `address` is a URL already.
    fn resolve(&self, address: &str) -> Option<NetResult<String>>;

    /// The URL `address` resolved to was refused: the next open resolves it anew.
    fn refused(&self, address: &str);

    /// Live stream `address` as one endless body, read on the reading thread; None when `address` is no
    /// live stream of the provider's.
    fn live(&self, _address: &str) -> Option<Box<dyn std::io::Read + Send>> {
        None
    }

    /// The URL to fetch `address` at from byte `from` to its end in one request, for a platform's download
    /// worker; None when `address` is a URL already.
    fn resolve_rest(&self, address: &str, _from: u64) -> Option<NetResult<String>> {
        self.resolve(address)
    }

    /// How `address` is asked for when its server sends a long range no faster than it plays: in parts,
    /// each a request of its own. None when one request may ask for the rest. Asked after
    /// [`ResolveStream::resolve`], on the same thread.
    fn parts(&self, _address: &str) -> Option<StreamParts> {
        None
    }
}

/// A stream asked for in parts ([`ResolveStream::parts`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamParts {
    /// The stream's length in bytes.
    pub len: u64,
    /// The most bytes one request asks for.
    pub most: u64,
}

/// What adding a profile of a kind asks for, and how its account signs in.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ProviderInfo {
    pub kind: ServerKind,
    /// The add form's fields, in order.
    pub fields: Vec<FieldSpec>,
    /// How the account signs in once the profile is added, the first offered right away; empty when
    /// the form is the sign-in.
    pub sign_in: Vec<SignIn>,
}

impl ProviderInfo {
    fn asks(&self, what: FormField) -> bool {
        self.fields.iter().any(|f| f.what == what)
    }
}

/// One field of an add form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct FieldSpec {
    pub what: FormField,
    /// Folded under the form's advanced part.
    pub advanced: bool,
}

/// What a form field fills in of a [`SavedServer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum FormField {
    Name,
    /// `url`; a profile with one is told apart from others of its kind by it and its `user`.
    Address,
    User,
    Password,
    /// `alt_url`
    SecondAddress,
    ApiKey,
    Headers,
    LegacyAuth,
    SelfSigned,
    WifiOnly,
    ClientCert,
}

/// A way to sign a profile's account in.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SignIn {
    /// A web page of the app's own opened at `start_url`; once [`sign_in_done`] says so, its cookies go
    /// to [`Client::sign_in`].
    WebPage { start_url: String },
    /// Text the user copies from elsewhere (a browser session's cookies), for [`Client::sign_in`].
    Paste,
}

/// Where a profile's account stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AccountState {
    SignedOut,
    SignedIn,
    /// The source ended the session: the account's library and likes wait until it is signed in again.
    Ended,
}

/// How a sign-in went.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SignInResult {
    /// Signed in to the account the source calls `name`.
    SignedIn { name: String },
    /// The answer holds no signed-in session.
    NoSession,
    /// The source does not take it: the session has ended, or it is another browser's.
    Refused,
    /// The source could not be asked; nothing changed.
    Unreachable { reason: String },
}

/// A provider for each kind this build has.
fn provider(kind: ServerKind) -> Option<Arc<dyn Provider>> {
    match kind {
        ServerKind::Subsonic => Some(Arc::new(subsonic::Subsonic)),
        #[cfg(feature = "youtube")]
        ServerKind::YouTubeMusic => Some(Arc::new(youtube::YouTube::default())),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// `kind`'s provider; a kind this build lacks reads as a Subsonic server.
fn or_subsonic(kind: ServerKind) -> Arc<dyn Provider> {
    provider(kind).unwrap_or_else(|| Arc::new(subsonic::Subsonic))
}

/// The kinds of profile this build can add, in the order they are offered.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn providers() -> Vec<ProviderInfo> {
    ServerKind::ALL.iter().filter_map(|k| provider(*k)).map(|p| p.info()).collect()
}

/// What the app offers over saved `profile`; `guest` when it is a jam guest's.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn rules_of(profile: SavedServer, guest: bool) -> ProfileRules {
    or_subsonic(profile.kind).rules(&profile, guest)
}

/// Whether the add form can be sent.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn server_ready(profile: SavedServer) -> bool {
    provider(profile.kind).is_some_and(|p| p.ready(&profile))
}

/// Whether a `kind` sign-in page showing `address`, holding `cookies` for it, is done.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn sign_in_done(kind: ServerKind, address: String, cookies: String) -> bool {
    provider(kind).is_some_and(|p| p.sign_in_done(&address, &cookies))
}

/// Checks `profile` before it is kept (a server is asked, through its second address too); the profile
/// to keep.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub async fn check_profile(transport: Arc<dyn Transport>, profile: SavedServer) -> NetResult<SavedServer> {
    match provider(profile.kind) {
        Some(p) => p.check(transport, profile).await,
        None => Err(nowhere("provider for this kind")),
    }
}

/// The profile a login saves. A new profile for one already saved takes over that profile's id (and its
/// library and downloads), music folder, bitrate cap and, if the form's is empty, name; what the form
/// does not ask (a signed-in account's name) stays the saved one's. A profile is the saved one of its
/// kind with the same address and user; without an address in the form, the kind's saved one. A
/// profile already saved is returned as it is.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn server_for_login(list: ServerList, profile: SavedServer) -> SavedServer {
    if list.servers.iter().any(|s| s.id == profile.id) {
        return profile;
    }
    let info = or_subsonic(profile.kind).info();
    let address = |url: &str| url.trim().trim_end_matches('/').to_ascii_lowercase();
    let same = |s: &&SavedServer| {
        s.kind == profile.kind && (!info.asks(FormField::Address) || (address(&s.url) == address(&profile.url) && s.user.trim().eq_ignore_ascii_case(profile.user.trim())))
    };
    match list.servers.iter().find(same) {
        Some(saved) => SavedServer {
            id: saved.id.clone(),
            name: if profile.name.trim().is_empty() { saved.name.clone() } else { profile.name },
            user: if info.asks(FormField::User) { profile.user } else { saved.user.clone() },
            music_folder_ids: saved.music_folder_ids.clone(),
            alt_max_bit_rate: saved.alt_max_bit_rate,
            ..profile
        },
        None => profile,
    }
}

/// Logs `form` in and keeps it as the active profile of `settings`: the profile saved already for it
/// takes it in ([`server_for_login`]), and its provider checks it ([`check_profile`]). The profile kept.
pub async fn log_in(transport: Arc<dyn Transport>, settings: &crate::settings_store::Settings, form: SavedServer) -> NetResult<SavedServer> {
    let draft = settings.prefs(|p| server_for_login(ServerList { servers: p.servers.clone(), active_server_id: p.active_server_id.clone() }, form));
    let profile = check_profile(transport, draft).await?;
    settings.put(profile_activated(settings.current().unwrap_or_default(), profile.clone()));
    Ok(profile)
}

/// `prefs` with `profile` the active profile, in place of the saved one with its id or added
/// ([`crate::settings::servers_activated`]).
pub fn profile_activated(mut prefs: crate::settings::StoredPrefs, profile: SavedServer) -> crate::settings::StoredPrefs {
    let list = ServerList { servers: std::mem::take(&mut prefs.servers), active_server_id: std::mem::take(&mut prefs.active_server_id) };
    let list = crate::settings::servers_activated(list, profile);
    (prefs.servers, prefs.active_server_id) = (list.servers, list.active_server_id);
    prefs
}

/// Every request a profile's provider has nowhere to send: not an I/O failure, so a write is dropped,
/// not queued.
pub(crate) fn nowhere(what: &str) -> NetError {
    NetError::Api { code: 0, reason: format!("this profile has no {what}") }
}

/// The provider a core has, and the kind it was made for.
pub(crate) struct Installed {
    pub(crate) kind: ServerKind,
    pub(crate) provider: Arc<dyn Provider>,
}

impl Default for Installed {
    fn default() -> Self {
        Installed { kind: ServerKind::Subsonic, provider: Arc::new(subsonic::Subsonic) }
    }
}

impl Core {
    /// The provider of the profile in use.
    pub(crate) fn provider(&self) -> Arc<dyn Provider> {
        self.provider.read().provider.clone()
    }

    /// Makes `provider` the profile's, for `kind`.
    pub(crate) fn install(&self, kind: ServerKind, provider: Arc<dyn Provider>) {
        *self.provider.write() = Installed { kind, provider };
    }

    /// Makes `kind`'s provider the profile's, unless it is already: its state (a session, streams
    /// resolved) lasts as long as the profile.
    pub(crate) fn install_kind(&self, kind: ServerKind) -> nori_model::Result<Arc<dyn Provider>> {
        if self.kind() != kind {
            let p = provider(kind).ok_or_else(|| crate::CoreError::Api { code: 0, reason: format!("this build has no provider for {kind:?}") })?;
            self.install(kind, p);
        }
        Ok(self.provider())
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// Points the core at saved `profile` through its kind's provider. Clears the library when the
    /// profile now names another server, user or kind.
    pub fn configure_profile(&self, profile: SavedServer) -> nori_model::Result<()> {
        *self.lan_jam.write() = nori_remote::lan::guest_secret(&profile.api_key).map(|secret| (profile.url.clone(), secret.into()));
        self.install_kind(profile.kind)?.configure(self, &profile)
    }

    /// What the profile plays from.
    pub fn kind(&self) -> ServerKind {
        self.provider.read().kind
    }

    /// What every cover address of the profile starts with, the cover's id and size appended
    /// ([`crate::covers::cover_rules`]); None when the core makes each one ([`Core::cover_address`]).
    pub fn cover_prefix(&self) -> Option<String> {
        self.provider().cover_prefix(self)
    }
}

impl Client {
    /// What opens the profile's stream addresses at the URLs they are served at; None when they are
    /// URLs already.
    pub fn stream_resolver(self: &Arc<Self>) -> Option<Arc<dyn ResolveStream>> {
        self.core.provider().resolver(self)
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Resolves a provider address for a platform's download worker. May block on the provider.
    pub fn resolve_stream_url(self: Arc<Self>, address: String, from: u64) -> NetResult<String> {
        match self.stream_resolver().and_then(|r| r.resolve_rest(&address, from)) {
            Some(url) => url,
            None => Ok(address),
        }
    }

    /// Signs the profile's account in with `answer` (a sign-in page's cookies, or what was pasted).
    /// Anything but success leaves the account as it was.
    pub async fn sign_in(&self, answer: String) -> SignInResult {
        self.core.provider().sign_in(self, answer).await
    }

    /// Signs the profile's account out here (a browser's session is not ended).
    pub fn sign_out(&self) {
        self.core.provider().sign_out(self)
    }

    /// Where the profile's account stands.
    pub fn account(&self) -> AccountState {
        self.core.provider().account(self)
    }

    /// The account's session ended and the client reminds to sign in again at start.
    pub fn reminds_to_sign_in(&self) -> bool {
        self.core.provider().reminds_to_sign_in(self)
    }

    /// No reminder until the account is signed in, and its session ends, again.
    pub fn stop_reminding(&self) {
        self.core.provider().stop_reminding(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(id: &str, kind: ServerKind, url: &str, user: &str) -> SavedServer {
        SavedServer { id: id.into(), kind, url: url.into(), user: user.into(), ..Default::default() }
    }

    /// A login for a saved address and user takes over the saved profile; another address, user or a
    /// saved profile edited stays as it is.
    #[test]
    fn a_login_finds_the_saved_server() {
        let kept = SavedServer { name: "Home".into(), music_folder_ids: vec!["3".into()], alt_max_bit_rate: 96, ..saved("a1", ServerKind::Subsonic, "https://music.example.com", "ann") };
        let list = ServerList { servers: vec![kept.clone()], active_server_id: "a1".into() };
        let form = saved("new", ServerKind::Subsonic, "HTTPS://music.example.com/ ", "Ann");
        let login = server_for_login(list.clone(), form.clone());
        assert_eq!((login.id.as_str(), login.name.as_str(), login.music_folder_ids.as_slice(), login.alt_max_bit_rate, login.user.as_str()), ("a1", "Home", &["3".to_string()][..], 96, "Ann"));
        let stranger = saved("s", ServerKind::Subsonic, "https://music.example.com", "bob");
        assert_eq!(server_for_login(list.clone(), stranger.clone()), stranger);
        let elsewhere = saved("e", ServerKind::Subsonic, "https://other.example.com", "ann");
        assert_eq!(server_for_login(list.clone(), elsewhere.clone()), elsewhere);
        let edited = SavedServer { url: "https://moved.example.com".into(), ..kept };
        assert_eq!(server_for_login(list, edited.clone()), edited);
    }

    /// A login to a saved server keeps that profile's library, music folder and bitrate cap, and makes it
    /// the active one.
    #[test]
    fn a_login_keeps_the_saved_profile_and_makes_it_active() {
        let settings = crate::settings_store::Settings::new();
        let mut prefs = settings.open("").unwrap();
        let kept = SavedServer { music_folder_ids: vec!["3".into()], alt_max_bit_rate: 96, ..saved("a1", ServerKind::Subsonic, "https://music.example.com", "ann") };
        prefs.servers = vec![kept, saved("b2", ServerKind::Subsonic, "https://other.example.com", "bob")];
        prefs.active_server_id = "b2".into();
        settings.put(prefs);
        let fake = Arc::new(crate::client::tests::Fake::default());
        for _ in 0..4 {
            fake.answer(r#"{"subsonic-response":{"status":"ok","version":"1.16.1","type":"navidrome"}}"#);
        }
        let form = SavedServer { password: "pw".into(), ..saved("new", ServerKind::Subsonic, "https://music.example.com", "ann") };
        let p = crate::client::tests::block(log_in(fake, &settings, form)).unwrap();
        assert_eq!((p.id.as_str(), p.music_folder_ids.as_slice(), p.alt_max_bit_rate, p.password.as_str()), ("a1", &["3".to_string()][..], 96, "pw"));
        let prefs = settings.current().unwrap();
        let ids: Vec<&str> = prefs.servers.iter().map(|s| s.id.as_str()).collect();
        assert_eq!((ids, prefs.active_server().map(|s| s.id.as_str())), (vec!["b2", "a1"], Some("a1")));
    }

    /// A server's form needs an address, and a user or an API key.
    #[test]
    fn a_server_form_is_ready_with_an_address_and_a_user_or_key() {
        let p = saved("p", ServerKind::Subsonic, "https://x", "");
        assert!(!server_ready(p.clone()), "a user or a key");
        assert!(server_ready(SavedServer { user: "u".into(), ..p.clone() }));
        assert!(server_ready(SavedServer { api_key: "k".into(), ..p.clone() }));
        assert!(!server_ready(SavedServer { url: "  ".into(), user: "u".into(), ..p }));
    }

    /// A kind this build lacks has no provider: it is not offered, its form is never ready, and a
    /// profile saved with it does not open.
    #[test]
    fn every_kind_offered_has_a_provider() {
        let offered: Vec<ServerKind> = providers().into_iter().map(|p| p.kind).collect();
        assert_eq!(offered[0], ServerKind::Subsonic);
        for kind in ServerKind::ALL {
            assert_eq!(offered.contains(&kind), provider(kind).is_some(), "{kind:?}");
            let core = Core::new(String::new(), "k".into(), Default::default()).unwrap();
            assert_eq!(core.configure_profile(saved("p", kind, "https://x", "u")).is_ok(), provider(kind).is_some(), "{kind:?}");
        }
    }
}
