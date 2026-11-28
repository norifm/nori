//! Where a profile's music comes from, as a plug-in: a [`Provider`] answers the core's reads and writes,
//! says where songs and covers are, the rules of its profiles, and how one is added and signed in. The
//! rest of the core and every client go through it and never name one (docs/developers/providers.md).
//! Not octo-fiesta's provider items (`ext-` ids), which are a Subsonic server's.

pub(crate) mod subsonic;
#[cfg(feature = "plugins")]
mod plugin;
#[cfg(feature = "plugins")]
mod plugged;
pub(crate) mod hosted;
pub mod local;
pub mod flow;
pub mod code;
#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
pub(crate) mod harness;

use std::sync::Arc;

pub use subsonic::login_check;

use crate::browse::ProfileRules;
use crate::cache_policy::{Page, Read};
use crate::client::{Client, NetResult, Write};
use crate::settings::{SavedServer, ServerKind, ServerList};
use crate::stream::{Stream, StreamQuality};
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

    /// Whether favorites its account can't keep are kept on this device (`ProfileRules::stars_here`).
    fn favorites_here(&self) -> bool {
        false
    }

    /// Whether the account open keeps favorites of `kind`. One it has nowhere for is kept on this
    /// device, where the provider keeps any ([`Provider::favorites_here`]).
    fn keeps_favorites(&self, _kind: crate::client::Starrable) -> bool {
        true
    }

    /// `read` asked of the source now; None when it has nothing for such a read (the empty page). How
    /// its answer is kept, how long it is fresh and what the session lays over it are the core's
    /// (cache_policy.rs).
    async fn read(&self, client: &Client, read: Read) -> NetResult<Option<Answer>>;

    /// [`Provider::read`], sending `parts` what the source shows of its answer before the whole comes
    /// (a long playlist's first page). The whole answer follows them.
    async fn read_in_parts(&self, client: &Client, read: Read, _parts: crate::cache_policy::Parts) -> NetResult<Option<Answer>> {
        self.read(client, read).await
    }

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

    /// Where song `id` streams from at quality `q`.
    fn stream(&self, client: &Client, id: String, q: StreamQuality) -> Stream;

    /// How many songs past the next one may be fetched ahead; None: as many as the settings say.
    fn ahead(&self) -> Option<usize> {
        None
    }

    /// What it holds: the listener's own files win over a service's catalog where a profile's sources
    /// have the same song ([`nori_library::merged`]).
    fn holds(&self) -> nori_library::merged::SourceKind {
        nori_library::merged::SourceKind::Service
    }

    /// The URL of cover `cover` at `size` px.
    fn cover_address(&self, core: &Core, cover: &str, size: u32) -> String;

    /// The sizes, px square, at which the image server of its covers that are addresses already keeps
    /// pictures ready ([`crate::covers::ready_size`]); empty when it makes any size.
    fn cover_sizes(&self) -> &[u32] {
        &[]
    }

    /// What every cover address starts with, the cover's id and size appended ([`crate::covers::cover_url_into`]);
    /// None when each is the provider's own.
    fn cover_prefix(&self, _core: &Core) -> Option<String> {
        None
    }

    /// The file on this device song `id` plays from, whatever the stream says; None for a song that streams.
    fn file(&self, _client: &Client, _id: &str) -> Option<local::SongFile> {
        None
    }

    /// Fills the offline index from the source: everything it lists, what it no longer lists dropped.
    /// What the index holds after.
    async fn sync(&self, _client: &Client) -> NetResult<crate::IngestStats> {
        Err(nowhere("library to sync"))
    }

    /// How far a [`Provider::sync`] running is; None when none is, or when it cannot say.
    fn sync_progress(&self) -> Option<local::SyncProgress> {
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

    /// A code to sign in with elsewhere ([`SignIn::Code`]).
    async fn start_code(&self, _client: &Client) -> NetResult<code::StartedCode> {
        Err(nowhere("sign-in code"))
    }

    /// How `code` stands; allowed, the account is signed in with it as [`Provider::sign_in`] would.
    async fn check_code(&self, _client: &Client, _code: &code::StartedCode) -> code::CodeCheck {
        code::CodeCheck::Expired
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

    /// The signed-in account's picture as it acts now, a cover as the profile's covers are.
    async fn account_picture(&self, _client: &Client) -> Option<String> {
        None
    }

    /// Those the signed-in account may act as ([`ProviderInfo::identities`]), the account's own first.
    async fn identities(&self, _client: &Client) -> NetResult<Vec<Identity>> {
        Ok(Vec::new())
    }

    /// Acts as [`Provider::identities`]' entry `index` from now on.
    async fn choose_identity(&self, _client: &Client, _index: u32) -> NetResult<()> {
        Err(nowhere("identity"))
    }
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
    /// What the signed-in account may act as besides itself ([`Client::identities`]); None when it is
    /// only itself.
    pub identities: Option<IdentityKind>,
}

/// What a signed-in account may act as besides itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum IdentityKind {
    /// A Google account's YouTube channels.
    Channel,
}

/// One the account may act as.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Identity {
    pub name: String,
    /// What tells it apart from one of the same name ("@handle").
    pub detail: Option<String>,
    /// Its picture, a cover as the profile's covers are.
    pub picture: Option<String>,
    /// The one the account acts as now.
    pub chosen: bool,
}

impl ProviderInfo {
    fn asks(&self, what: FormField) -> bool {
        self.fields.iter().any(|f| f.what == what)
    }

    /// The account signs in elsewhere (a page, or a code allowed in a browser), the first way offered.
    fn signs_in_elsewhere(&self) -> bool {
        matches!(self.sign_in.first(), Some(SignIn::WebPage { .. } | SignIn::Code))
    }

    /// Where its sign-in page starts, when it offers one.
    pub fn page(&self) -> Option<&str> {
        self.sign_in.iter().find_map(|s| match s {
            SignIn::WebPage { start_url } => Some(start_url.as_str()),
            SignIn::Paste | SignIn::Code => None,
        })
    }

    /// What opens the first way offered, when it is a page or a code.
    pub(crate) fn opens(&self) -> Option<flow::SignInEvent> {
        match self.sign_in.first()? {
            SignIn::WebPage { .. } => Some(flow::SignInEvent::Open { start: None }),
            SignIn::Code => Some(flow::SignInEvent::Code),
            SignIn::Paste => None,
        }
    }

    /// What choosing this kind on the first screen leads to. A name alone is nothing to ask, nor is what
    /// only a saved profile's form shows.
    pub fn first_step(&self) -> FirstStep {
        let asks = self.fields.iter().any(|f| !f.advanced && f.what != FormField::Name && f.what.asked_adding());
        match (asks, self.signs_in_elsewhere()) {
            (true, _) => FirstStep::Form,
            (false, true) => FirstStep::Account,
            (false, false) => FirstStep::AddNow,
        }
    }

    /// What a profile of this kind just added goes on to about its account.
    pub(crate) fn after_adding(&self, after: AfterAdding) -> Option<SignInStep> {
        if !self.signs_in_elsewhere() {
            return None;
        }
        Some(match after {
            AfterAdding::Remind => SignInStep::Offer,
            AfterAdding::SignIn => SignInStep::Open,
            AfterAdding::Guest => SignInStep::StopReminding,
        })
    }
}

/// What choosing a kind of profile on the first screen leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum FirstStep {
    /// Its form, which has fields to fill.
    Form,
    /// No fields but a sign-in page or code: its page, to sign in or go on as a guest.
    Account,
    /// Nothing to ask: added at once; its page shows only if that fails.
    AddNow,
}

/// How a new profile was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AfterAdding {
    /// From its form: the sign-in is offered.
    #[default]
    Remind,
    /// With the page's sign-in button.
    SignIn,
    /// With the page's guest button.
    Guest,
}

/// What follows about the account of a profile just added ([`flow::SignInEvent`] says it to the client).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignInStep {
    /// Offers signing in, which can be skipped.
    Offer,
    /// Opens the first way to sign in ([`ProviderInfo::opens`]).
    Open,
    /// `Client::stop_reminding`.
    StopReminding,
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
    /// `proxy`: its address, user and password, asked together in a sheet of their own that the form's
    /// entry opens (`nori_settings::proxy::proxy_from_form`). Every kind's ([`PROXY_FIELD`]), as all of
    /// a profile's connections are nori's.
    Proxy,
    /// `folders`: where a local profile's music is.
    Folders,
}

impl FormField {
    /// Whether a form adding a profile shows it: a local profile starts from the platform's own folders,
    /// and its saved form changes them.
    fn asked_adding(self) -> bool {
        self != FormField::Folders
    }
}

/// The proxy's entry, under every kind's advanced part.
pub(crate) const PROXY_FIELD: FieldSpec = FieldSpec { what: FormField::Proxy, advanced: true };

/// Whether `profile` has a value in form field `what`: the form opens its advanced part on a saved profile
/// with one there.
fn field_set(what: FormField, profile: &SavedServer) -> bool {
    match what {
        FormField::Name => !profile.name.trim().is_empty(),
        FormField::Address => !profile.url.trim().is_empty(),
        FormField::User => !profile.user.trim().is_empty(),
        FormField::Password => !profile.password.is_empty(),
        FormField::SecondAddress => !profile.alt_url.trim().is_empty(),
        FormField::ApiKey => !profile.api_key.trim().is_empty(),
        FormField::Headers => !profile.headers.is_empty(),
        FormField::LegacyAuth => profile.legacy_auth,
        FormField::SelfSigned => profile.allow_self_signed,
        FormField::WifiOnly => profile.wifi_only,
        FormField::ClientCert => !profile.client_cert.is_empty(),
        FormField::Proxy => profile.proxy.is_some(),
        FormField::Folders => !profile.folders.is_empty(),
    }
}

/// What a client runs on, as far as which of a form's fields it can honor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Platform {
    /// Android: it tells a metered network from others.
    Phone,
    /// The desktop and terminal clients: every network is the same to them (their transport's
    /// `network` is always unmetered).
    Computer,
    /// The iPod touch, which has only Wi-Fi.
    Ipod,
}

impl Platform {
    /// Whether a value in `what` does anything here: Wi-Fi only needs a metered network to keep off.
    fn honors(self, what: FormField) -> bool {
        what != FormField::WifiOnly || self == Platform::Phone
    }
}

/// The fields of `info`'s form on `platform`, in its order, as they show for `profile` as typed: those the
/// platform honors; a user and password step aside while an API key is given; a saved profile (`adding`
/// false) is renamed from its menu, not on its form, and only its form changes a local profile's folders.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn form_fields(info: &ProviderInfo, platform: Platform, profile: &SavedServer, adding: bool) -> Vec<FieldSpec> {
    let keyed = field_set(FormField::ApiKey, profile) && info.asks(FormField::ApiKey);
    let shown = |what: FormField| match what {
        FormField::User | FormField::Password => !keyed,
        FormField::Name => adding,
        what => what.asked_adding() || !adding,
    };
    info.fields.iter().copied().filter(|f| platform.honors(f.what) && shown(f.what)).collect()
}

/// What a form of `fields` saves over `base` (the profile it edits, or a new one): each field's value as
/// `typed`; what the form doesn't show stays as `base` has it.
pub fn form_taken(base: SavedServer, typed: &SavedServer, fields: &[FieldSpec]) -> SavedServer {
    let mut p = base;
    for f in fields {
        match f.what {
            FormField::Name => p.name = typed.name.clone(),
            FormField::Address => p.url = typed.url.clone(),
            FormField::User => p.user = typed.user.clone(),
            FormField::Password => p.password = typed.password.clone(),
            FormField::SecondAddress => p.alt_url = typed.alt_url.clone(),
            FormField::ApiKey => p.api_key = typed.api_key.clone(),
            FormField::Headers => p.headers = typed.headers.clone(),
            FormField::LegacyAuth => p.legacy_auth = typed.legacy_auth,
            FormField::SelfSigned => p.allow_self_signed = typed.allow_self_signed,
            FormField::WifiOnly => p.wifi_only = typed.wifi_only,
            FormField::ClientCert => (p.client_cert, p.client_cert_password) = (typed.client_cert.clone(), typed.client_cert_password.clone()),
            FormField::Proxy => p.proxy = typed.proxy.clone(),
            FormField::Folders => p.folders = typed.folders.clone(),
        }
    }
    p
}

/// Whether a form of `fields` ([`form_fields`]) opens with its advanced part shown: one of them has a value
/// in `profile`.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn advanced_shown(fields: &[FieldSpec], profile: &SavedServer) -> bool {
    fields.iter().any(|f| f.advanced && field_set(f.what, profile))
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
    /// A code the user allows in a browser of their own, where they are signed in already
    /// ([`code::CodeSignIn`]).
    Code,
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

/// What a saved profile of a plugin's kind waits for before it opens here ([`Kinds::waits`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ProfileWait {
    /// Its plugin installed.
    Install,
    /// Its plugin, built for another nori's plugin API, updated to a version this one runs (looked for
    /// at once: [`nori_plugins::Plugins::updates_due`]).
    Update,
    /// Its plugin, installed, turned on.
    TurnOn,
    /// The switch over every plugin turned back on.
    PluginsOn,
    /// A client that runs no plugins: it never opens here.
    NoPlugins,
}

/// The kinds of profile this app opens: a Subsonic server's, and the providers of its enabled plugins.
/// A client makes one ([`Kinds::builtin`] where it has no plugins) and asks it about profiles to add;
/// each [`Core`] it opens uses it.
#[derive(Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct Kinds {
    #[cfg(feature = "plugins")]
    plugins: Option<Arc<nori_plugins::Plugins>>,
    /// The device's music files; without them no local profile is offered or opens.
    files: parking_lot::RwLock<Option<Arc<dyn local::Files>>>,
}

#[cfg(feature = "plugins")]
#[cfg_attr(feature = "ffi", uniffi::export)]
impl Kinds {
    /// nori's own kinds and those of `plugins`' enabled providers.
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn with_plugins(plugins: Arc<nori_plugins::Plugins>) -> Arc<Kinds> {
        Arc::new(Kinds { plugins: Some(plugins), files: Default::default() })
    }
}

/// Plays told to the plugins' scrobblers.
#[cfg(feature = "plugins")]
struct PluginScrobbles(Arc<nori_plugins::Plugins>);

#[cfg(feature = "plugins")]
impl nori_queue::scrobble::Scrobbles for PluginScrobbles {
    fn now_playing(&self, song: &nori_model::Song) {
        self.0.scrobblers_now_playing(song);
    }
    fn scrobble(&self, song: &nori_model::Song, at_ms: i64) {
        self.0.scrobblers_scrobble(song, at_ms);
    }
}

impl Kinds {
    /// The device's music files, which local profiles read: local profiles are offered from now on.
    pub fn set_files(&self, files: Arc<dyn local::Files>) {
        *self.files.write() = Some(files);
    }

    /// The enabled plugins' lyrics services; none without plugins.
    pub(crate) fn lyrics_services(&self) -> Vec<Arc<dyn nori_lyrics::race::PluginLyrics>> {
        #[cfg(feature = "plugins")]
        if let Some(p) = &self.plugins {
            return p.lyrics_services();
        }
        Vec::new()
    }

    /// Where plays go besides the profile's server: the plugins' scrobblers; None without plugins.
    pub(crate) fn scrobbles(&self) -> Option<Arc<dyn nori_queue::scrobble::Scrobbles>> {
        #[cfg(feature = "plugins")]
        if let Some(p) = &self.plugins {
            return Some(Arc::new(PluginScrobbles(p.clone())));
        }
        None
    }

    /// A provider for `kind`; None for a plugin not installed and enabled here.
    pub(crate) fn provider(&self, kind: &ServerKind) -> Option<Arc<dyn Provider>> {
        match kind {
            ServerKind::Subsonic => Some(Arc::new(subsonic::Subsonic)),
            #[cfg(feature = "plugins")]
            ServerKind::Plugin { id } => Some(Arc::new(plugin::PluginProvider::new(id, self.plugins.clone()?)?)),
            #[cfg(not(feature = "plugins"))]
            ServerKind::Plugin { .. } => None,
            ServerKind::Local => Some(Arc::new(local::Local::new(self.files.read().clone()?))),
        }
    }

    /// Where its enabled plugins decide in place of nori; None without plugins.
    pub(crate) fn deciders(&self) -> Option<Arc<dyn nori_model::deciders::Deciders>> {
        #[cfg(feature = "plugins")]
        return self.plugins.clone().map(|p| p as Arc<dyn nori_model::deciders::Deciders>);
        #[cfg(not(feature = "plugins"))]
        None
    }

    /// `kind`'s provider; a kind missing here reads as a Subsonic server.
    fn or_subsonic(&self, kind: &ServerKind) -> Arc<dyn Provider> {
        self.provider(kind).unwrap_or_else(|| Arc::new(subsonic::Subsonic))
    }

    /// The kinds a profile can be of here, a Subsonic server's first.
    fn kinds(&self) -> Vec<ServerKind> {
        #[cfg(feature = "plugins")]
        let plugins: Vec<ServerKind> = self.plugins.iter().flat_map(|p| p.running()).filter(|i| i.manifest.extends("provider")).map(|i| ServerKind::Plugin { id: i.manifest.id.clone() }).collect();
        #[cfg(not(feature = "plugins"))]
        let plugins = Vec::new();
        let local = self.files.read().is_some().then_some(ServerKind::Local);
        std::iter::once(ServerKind::Subsonic).chain(local).chain(plugins).collect()
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Kinds {
    /// nori's own kinds alone: for a client without plugins.
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn builtin() -> Arc<Kinds> {
        Arc::default()
    }

    /// The open profile's transport, which plugins' HTTP goes through.
    pub fn set_transport(&self, transport: Arc<dyn Transport>) {
        #[cfg(feature = "plugins")]
        if let Some(p) = &self.plugins {
            p.set_transport(Some(transport));
        }
        #[cfg(not(feature = "plugins"))]
        let _ = transport;
    }

    /// The device's media library, which local profiles read (Android's): local profiles are offered from
    /// now on.
    pub fn set_device_music(&self, music: Arc<dyn local::DeviceMusic>) {
        self.set_files(Arc::new(local::Listed(music)));
    }

    /// The kinds of profile that can be added, in the order they are offered.
    pub fn providers(&self) -> Vec<ProviderInfo> {
        self.kinds().iter().filter_map(|k| self.provider(k)).map(|p| p.info()).collect()
    }

    /// What the app offers over saved `profile`; `guest` when it is a jam guest's.
    pub fn rules_of(&self, profile: SavedServer, guest: bool) -> ProfileRules {
        self.or_subsonic(&profile.kind).rules(&profile, guest)
    }

    /// What saved `profile` waits for before it opens here; None for one that opens. A LAN jam's guest
    /// profile opens through its host, whatever its kind.
    pub fn waits(&self, profile: SavedServer) -> Option<ProfileWait> {
        let ServerKind::Plugin { id } = profile.kind else { return None };
        if nori_remote::lan::guest_secret(&profile.api_key).is_some() {
            return None;
        }
        #[cfg(feature = "plugins")]
        if let Some(p) = &self.plugins {
            let has = |list: Vec<nori_plugins::Installed>| list.iter().any(|i| i.manifest.id == id);
            return match p.installed().into_iter().find(|i| i.manifest.id == id) {
                None => Some(ProfileWait::Install),
                Some(i) if !i.runs_here() => Some(ProfileWait::Update),
                Some(_) if has(p.running()) => None,
                Some(_) if p.all_off() => Some(ProfileWait::PluginsOn),
                Some(_) => Some(ProfileWait::TurnOn),
            };
        }
        let _ = id;
        Some(ProfileWait::NoPlugins)
    }

    /// Whether the add form can be sent.
    pub fn server_ready(&self, profile: SavedServer) -> bool {
        self.provider(&profile.kind).is_some_and(|p| p.ready(&profile))
    }

    /// Whether a `kind` sign-in page showing `address`, holding `cookies` for it, is done.
    pub fn sign_in_done(&self, kind: ServerKind, address: String, cookies: String) -> bool {
        self.provider(&kind).is_some_and(|p| p.sign_in_done(&address, &cookies))
    }

    /// A plugin kind's name in `lang`, from the plugin's words; None for nori's own kinds, which each
    /// client words itself.
    pub fn plugin_name(&self, kind: ServerKind, lang: String) -> Option<String> {
        #[cfg(feature = "plugins")]
        if let (ServerKind::Plugin { id }, Some(p)) = (&kind, &self.plugins) {
            return Some(p.word(id.clone(), "name".into(), lang).unwrap_or_else(|| id.clone()));
        }
        let _ = (kind, lang);
        None
    }

    /// Checks `profile` before it is kept (a server is asked, through its second address too); the
    /// profile to keep.
    pub async fn check_profile(&self, transport: Arc<dyn Transport>, profile: SavedServer) -> NetResult<SavedServer> {
        match self.provider(&profile.kind) {
            Some(p) => p.check(transport, profile).await,
            None => Err(nowhere("provider for this kind")),
        }
    }

    /// The profile a login saves ([`server_for_login`]).
    pub fn login_profile(&self, list: ServerList, profile: SavedServer) -> SavedServer {
        server_for_login(&self.or_subsonic(&profile.kind).info(), list, profile)
    }

    /// Logging in with `profile` adds one to `list`, rather than logging in to one saved.
    pub fn adds_profile(&self, list: ServerList, profile: SavedServer) -> bool {
        let id = self.login_profile(list.clone(), profile).id;
        !list.servers.iter().any(|s| s.id == id)
    }
}

/// The profile a login saves. A login saves what its form holds over the saved profile it is
/// ([`over_saved`]): the saved one with its id, else the saved one of its kind with the same address and
/// user, which a new profile for that account takes over, keeping the saved one's name if the form's is
/// empty. A kind whose form asks no address has nothing to tell its profiles apart by, so each new
/// profile is one more (an account each).
fn server_for_login(info: &ProviderInfo, list: ServerList, profile: SavedServer) -> SavedServer {
    if let Some(saved) = list.servers.iter().find(|s| s.id == profile.id) {
        return over_saved(info, saved, profile);
    }
    let address = |url: &str| url.trim().trim_end_matches('/').to_ascii_lowercase();
    let same = |s: &&SavedServer| {
        s.kind == profile.kind && info.asks(FormField::Address) && address(&s.url) == address(&profile.url) && s.user.trim().eq_ignore_ascii_case(profile.user.trim())
    };
    match list.servers.iter().find(same) {
        Some(saved) => {
            let name = if profile.name.trim().is_empty() { saved.name.clone() } else { profile.name.clone() };
            SavedServer { name, ..over_saved(info, saved, profile) }
        }
        None => profile,
    }
}

/// `form` saved over profile `saved`. What the profile keeps besides its form stays as saved, as other
/// pages may have changed it while the form was open: its id (and so its library and downloads), music
/// folders, bitrate cap, icon, account (its user, when the form does not ask it, and picture),
/// scrobbling and sources.
fn over_saved(info: &ProviderInfo, saved: &SavedServer, form: SavedServer) -> SavedServer {
    SavedServer {
        id: saved.id.clone(),
        user: if info.asks(FormField::User) { form.user } else { saved.user.clone() },
        music_folder_ids: saved.music_folder_ids.clone(),
        alt_max_bit_rate: saved.alt_max_bit_rate,
        icon: saved.icon,
        picture: saved.picture.clone(),
        scrobble_off: saved.scrobble_off,
        scrobblers: saved.scrobblers.clone(),
        sources: saved.sources.clone(),
        ..form
    }
}

/// Logs `form` in and keeps it as the active profile of `settings`: the profile saved already for it
/// takes it in ([`server_for_login`]), and its provider checks it ([`Kinds::check_profile`]). The
/// profile kept.
pub async fn log_in(kinds: &Kinds, transport: Arc<dyn Transport>, settings: &crate::settings_store::Settings, form: SavedServer) -> NetResult<SavedServer> {
    let draft = settings.prefs(|p| kinds.login_profile(ServerList { servers: p.servers.clone(), active_server_id: p.active_server_id.clone() }, form));
    let profile = kinds.check_profile(transport, draft).await?;
    let activated = profile_activated(settings.current().unwrap_or_default(), profile.clone());
    let opened = activated.active_server().cloned().unwrap_or(profile);
    settings.put(activated);
    Ok(opened)
}

/// `prefs` with `profile` the active profile, in place of the saved one with its id or added
/// ([`crate::settings::servers_activated`]). A jam's guest profile lasts only while it is the one open.
pub fn profile_activated(mut prefs: crate::settings::StoredPrefs, profile: SavedServer) -> crate::settings::StoredPrefs {
    prefs.servers.retain(|s| s.id == profile.id || !nori_remote::is_guest_key(&s.api_key));
    let list = ServerList { servers: std::mem::take(&mut prefs.servers), active_server_id: std::mem::take(&mut prefs.active_server_id) };
    let list = crate::settings::servers_activated(list, profile);
    (prefs.servers, prefs.active_server_id) = (list.servers, list.active_server_id);
    prefs
}

/// The saved profiles `profile` can take as a source: each standing alone (no profile's source, with no
/// sources of its own), not itself, not a jam's guest profile.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn sources_offered(servers: Vec<SavedServer>, profile: SavedServer) -> Vec<SavedServer> {
    let alone = crate::settings::servers_shown(servers);
    alone.into_iter().filter(|s| s.id != profile.id && s.sources.is_empty() && !profile.sources.contains(&s.id) && !nori_remote::is_guest_key(&s.api_key)).collect()
}

/// Every request a profile's provider has nowhere to send: not an I/O failure, so a write is dropped,
/// not queued.
pub fn nowhere(what: &str) -> NetError {
    NetError::Api { code: 0, reason: format!("this profile has no {what}") }
}

/// A cover's URL at `size` px (`{size}` in it filled in).
pub(crate) fn sized(cover: &str, size: u32) -> String {
    crate::covers::own_address(cover, size).unwrap_or_else(|| cover.to_string())
}

/// Where song `id` of a provider whose songs resolve where they open streams from at quality `q`: an
/// address of `scheme`, its bit rate and the song; the bytes are kept by song and bit rate.
pub(crate) fn resolved_stream(scheme: &str, id: &str, q: &StreamQuality) -> Stream {
    Stream { url: format!("{scheme}{}/{id}", q.bit_rate), quality: q.bit_rate.to_string() }
}

/// The bit rate (0: as it is) and song of a [`resolved_stream`] address of `scheme`.
pub(crate) fn resolved_song(scheme: &str, address: &str) -> Option<(u32, String)> {
    let (kbps, id) = address.strip_prefix(scheme)?.split_once('/')?;
    Some((kbps.parse().ok()?, id.to_string()))
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
    /// What every cover address of this core's source starts with, the cover's id and size appended
    /// ([`crate::covers::cover_rules`]); None when the core makes each one ([`Core::cover_address`]).
    pub(crate) fn cover_prefix(&self) -> Option<String> {
        self.provider().cover_prefix(self)
    }

    /// Remote control and jams go over the LAN alone, never a server's relay: the profile has no
    /// Subsonic server ([`Provider::subsonic_api`]), or it plays from several sources, which no one
    /// server has.
    pub(crate) fn lan_only(&self) -> bool {
        !self.provider().subsonic_api() || !self.others().is_empty()
    }

    /// The provider of the profile in use.
    pub(crate) fn provider(&self) -> Arc<dyn Provider> {
        self.provider.read().provider.clone()
    }

    /// Makes `provider` the profile's, for `kind`.
    pub(crate) fn install(&self, kind: ServerKind, provider: Arc<dyn Provider>) {
        *self.provider.write() = Installed { kind, provider };
    }

    /// Makes `kind`'s provider the profile's, unless it is already: its state (a session, streams
    /// resolved) lasts as long as the profile. A LAN jam host's answering for `kind` is not it.
    pub(crate) fn install_kind(&self, kind: ServerKind) -> nori_model::Result<Arc<dyn Provider>> {
        let installed = self.provider.read().kind == kind && self.lan_jam.read().is_none();
        if !installed {
            let p = self.kinds.read().provider(&kind).ok_or_else(|| crate::CoreError::Api { code: 0, reason: format!("no provider here for {}", kind.id()) })?;
            self.install(kind, p);
        }
        Ok(self.provider())
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// Points the core at saved `profile` through its kind's provider; a LAN jam's guest profile, of
    /// whatever kind, through its host ([`hosted::HostedProvider`]). Clears the library when the profile
    /// now names another server, user or kind.
    pub fn configure_profile(&self, profile: SavedServer) -> nori_model::Result<()> {
        let provider = match nori_remote::lan::guest_secret(&profile.api_key) {
            Some(secret) => {
                *self.lan_jam.write() = Some((profile.url.clone(), secret.to_string()));
                let provider: Arc<dyn Provider> = Arc::new(hosted::HostedProvider { kind: profile.kind.clone() });
                *self.provider.write() = Installed { kind: profile.kind.clone(), provider: provider.clone() };
                provider
            }
            None => {
                // Installed while the jam's credential still says whether the host's provider is in.
                let provider = self.install_kind(profile.kind.clone())?;
                *self.lan_jam.write() = None;
                provider
            }
        };
        provider.configure(self, &profile)?;
        self.load_favorites_here()
    }

    /// What the profile plays from.
    pub fn kind(&self) -> ServerKind {
        self.provider.read().kind.clone()
    }

    /// The kinds of profile it opens: [`Kinds::builtin`] until a client with plugins gives it its own.
    pub fn use_kinds(&self, kinds: Arc<Kinds>) {
        #[cfg(feature = "plugins")]
        if let Some(p) = &kinds.plugins {
            // What plugins are handed of a song is the profile's record of it.
            let session = Arc::downgrade(&self.session);
            p.set_songs(Arc::new(move |id: &nori_model::ItemKey| session.upgrade()?.song(id)));
            self.session.planner.set_mixing(Some(p.mixing()));
        }
        self.session.use_deciders(kinds.deciders());
        self.session.scrobble_to(kinds.scrobbles());
        *self.kinds.write() = kinds;
    }

}

impl Client {
    /// The file on this device song `id` plays from; None for a song that streams.
    pub fn file_of(&self, id: &crate::ItemKey) -> Option<local::SongFile> {
        match &id.source {
            None => self.core.provider().file(self, &id.id),
            Some(source) => {
                let c = self.core.source(source)?;
                c.core.provider().file(&c, &id.id)
            }
        }
    }

    /// What opens the profile's stream addresses at the URLs they are served at; None when they are
    /// URLs already.
    pub fn stream_resolver(self: &Arc<Self>) -> Option<Arc<dyn ResolveStream>> {
        self.resolvers()
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

    /// Fills the offline index from the profile's source (and any other indexed sources it merges); what it holds after.
    pub async fn sync_library(&self) -> NetResult<crate::IngestStats> {
        let others = self.core.others();
        if others.is_empty() {
            return self.core.provider().sync(self).await;
        }
        let mut total = crate::IngestStats { artists: 0, albums: 0, songs: 0 };
        let mut synced = false;
        let mut err = None;
        let add = |total: &mut crate::IngestStats, s: crate::IngestStats| {
            total.artists += s.artists;
            total.albums += s.albums;
            total.songs += s.songs;
        };
        let own_rules = self.core.rules();
        if own_rules.indexed || own_rules.rescans {
            match self.core.provider().sync(self).await {
                Ok(s) => {
                    add(&mut total, s);
                    synced = true;
                }
                Err(e) => err = Some(e),
            }
        }
        for (_, c) in others {
            let r = c.core.rules();
            if r.indexed || r.rescans {
                match c.core.provider().sync(&c).await {
                    Ok(s) => {
                        add(&mut total, s);
                        synced = true;
                    }
                    Err(e) => {
                        err.get_or_insert(e);
                    }
                }
            }
        }
        match (synced, err) {
            (true, _) => Ok(total),
            (false, Some(e)) => Err(e),
            (false, None) => self.core.provider().sync(self).await,
        }
    }

    /// How far the library's sync running is ([`Client::sync_library`]); None when none runs, or it
    /// cannot say.
    pub fn sync_progress(&self) -> Option<local::SyncProgress> {
        self.core.provider().sync_progress().or_else(|| self.core.others().into_iter().find_map(|(_, c)| c.sync_progress()))
    }

    /// Signs the profile's account in with `answer` (a sign-in page's cookies, or what was pasted).
    /// Anything but success leaves the account as it was.
    pub async fn sign_in(&self, answer: String) -> SignInResult {
        self.core.provider().sign_in(self, answer).await
    }

    /// Takes the plugins back from `apart`, the client of an account opened apart from this profile (even
    /// one since removed): their requests go through this client's transport again, and a plugin both
    /// play from serves `profile` (this client's) again.
    pub fn take_plugins_back(&self, profile: SavedServer, apart: Arc<Client>) -> nori_model::Result<()> {
        self.core.kinds.read().set_transport(self.transport.clone());
        match apart.core.kind() == profile.kind {
            true => self.core.configure_profile(profile),
            false => Ok(()),
        }
    }

    /// Signs the profile's account out here (a browser's session is not ended); the account's pages saved
    /// on disk go.
    pub fn sign_out(&self) {
        self.core.provider().sign_out(self);
        if let Err(e) = self.core.saved_drop(self.home_saved_key()) {
            crate::alog::info(&format!("sign out: the saved pages could not be dropped: {e}"));
        }
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

    /// The signed-in account's picture as it acts now (its chosen identity's), for the profile to keep
    /// ([`crate::settings::SavedServer::picture`]); None while signed out or when it has none.
    pub async fn account_picture(&self) -> Option<String> {
        self.core.provider().account_picture(self).await
    }

    /// Those the signed-in account may act as, the account's own first, the one it acts as chosen.
    pub async fn identities(&self) -> NetResult<Vec<Identity>> {
        self.core.provider().identities(self).await
    }

    /// Acts as [`Client::identities`]' entry `index` from now on. What was kept of the library and the
    /// session's hearts were another's, so they go; the rules are asked anew.
    pub async fn choose_identity(&self, index: u32) -> NetResult<()> {
        self.core.provider().choose_identity(self, index).await?;
        self.core.cache_evict(String::new())?;
        self.core.forget_marks();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(fields: Vec<FieldSpec>, sign_in: Vec<SignIn>) -> ProviderInfo {
        ProviderInfo { kind: ServerKind::Subsonic, fields, sign_in, identities: None }
    }

    fn web() -> SignIn {
        SignIn::WebPage { start_url: "https://a.example".into() }
    }

    #[test]
    fn the_first_step_follows_the_fields_and_the_sign_in() {
        let field = |what, advanced| FieldSpec { what, advanced };
        let (address, name) = (FormField::Address, FormField::Name);
        for (fields, sign_in, step, why) in [
            (vec![field(address, false)], vec![web()], FirstStep::Form, "fields to fill"),
            (vec![field(address, true)], vec![web()], FirstStep::Account, "advanced fields don't make a form"),
            (vec![field(name, false)], vec![web()], FirstStep::Account, "a name alone is nothing to ask"),
            (vec![field(name, false)], Vec::new(), FirstStep::AddNow, "a name alone, and no account"),
            (Vec::new(), vec![SignIn::Paste], FirstStep::AddNow, "no page to go to"),
            (Vec::new(), vec![SignIn::Paste, web()], FirstStep::AddNow, "the page isn't offered first"),
            (vec![field(name, false)], vec![SignIn::Code, web()], FirstStep::Account, "a code is signed in with elsewhere too"),
            (vec![field(address, true)], Vec::new(), FirstStep::AddNow, "only advanced fields"),
        ] {
            assert_eq!(info(fields, sign_in).first_step(), step, "{why}");
        }
    }

    /// Each field reads its own part of the profile: set there, it is set, and no other field is.
    #[test]
    fn a_field_is_set_by_its_own_value() {
        use FormField::*;
        let proxy = crate::proxy::proxy_from_form("proxy.lan".into(), String::new(), String::new()).unwrap();
        let headers = std::collections::HashMap::from([("X-A".to_string(), "b".to_string())]);
        let cases = [
            (Name, SavedServer { name: "Home".into(), ..Default::default() }),
            (Address, SavedServer { url: "https://music.example".into(), ..Default::default() }),
            (User, SavedServer { user: "ann".into(), ..Default::default() }),
            (Password, SavedServer { password: " ".into(), ..Default::default() }),
            (SecondAddress, SavedServer { alt_url: "http://10.0.0.2".into(), ..Default::default() }),
            (ApiKey, SavedServer { api_key: "k".into(), ..Default::default() }),
            (Headers, SavedServer { headers, ..Default::default() }),
            (LegacyAuth, SavedServer { legacy_auth: true, ..Default::default() }),
            (SelfSigned, SavedServer { allow_self_signed: true, ..Default::default() }),
            (WifiOnly, SavedServer { wifi_only: true, ..Default::default() }),
            (ClientCert, SavedServer { client_cert: "c.p12".into(), ..Default::default() }),
            (Proxy, SavedServer { proxy, ..Default::default() }),
            (Folders, SavedServer { folders: vec!["/music".into()], ..Default::default() }),
        ];
        for (what, profile) in &cases {
            for (other, _) in &cases {
                assert_eq!(field_set(*other, profile), other == what, "{other:?} of a profile with {what:?}");
            }
        }
        assert!(!field_set(Name, &SavedServer { name: "  ".into(), ..Default::default() }), "blank is not set");
    }

    /// A server's form on each platform: Wi-Fi only only where a metered network can be kept off, a user
    /// and password only without an API key, a name only on a profile being added.
    #[test]
    fn a_form_shows_what_its_platform_honors() {
        use FormField::*;
        let server = subsonic::Subsonic.info();
        let keyed = SavedServer { api_key: "k".into(), ..Default::default() };
        let all = [Address, User, Password, Name, SecondAddress, ApiKey, Headers, LegacyAuth, SelfSigned, WifiOnly, ClientCert, Proxy];
        let without = |gone: &[FormField]| all.iter().copied().filter(|f| !gone.contains(f)).collect::<Vec<_>>();
        for (platform, profile, adding, want) in [
            (Platform::Phone, SavedServer::default(), true, without(&[])),
            (Platform::Computer, SavedServer::default(), true, without(&[WifiOnly])),
            (Platform::Ipod, SavedServer::default(), true, without(&[WifiOnly])),
            (Platform::Phone, keyed.clone(), true, without(&[User, Password])),
            (Platform::Computer, keyed, false, without(&[User, Password, Name, WifiOnly])),
            (Platform::Phone, SavedServer::default(), false, without(&[Name])),
        ] {
            let shown: Vec<FormField> = form_fields(&server, platform, &profile, adding).iter().map(|f| f.what).collect();
            assert_eq!(shown, want, "{platform:?}, adding {adding}");
        }
        let unasked = info(vec![FieldSpec { what: User, advanced: false }], Vec::new());
        assert_eq!(form_fields(&unasked, Platform::Phone, &SavedServer { api_key: "k".into(), ..Default::default() }, true).len(), 1, "a key the form doesn't ask hides nothing");
    }

    /// Each field shown takes its value as typed, in place of the saved one; one not shown keeps it.
    #[test]
    fn a_form_saves_only_what_it_shows() {
        let proxy = crate::proxy::proxy_from_form("proxy.lan".into(), String::new(), String::new()).unwrap();
        let saved = SavedServer { id: "p1".into(), name: "Home".into(), wifi_only: true, alt_max_bit_rate: 128, ..Default::default() };
        let typed = SavedServer {
            name: "Away".into(),
            url: "https://m".into(),
            alt_url: "http://lan".into(),
            api_key: "k".into(),
            headers: std::collections::HashMap::from([("X-A".to_string(), "b".to_string())]),
            legacy_auth: true,
            allow_self_signed: true,
            client_cert: "p1.p12".into(),
            client_cert_password: "pw".into(),
            proxy: proxy.clone(),
            ..Default::default()
        };
        let fields = form_fields(&subsonic::Subsonic.info(), Platform::Computer, &typed, false);
        let kept = form_taken(saved.clone(), &typed, &fields);
        let want = SavedServer { name: saved.name.clone(), wifi_only: true, user: String::new(), password: String::new(), id: "p1".into(), alt_max_bit_rate: 128, ..typed.clone() };
        assert_eq!(kept, want, "Name, Wi-Fi only, user and password aren't on this form");
        assert_eq!(form_taken(saved.clone(), &typed, &[]), saved);
    }

    /// The advanced part opens on a value of its own; one the platform doesn't show doesn't open it.
    #[test]
    fn the_advanced_part_opens_on_a_value_it_shows() {
        let server = subsonic::Subsonic.info();
        let shown = |p: &SavedServer, platform| advanced_shown(&form_fields(&server, platform, p, false), p);
        assert!(!shown(&SavedServer { url: "https://m".into(), user: "a".into(), ..Default::default() }, Platform::Computer), "only plain fields set");
        assert!(shown(&SavedServer { allow_self_signed: true, ..Default::default() }, Platform::Computer));
        assert!(shown(&SavedServer { wifi_only: true, ..Default::default() }, Platform::Phone));
        assert!(!shown(&SavedServer { wifi_only: true, ..Default::default() }, Platform::Computer), "Wi-Fi only isn't shown there");
        assert!(!shown(&SavedServer { name: "Home".into(), ..Default::default() }, Platform::Phone), "a saved profile's name is renamed from its menu");
    }

    #[test]
    fn only_a_sign_in_elsewhere_goes_on_after_adding() {
        for way in [web(), SignIn::Code] {
            let elsewhere = info(Vec::new(), vec![way, SignIn::Paste]);
            for (after, step) in [(AfterAdding::Remind, SignInStep::Offer), (AfterAdding::SignIn, SignInStep::Open), (AfterAdding::Guest, SignInStep::StopReminding)] {
                assert_eq!(elsewhere.after_adding(after), Some(step));
                assert_eq!(info(Vec::new(), vec![SignIn::Paste]).after_adding(after), None);
                assert_eq!(info(Vec::new(), Vec::new()).after_adding(after), None);
            }
        }
    }

    #[test]
    fn the_first_way_offered_is_the_one_opened() {
        use flow::SignInEvent;
        for (ways, opens) in [
            (vec![SignIn::Code, web(), SignIn::Paste], Some(SignInEvent::Code)),
            (vec![web(), SignIn::Code], Some(SignInEvent::Open { start: None })),
            (vec![SignIn::Paste, SignIn::Code], None),
            (Vec::new(), None),
        ] {
            assert_eq!(info(Vec::new(), ways.clone()).opens(), opens, "{ways:?}");
        }
    }

    #[test]
    fn a_resolved_stream_address_keeps_bit_rate_and_song() {
        for (kbps, id) in [(0, "abc"), (320, "a/b:c")] {
            let q = StreamQuality { bit_rate: kbps, format: String::new() };
            assert_eq!(resolved_song("x-song:", &resolved_stream("x-song:", id, &q).url), Some((kbps, id.to_string())));
        }
        assert_eq!(resolved_song("x-song:", "https://example.com/a"), None);
        assert_eq!(resolved_song("x-song:", "y-song:0/a"), None);
    }

    fn saved(id: &str, kind: ServerKind, url: &str, user: &str) -> SavedServer {
        SavedServer { id: id.into(), kind, url: url.into(), user: user.into(), ..Default::default() }
    }

    /// A login for a saved address and user takes over the saved profile; another address, user or a
    /// saved profile edited stays as it is.
    #[test]
    fn a_login_finds_the_saved_server() {
        let icon = crate::settings::ProfileIcon::Think;
        let scrobblers = vec![crate::settings::SavedScrobbler { id: "c".into(), on: true, key: "k".into(), ..Default::default() }];
        let kept = SavedServer { name: "Home".into(), music_folder_ids: vec!["3".into()], alt_max_bit_rate: 96, icon, scrobblers: scrobblers.clone(), ..saved("a1", ServerKind::Subsonic, "https://music.example.com", "ann") };
        let list = ServerList { servers: vec![kept.clone()], active_server_id: "a1".into() };
        let form = saved("new", ServerKind::Subsonic, "HTTPS://music.example.com/ ", "Ann");
        let k = Kinds::builtin();
        let login = k.login_profile(list.clone(), form.clone());
        assert_eq!((login.id.as_str(), login.name.as_str(), login.music_folder_ids.as_slice(), login.alt_max_bit_rate, login.user.as_str(), login.icon), ("a1", "Home", &["3".to_string()][..], 96, "Ann", icon));
        assert_eq!(login.scrobblers, scrobblers);
        let stranger = saved("s", ServerKind::Subsonic, "https://music.example.com", "bob");
        assert_eq!(k.login_profile(list.clone(), stranger.clone()), stranger);
        let elsewhere = saved("e", ServerKind::Subsonic, "https://other.example.com", "ann");
        assert_eq!(k.login_profile(list.clone(), elsewhere.clone()), elsewhere);
        let edited = SavedServer { url: "https://moved.example.com".into(), ..kept };
        assert_eq!(k.login_profile(list, edited.clone()), edited);
    }

    /// A login saves what its form holds. What the profile keeps besides stays as saved now, whether its
    /// Edit form, opened before the page changed it, is saved, or its account is signed in to again.
    #[test]
    fn a_login_keeps_what_its_form_does_not_hold() {
        let scrobblers = vec![crate::settings::SavedScrobbler { id: "c".into(), on: true, ..Default::default() }];
        let now = SavedServer {
            music_folder_ids: vec!["3".into()],
            alt_max_bit_rate: 96,
            icon: crate::settings::ProfileIcon::Think,
            picture: Some("https://pictures.example.com/ann".into()),
            scrobble_off: true,
            scrobblers,
            sources: vec!["b2".into()],
            ..saved("a1", ServerKind::Subsonic, "https://music.example.com", "ann")
        };
        let list = ServerList { servers: vec![now.clone()], active_server_id: "a1".into() };
        let edited = SavedServer { password: "new".into(), ..saved("a1", ServerKind::Subsonic, "https://music.example.com", "ann") };
        let again = SavedServer { password: "new".into(), ..saved("x", ServerKind::Subsonic, "https://music.example.com", "ann") };
        for form in [edited, again] {
            assert_eq!(Kinds::builtin().login_profile(list.clone(), form), SavedServer { password: "new".into(), ..now.clone() });
        }
        let signs_in = ProviderInfo { kind: ServerKind::Plugin { id: "x".into() }, fields: vec![FieldSpec { what: FormField::Name, advanced: false }], sign_in: vec![SignIn::Paste], identities: None };
        let signed_in = saved("p1", signs_in.kind.clone(), "", "ann");
        let list = ServerList { servers: vec![signed_in], active_server_id: "p1".into() };
        let opened_signed_out = SavedServer { name: "Mine".into(), ..saved("p1", signs_in.kind.clone(), "", "") };
        assert_eq!(server_for_login(&signs_in, list, opened_signed_out).user, "ann");
    }

    /// A kind that asks no address (a service's account) takes a profile per login.
    #[test]
    fn a_login_without_an_address_adds_a_profile() {
        let info = ProviderInfo { kind: ServerKind::Plugin { id: "x".into() }, fields: vec![FieldSpec { what: FormField::Name, advanced: false }], sign_in: Vec::new(), identities: None };
        let first = saved("a1", info.kind.clone(), "", "");
        let list = ServerList { servers: vec![first.clone()], active_server_id: "a1".into() };
        let second = saved("a2", info.kind.clone(), "", "");
        assert_eq!(server_for_login(&info, list.clone(), second.clone()), second);
        assert_eq!(server_for_login(&info, list, first.clone()), first);
    }

    /// A login to a saved server keeps that profile's library, music folder, bitrate cap and place in the
    /// list, and makes it the active one.
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
        let p = crate::client::tests::block(log_in(&Kinds::builtin(), fake, &settings, form)).unwrap();
        assert_eq!((p.id.as_str(), p.music_folder_ids.as_slice(), p.alt_max_bit_rate, p.password.as_str()), ("a1", &["3".to_string()][..], 96, "pw"));
        let prefs = settings.current().unwrap();
        let ids: Vec<&str> = prefs.servers.iter().map(|s| s.id.as_str()).collect();
        assert_eq!((ids, prefs.active_server().map(|s| s.id.as_str())), (vec!["a1", "b2"], Some("a1")));
    }

    /// A server's form needs an address, and a user or an API key.
    #[test]
    fn a_server_form_is_ready_with_an_address_and_a_user_or_key() {
        let (k, p) = (Kinds::builtin(), saved("p", ServerKind::Subsonic, "https://x", ""));
        assert!(!k.server_ready(p.clone()), "a user or a key");
        assert!(k.server_ready(SavedServer { user: "u".into(), ..p.clone() }));
        assert!(k.server_ready(SavedServer { api_key: "k".into(), ..p.clone() }));
        assert!(!k.server_ready(SavedServer { url: "  ".into(), user: "u".into(), ..p }));
    }

    /// A profile is offered as a source only standing alone: not one already a source, nor one with
    /// sources of its own, nor itself or a jam's guest.
    #[test]
    fn a_source_offered_stands_alone() {
        let with = |id: &str, sources: &[&str]| SavedServer { id: id.into(), sources: sources.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        let guest = SavedServer { api_key: nori_remote::guest_key("k"), ..with("guest", &[]) };
        let servers = vec![with("home", &["a"]), with("a", &[]), with("b", &[]), with("merged", &["c"]), with("c", &[]), guest];
        let offered: Vec<String> = sources_offered(servers.clone(), servers[0].clone()).into_iter().map(|s| s.id).collect();
        assert_eq!(offered, ["b"]);
    }

    /// A jam's guest profile lasts only while it is the one open.
    #[test]
    fn activating_another_profile_drops_a_jam_guest() {
        let own = saved("own", ServerKind::Subsonic, "http://own", "ann");
        let guest = SavedServer { api_key: nori_remote::guest_key("k"), ..saved("g", ServerKind::Subsonic, "http://friend", "") };
        let prefs = crate::settings::StoredPrefs { servers: vec![own.clone(), guest.clone()], active_server_id: "g".into(), ..Default::default() };
        let ids = |p: &crate::settings::StoredPrefs| p.servers.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&profile_activated(prefs.clone(), guest)), ["own", "g"], "opened again, it stays");
        let after = profile_activated(prefs, own);
        assert_eq!((ids(&after), after.active_server_id.as_str()), (vec!["own".to_string()], "own"));
    }

    /// A plugin's kind without the plugin has no provider: it is not offered, its form is never
    /// ready, and a profile saved with it does not open (a LAN jam guest's opens through its host).
    #[test]
    fn a_kind_without_its_plugin_has_no_provider() {
        let k = Kinds::builtin();
        assert_eq!(k.providers().into_iter().map(|p| p.kind).collect::<Vec<_>>(), vec![ServerKind::Subsonic]);
        let missing = ServerKind::Plugin { id: "fm.nori.youtube-music".into() };
        let guest = SavedServer { api_key: nori_remote::guest_key("lan-secret"), ..saved("g", missing.clone(), "http://10.0.0.2:4000", "") };
        assert_eq!([k.waits(saved("p", missing.clone(), "", "")), k.waits(saved("s", ServerKind::Subsonic, "", "")), k.waits(guest)], [Some(ProfileWait::NoPlugins), None, None]);
        assert!(!k.server_ready(saved("p", missing.clone(), "", "")));
        let core = Core::new(String::new(), "k".into(), Default::default()).unwrap();
        assert!(core.configure_profile(saved("p", missing, "https://x", "u")).is_err());
        assert!(core.configure_profile(saved("p", ServerKind::Subsonic, "https://x", "u")).is_ok());
    }

    /// A LAN jam's guest profile plays through its host, keeping the host's kind, even one not here; the
    /// next profile of that kind has its own provider again.
    #[test]
    fn a_lan_guest_plays_through_its_host_whatever_its_kind() {
        let core = Core::new(String::new(), "k".into(), Default::default()).unwrap();
        for kind in [ServerKind::Plugin { id: "fm.nori.youtube-music".into() }, ServerKind::Subsonic] {
            let guest = SavedServer { api_key: nori_remote::guest_key("lan-secret"), ..saved("g", kind.clone(), "http://10.0.0.2:4000", "") };
            core.configure_profile(guest).unwrap();
            assert_eq!(core.kind(), kind);
            assert!(core.lan_only() && core.lan_jam.read().is_some(), "{kind:?} through the host");
        }
        core.configure_profile(saved("p", ServerKind::Subsonic, "https://x", "u")).unwrap();
        assert!(!core.lan_only() && core.lan_jam.read().is_none(), "a server's own provider");
    }

    /// A profile of several sources has no one server to relay remote control and jams: they go over
    /// the LAN, as for a profile with no server.
    #[test]
    fn a_profile_of_several_sources_is_reached_over_the_lan() {
        let core = Core::new(String::new(), "k".into(), Default::default()).unwrap();
        core.configure_profile(saved("p", ServerKind::Subsonic, "https://x", "u")).unwrap();
        assert!(!core.lan_only(), "a server of its own");
        core.source_added("yt".into(), crate::providers::fake::source(String::new(), "yt")).unwrap();
        assert!(core.lan_only());
    }
}
