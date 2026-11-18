//! A profile's scrobbling accounts, speaking Last.fm's API (Last.fm, Libre.fm) or ListenBrainz's: adding one, and telling
//! the server and them what plays. A counted play waits in the profile's database until its account
//! takes it, so plays made offline go with a later song change.

use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::params;
use serde_json::{json, Value};

use crate::cache_policy::Read;
use crate::client::{Client, NetResult, Write};
use crate::settings::{new_server_id, SavedScrobbler, ScrobblerKind};
use crate::transport::{Exchange, NetError};
use crate::{Core, Song};
use nori_net::api::{form, md5_hex};
use nori_queue::scrobble::ScrobbleSend;

const LASTFM: &str = "https://ws.audioscrobbler.com/2.0/";
const LIBREFM: &str = "https://libre.fm/2.0/";
/// Libre.fm's API key: it takes any of 32 characters and checks no signature.
const LIBREFM_KEY: &str = "6e6f72692d6d757369632d6c69627265";
const LISTENBRAINZ: &str = "https://api.listenbrainz.org";
/// Plays sent in one request: Last.fm takes at most 50.
const BATCH: usize = 50;
/// Age at which a waiting play is dropped: Last.fm refuses older ones.
const OLDEST_MS: i64 = 14 * 24 * 3_600_000;

/// What adding an account asks.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ScrobblerForm {
    pub kind: ScrobblerKind,
    /// The service's API address; empty for the kind's own.
    pub url: String,
    /// Last.fm's API key, or ListenBrainz's user token.
    pub key: String,
    /// Last.fm's shared secret.
    pub secret: String,
    /// Libre.fm's account and password, which sign in once and aren't kept.
    pub user: String,
    pub password: String,
    /// The token allowed on the service's page ([`Client::scrobbler_approval`]); empty until then.
    pub approved: String,
}

/// A field of [`ScrobblerForm`] adding an account asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ScrobblerField {
    /// ListenBrainz's user token, in `key`.
    Token,
    /// Last.fm's API key, in `key`.
    ApiKey,
    Secret,
    User,
    Password,
    Address,
}

impl ScrobblerField {
    /// Its name in a form sent as JSON.
    pub fn id(self) -> &'static str {
        match self {
            Self::Token => "token",
            Self::ApiKey => "apiKey",
            Self::Secret => "secret",
            Self::User => "user",
            Self::Password => "password",
            Self::Address => "address",
        }
    }
}

/// A field adding an account asks, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ScrobblerAsk {
    pub field: ScrobblerField,
    /// Needed filled before the account signs in.
    pub required: bool,
    /// Folded away with the form's other advanced fields: an address only someone running their own
    /// server types.
    pub advanced: bool,
}

/// How adding a kind of account signs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ScrobblerSignIn {
    /// With what the form asks ([`Client::sign_in_scrobbler`]).
    Form,
    /// On the service's page: [`Client::scrobbler_approval`] gives the page to allow nori on, then
    /// [`Client::sign_in_scrobbler`] with that token `approved`.
    Browser,
}

/// The kinds of account a profile can add, in the order offered.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn scrobbler_kinds() -> Vec<ScrobblerKind> {
    ScrobblerKind::ALL.to_vec()
}

/// The fields adding a `kind` account asks, in order.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn scrobbler_fields(kind: ScrobblerKind) -> Vec<ScrobblerAsk> {
    use ScrobblerField::*;
    let fields: &[ScrobblerField] = match kind {
        ScrobblerKind::ListenBrainz => &[Token, Address],
        ScrobblerKind::LibreFm => &[User, Password, Address],
        ScrobblerKind::LastFm => &[ApiKey, Secret],
    };
    fields.iter().map(|&field| ScrobblerAsk { field, required: field != Address, advanced: field == Address }).collect()
}

/// How adding a `kind` account signs in.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn scrobbler_sign_in(kind: ScrobblerKind) -> ScrobblerSignIn {
    match kind {
        ScrobblerKind::LastFm => ScrobblerSignIn::Browser,
        ScrobblerKind::ListenBrainz | ScrobblerKind::LibreFm => ScrobblerSignIn::Form,
    }
}

/// The page where a `kind` account's form finds what it asks: ListenBrainz's token, Last.fm's API key.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn scrobbler_help_page(kind: ScrobblerKind) -> Option<String> {
    match kind {
        ScrobblerKind::ListenBrainz => Some("https://listenbrainz.org/settings/".into()),
        ScrobblerKind::LastFm => Some("https://www.last.fm/api/account/create".into()),
        ScrobblerKind::LibreFm => None,
    }
}

/// [`scrobbler_kinds`] with how each signs in, its help page and [`scrobbler_fields`], for a client
/// reading JSON: `[{kind, signIn: "form"|"browser", help, fields: [{id, required, advanced}]}]`, each by its `id`.
pub fn scrobbler_kinds_json() -> String {
    let kinds: Vec<Value> = scrobbler_kinds()
        .into_iter()
        .map(|k| {
            let fields: Vec<Value> = scrobbler_fields(k).into_iter().map(|a| json!({ "id": a.field.id(), "required": a.required, "advanced": a.advanced })).collect();
            let sign_in = match scrobbler_sign_in(k) {
                ScrobblerSignIn::Form => "form",
                ScrobblerSignIn::Browser => "browser",
            };
            json!({ "kind": k.id(), "signIn": sign_in, "help": scrobbler_help_page(k), "fields": fields })
        })
        .collect();
    Value::from(kinds).to_string()
}

/// What `form` holds in `field`.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn scrobbler_form_value(form: &ScrobblerForm, field: ScrobblerField) -> String {
    match field {
        ScrobblerField::Token | ScrobblerField::ApiKey => &form.key,
        ScrobblerField::Secret => &form.secret,
        ScrobblerField::User => &form.user,
        ScrobblerField::Password => &form.password,
        ScrobblerField::Address => &form.url,
    }
    .clone()
}

/// `form` with `value` in `field`.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn scrobbler_form_with(mut form: ScrobblerForm, field: ScrobblerField, value: String) -> ScrobblerForm {
    *match field {
        ScrobblerField::Token | ScrobblerField::ApiKey => &mut form.key,
        ScrobblerField::Secret => &mut form.secret,
        ScrobblerField::User => &mut form.user,
        ScrobblerField::Password => &mut form.password,
        ScrobblerField::Address => &mut form.url,
    } = value;
    form
}

/// Whether every field `form`'s kind needs is filled.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn scrobbler_form_ready(form: &ScrobblerForm) -> bool {
    scrobbler_fields(form.kind).into_iter().all(|a| !a.required || !scrobbler_form_value(form, a.field).trim().is_empty())
}

impl ScrobblerForm {
    /// A `kind` form from JSON naming each field by [`ScrobblerField::id`], and the token `approved` on
    /// the service's page; None for an unknown kind.
    pub fn from_json(kind: &str, fields: &Value, approved: &str) -> Option<ScrobblerForm> {
        let kind = ScrobblerKind::from_id(kind)?;
        let form = ScrobblerForm { kind, approved: approved.into(), ..Default::default() };
        Some(scrobbler_fields(kind).into_iter().fold(form, |form, a| scrobbler_form_with(form, a.field, fields[a.field.id()].as_str().unwrap_or_default().to_string())))
    }
}

/// A token to allow nori on the service's `page` with, before signing in with it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ScrobblerApproval {
    pub token: String,
    pub page: String,
}

/// A song as an account is told it.
#[derive(Debug, Clone, PartialEq)]
struct Track {
    artist: String,
    title: String,
    album: String,
    duration: u32,
    track: u32,
}

impl Track {
    /// None for a song no service takes: one without an artist or title (a radio stream).
    fn of(s: &Song) -> Option<Track> {
        (!s.artist.trim().is_empty() && !s.title.trim().is_empty())
            .then(|| Track { artist: s.artist.clone(), title: s.title.clone(), album: s.album.clone(), duration: s.duration, track: s.track })
    }
}

/// What became of plays sent to an account.
#[derive(Debug, PartialEq)]
enum Fate {
    Taken,
    /// Not taken now (offline, the service busy, the account's key refused): they wait.
    Kept,
    /// Refused as they stand: sent again they are refused again.
    Refused,
}

fn fate(answer: &NetResult<()>) -> Fate {
    match answer {
        Ok(()) => Fate::Taken,
        // Last.fm's codes: failed, offline, busy, too many requests; the account's sign-in or key refused.
        Err(NetError::Api { code: 4 | 8 | 9 | 10 | 11 | 13 | 16 | 26 | 29, .. }) => Fate::Kept,
        Err(NetError::Api { .. }) => Fate::Refused,
        Err(NetError::Http { status }) if (400..500).contains(status) && !matches!(status, 401 | 403 | 408 | 429) => Fate::Refused,
        Err(_) => Fate::Kept,
    }
}

/// Where `kind`'s requests go: `url` typed, or the kind's own service.
fn base(kind: ScrobblerKind, url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    match kind {
        ScrobblerKind::LastFm if url.is_empty() => LASTFM.into(),
        ScrobblerKind::LibreFm if url.is_empty() => LIBREFM.into(),
        ScrobblerKind::LastFm | ScrobblerKind::LibreFm => format!("{url}/"),
        ScrobblerKind::ListenBrainz if url.is_empty() => LISTENBRAINZ.into(),
        ScrobblerKind::ListenBrainz => url.into(),
    }
}

/// A Last.fm call's form, signed with `secret`: `api_sig` is the MD5 of every name and value in name
/// order, then the secret.
fn lastfm_form(mut fields: Vec<(String, String)>, secret: &str) -> String {
    fields.sort();
    let signed: String = fields.iter().map(|(k, v)| format!("{k}{v}")).collect();
    fields.push(("api_sig".into(), md5_hex(&(signed + secret))));
    fields.push(("format".into(), "json".into()));
    form(&fields)
}

/// The fields Last.fm describes `t` with, `[i]` after each name for a play in a batch.
fn lastfm_track(t: &Track, i: Option<usize>) -> Vec<(String, String)> {
    let at = |name: &str| i.map_or_else(|| name.to_string(), |i| format!("{name}[{i}]"));
    let mut f = vec![(at("artist"), t.artist.clone()), (at("track"), t.title.clone())];
    if !t.album.is_empty() {
        f.push((at("album"), t.album.clone()));
    }
    if t.duration > 0 {
        f.push((at("duration"), t.duration.to_string()));
    }
    if t.track > 0 {
        f.push((at("trackNumber"), t.track.to_string()));
    }
    f
}

fn listenbrainz_track(t: &Track) -> Value {
    let mut info = json!({ "media_player": "nori", "submission_client": "nori" });
    if t.duration > 0 {
        info["duration_ms"] = (t.duration as u64 * 1000).into();
    }
    if t.track > 0 {
        info["tracknumber"] = t.track.into();
    }
    let mut m = json!({ "artist_name": t.artist, "track_name": t.title, "additional_info": info });
    if !t.album.is_empty() {
        m["release_name"] = t.album.as_str().into();
    }
    m
}

fn answer(body: &[u8]) -> NetResult<Value> {
    serde_json::from_slice(body).map_err(|e| NetError::Parse { reason: e.to_string() })
}

/// Clears a flag however the work it marks ends.
struct Clears<'a>(&'a AtomicBool);

impl Drop for Clears<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Core {
    fn waiting_add(&self, account: &str, t: &Track, at_ms: i64) -> rusqlite::Result<()> {
        self.db.lock().execute(
            "INSERT INTO scrobbles_waiting(server, account, artist, title, album, duration, track, at_ms) VALUES(sid(), ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![account, t.artist, t.title, t.album, t.duration, t.track, at_ms],
        )?;
        Ok(())
    }

    /// The oldest plays waiting for `account`, with their row and start.
    fn waiting(&self, account: &str) -> rusqlite::Result<Vec<(i64, Track, i64)>> {
        let c = self.db.lock();
        let mut st = c.prepare_cached(
            "SELECT rowid, artist, title, album, duration, track, at_ms FROM scrobbles_waiting WHERE server=sid() AND account=?1 ORDER BY at_ms LIMIT ?2",
        )?;
        let rows = st.query_map(params![account, BATCH as i64], |r| {
            Ok((r.get(0)?, Track { artist: r.get(1)?, title: r.get(2)?, album: r.get(3)?, duration: r.get(4)?, track: r.get(5)? }, r.get(6)?))
        })?;
        rows.collect()
    }

    fn waiting_done(&self, rows: &[(i64, Track, i64)]) -> rusqlite::Result<()> {
        let c = self.db.lock();
        let mut st = c.prepare_cached("DELETE FROM scrobbles_waiting WHERE server=sid() AND rowid=?1")?;
        for (row, ..) in rows {
            st.execute([row])?;
        }
        Ok(())
    }

    /// Drops plays too old to send and those of accounts no longer on.
    fn waiting_trim(&self, on: &[SavedScrobbler], now_ms: i64) -> rusqlite::Result<()> {
        let ids = Value::from(on.iter().map(|a| a.id.as_str()).collect::<Vec<_>>()).to_string();
        self.db.lock().execute(
            "DELETE FROM scrobbles_waiting WHERE server=sid() AND (?1 - at_ms >= ?2 OR account NOT IN (SELECT value FROM json_each(?3)))",
            params![now_ms, OLDEST_MS, ids],
        )?;
        Ok(())
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// A token for `form`'s Last.fm API key, and the page where the user allows nori with it. Tokens
    /// last an hour and are used once.
    pub async fn scrobbler_approval(&self, form: ScrobblerForm) -> NetResult<ScrobblerApproval> {
        let key = form.key.trim();
        let fields = vec![("method".to_string(), "auth.getToken".to_string()), ("api_key".to_string(), key.to_string())];
        let got = self.lastfm(LASTFM, fields, form.secret.trim()).await?;
        let Some(token) = got["token"].as_str() else {
            return Err(NetError::Parse { reason: "no token in Last.fm's answer".into() });
        };
        let page = format!("https://www.last.fm/api/auth/?{}", nori_net::api::form(&[("api_key".to_string(), key.to_string()), ("token".to_string(), token.to_string())]));
        Ok(ScrobblerApproval { token: token.into(), page })
    }

    /// Signs in to the account `form` describes: Last.fm's session for the token `approved` on its page,
    /// Libre.fm's from its user and password, or ListenBrainz's token checked. The account to keep in
    /// the profile, on.
    pub async fn sign_in_scrobbler(&self, form: ScrobblerForm) -> NetResult<SavedScrobbler> {
        let key = if form.kind == ScrobblerKind::LibreFm { LIBREFM_KEY.into() } else { form.key.trim().to_string() };
        let account = SavedScrobbler { id: new_server_id(), kind: form.kind, on: true, url: form.url.trim().to_string(), key, ..Default::default() };
        let base = base(form.kind, &account.url);
        let secret = form.secret.trim().to_string();
        let session = match form.kind {
            ScrobblerKind::LastFm => vec![("method", "auth.getSession".to_string()), ("token", form.approved.clone())],
            // Libre.fm takes no password, only the MD5 of the user in lower case and the password's MD5.
            ScrobblerKind::LibreFm => {
                let user = form.user.trim();
                let proof = md5_hex(&(user.to_lowercase() + &md5_hex(&form.password)));
                vec![("method", "auth.getMobileSession".to_string()), ("username", user.to_string()), ("authToken", proof)]
            }
            ScrobblerKind::ListenBrainz => {
                let got = self.listenbrainz(&base, &account.key, "validate-token", None).await?;
                if got["valid"] != true {
                    return Err(NetError::Http { status: 401 });
                }
                let user = got["user_name"].as_str().unwrap_or_default().to_string();
                return Ok(SavedScrobbler { user, ..account });
            }
        };
        let mut fields: Vec<(String, String)> = session.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        fields.push(("api_key".into(), account.key.clone()));
        let got = self.lastfm(&base, fields, &secret).await?;
        let session = &got["session"];
        let (Some(user), Some(key)) = (session["name"].as_str(), session["key"].as_str()) else {
            return Err(NetError::Parse { reason: format!("no session in {}'s answer", form.kind.id()) });
        };
        Ok(SavedScrobbler { secret, session: key.into(), user: user.into(), ..account })
    }

    /// Tells what [`nori_queue::Session::scrobble_track`] said to the server, when scrobbling is on
    /// and the profile's rules let it count plays, and to the profile's accounts that are on, whatever
    /// those say: their accounts are their own. A play the server can't take now waits in its queue
    /// of writes; one an account can't, in `scrobbles_waiting`.
    pub async fn scrobble(&self, send: ScrobbleSend) {
        let server = self.settings().in_effect(|p| !p.active_server().is_some_and(|s| s.scrobble_off)) && self.core.rules().scrobbles;
        if server {
            if let Some(s) = &send.counted {
                if let Err(e) = self.write(Write::Scrobble { id: s.id.clone(), submission: true, time_ms: Some(send.counted_at) }).await {
                    crate::alog::info(&format!("scrobble: {e}"));
                }
            }
            // Asked, not queued: replayed later it would no longer be true.
            if let Some(s) = &send.playing {
                if let Err(e) = self.read_now(Read::NowPlaying { id: s.id.clone() }).await {
                    crate::alog::info(&format!("now playing: {e}"));
                }
            }
        }
        let accounts: Vec<SavedScrobbler> = self.settings().prefs(|p| p.active_server().map(|s| s.scrobblers.iter().filter(|a| a.on).cloned().collect()).unwrap_or_default());
        if let Err(e) = self.core.waiting_trim(&accounts, crate::db::now_ms()) {
            crate::alog::info(&format!("scrobble: the waiting plays could not be read: {e}"));
        }
        if accounts.is_empty() {
            return;
        }
        if let Some(t) = send.counted.as_ref().and_then(Track::of) {
            for a in &accounts {
                if let Err(e) = self.core.waiting_add(&a.id, &t, send.counted_at) {
                    crate::alog::info(&format!("scrobble: a play could not be kept: {e}"));
                }
            }
        }
        // A second call while plays go is left to the next song change: they'd be sent twice.
        if !self.scrobbling.swap(true, Ordering::AcqRel) {
            let _clears = Clears(&self.scrobbling);
            for a in &accounts {
                self.send_waiting(a).await;
            }
        }
        if let Some(t) = send.playing.as_ref().and_then(Track::of) {
            for a in &accounts {
                if let Err(e) = self.tell_playing(a, &t).await {
                    crate::alog::info(&format!("now playing, {}: {e}", a.kind.id()));
                }
            }
        }
    }
}

impl Client {
    /// Sends `account`'s waiting plays, a batch at a time, until one isn't taken.
    async fn send_waiting(&self, account: &SavedScrobbler) {
        loop {
            let rows = match self.core.waiting(&account.id) {
                Ok(rows) if !rows.is_empty() => rows,
                _ => return,
            };
            let answer = match account.kind {
                ScrobblerKind::LastFm | ScrobblerKind::LibreFm => self.lastfm_scrobble(account, &rows).await,
                ScrobblerKind::ListenBrainz => self.listenbrainz_listens(account, &rows).await,
            };
            let fate = fate(&answer);
            if let Err(e) = &answer {
                crate::alog::info(&format!("scrobble, {}: {} plays {fate:?}: {e}", account.kind.id(), rows.len()));
            }
            if fate == Fate::Kept || self.core.waiting_done(&rows).is_err() || rows.len() < BATCH {
                return;
            }
        }
    }

    async fn lastfm_scrobble(&self, a: &SavedScrobbler, rows: &[(i64, Track, i64)]) -> NetResult<()> {
        let mut fields = vec![("method".to_string(), "track.scrobble".to_string()), ("api_key".into(), a.key.clone()), ("sk".into(), a.session.clone())];
        for (i, (_, t, at_ms)) in rows.iter().enumerate() {
            fields.extend(lastfm_track(t, Some(i)));
            fields.push((format!("timestamp[{i}]"), (at_ms / 1000).to_string()));
        }
        self.lastfm(&base(a.kind, &a.url), fields, &a.secret).await.map(drop)
    }

    async fn listenbrainz_listens(&self, a: &SavedScrobbler, rows: &[(i64, Track, i64)]) -> NetResult<()> {
        let payload: Vec<Value> = rows.iter().map(|(_, t, at_ms)| json!({ "listened_at": at_ms / 1000, "track_metadata": listenbrainz_track(t) })).collect();
        let kind = if payload.len() == 1 { "single" } else { "import" };
        self.listenbrainz(&base(a.kind, &a.url), &a.key, "submit-listens", Some(json!({ "listen_type": kind, "payload": payload }))).await.map(drop)
    }

    async fn tell_playing(&self, a: &SavedScrobbler, t: &Track) -> NetResult<()> {
        let base = base(a.kind, &a.url);
        match a.kind {
            ScrobblerKind::LastFm | ScrobblerKind::LibreFm => {
                let mut fields = vec![("method".to_string(), "track.updateNowPlaying".to_string()), ("api_key".into(), a.key.clone()), ("sk".into(), a.session.clone())];
                fields.extend(lastfm_track(t, None));
                self.lastfm(&base, fields, &a.secret).await.map(drop)
            }
            ScrobblerKind::ListenBrainz => {
                let body = json!({ "listen_type": "playing_now", "payload": [{ "track_metadata": listenbrainz_track(t) }] });
                self.listenbrainz(&base, &a.key, "submit-listens", Some(body)).await.map(drop)
            }
        }
    }

    /// One Last.fm call; its error answer is [`NetError::Api`] with Last.fm's code.
    async fn lastfm(&self, base: &str, fields: Vec<(String, String)>, secret: &str) -> NetResult<Value> {
        let r = self.transport.send(Exchange { url: base.into(), form: Some(lastfm_form(fields, secret)), ..Default::default() }).await?;
        let got = answer(&r.body);
        if let Ok(Some(code)) = got.as_ref().map(|v| v["error"].as_i64()) {
            let reason = got.as_ref().ok().and_then(|v| v["message"].as_str()).unwrap_or_default().to_string();
            return Err(NetError::Api { code: code as i32, reason });
        }
        if !(200..300).contains(&r.status) {
            return Err(NetError::Http { status: r.status });
        }
        got
    }

    /// One ListenBrainz call to `/1/{path}`: a GET, or a POST of `body`.
    async fn listenbrainz(&self, base: &str, token: &str, path: &str, body: Option<Value>) -> NetResult<Value> {
        let headers = [("Authorization".to_string(), format!("Token {token}"))].into();
        let r = self.transport.send(Exchange { url: format!("{base}/1/{path}"), headers, json: body.map(|b| b.to_string()), ..Default::default() }).await?;
        if !(200..300).contains(&r.status) {
            return Err(NetError::Http { status: r.status });
        }
        answer(&r.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{block, client};
    use crate::client::NetProfile;
    use crate::settings::{SavedServer, StoredPrefs};
    use crate::transport::FailureKind;

    fn song(id: &str) -> Song {
        Song { id: id.into(), title: format!("T{id}"), artist: "A".into(), album: "L".into(), duration: 200, track: 3, ..Default::default() }
    }

    fn profile(c: &Client, accounts: Vec<SavedScrobbler>, scrobble: bool) {
        let server = SavedServer { id: "p".into(), scrobblers: accounts, scrobble_off: !scrobble, ..Default::default() };
        if c.core.session.settings.current().is_none() {
            c.core.session.settings.open("").unwrap();
        }
        c.core.session.settings.put(StoredPrefs { servers: vec![server], active_server_id: "p".into(), ..Default::default() });
    }

    fn lastfm() -> SavedScrobbler {
        SavedScrobbler { id: "fm".into(), kind: ScrobblerKind::LastFm, on: true, key: "k".into(), secret: "s".into(), session: "sk".into(), ..Default::default() }
    }

    fn listenbrainz() -> SavedScrobbler {
        SavedScrobbler { id: "lb".into(), kind: ScrobblerKind::ListenBrainz, on: true, url: "https://maloja.example/apis/listenbrainz/".into(), key: "t".into(), ..Default::default() }
    }

    fn played(counted: Option<&str>, at: i64, playing: Option<&str>) -> ScrobbleSend {
        ScrobbleSend { counted: counted.map(song), counted_at: at, playing: playing.map(song) }
    }

    fn fields(e: &Exchange) -> Vec<(String, String)> {
        e.form.as_deref().unwrap_or_default().split('&').map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())).unwrap()).collect()
    }

    fn field<'a>(f: &'a [(String, String)], name: &str) -> Option<&'a str> {
        f.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn waiting(c: &Client) -> u32 {
        c.core.db.lock().query_row("SELECT count(*) FROM scrobbles_waiting", [], |r| r.get(0)).unwrap()
    }

    /// Last.fm's example: the signature is the MD5 of the fields in name order, then the secret.
    #[test]
    fn a_lastfm_call_is_signed() {
        let f = lastfm_form(vec![("method".into(), "auth.getMobileSession".into()), ("api_key".into(), "xxx".into()), ("username".into(), "a b".into())], "sec");
        let sig = md5_hex("api_keyxxxmethodauth.getMobileSessionusernamea bsec");
        assert_eq!(f, format!("api_key=xxx&method=auth.getMobileSession&username=a%20b&api_sig={sig}&format=json"));
    }

    #[test]
    fn addresses() {
        assert_eq!(base(ScrobblerKind::LastFm, " "), LASTFM);
        assert_eq!(base(ScrobblerKind::LibreFm, ""), LIBREFM);
        assert_eq!(base(ScrobblerKind::LibreFm, "https://gnufm.example/2.0"), "https://gnufm.example/2.0/");
        assert_eq!(base(ScrobblerKind::ListenBrainz, ""), LISTENBRAINZ);
        assert_eq!(base(ScrobblerKind::ListenBrainz, "https://m.example/apis/listenbrainz/"), "https://m.example/apis/listenbrainz");
    }

    /// Last.fm signs in with a token allowed on its page: no password asked, the session kept.
    #[test]
    fn last_fm_signs_in_with_a_token_allowed_on_its_page() {
        let (c, fake) = client(NetProfile::default());
        fake.answer(r#"{"token":"tk"}"#);
        let form = ScrobblerForm { key: " k ".into(), secret: "s".into(), ..Default::default() };
        let approval = block(c.scrobbler_approval(form.clone())).unwrap();
        assert_eq!(approval, ScrobblerApproval { token: "tk".into(), page: "https://www.last.fm/api/auth/?api_key=k&token=tk".into() });
        let asked = fields(&fake.sent.lock()[0]);
        assert_eq!((field(&asked, "method"), field(&asked, "api_key")), (Some("auth.getToken"), Some("k")));
        assert!(field(&asked, "api_sig").is_some(), "signed");

        fake.answer(r#"{"error":14,"message":"Unauthorized Token - This token has not been authorized"}"#);
        let form = ScrobblerForm { approved: approval.token, ..form };
        assert!(matches!(block(c.sign_in_scrobbler(form.clone())), Err(NetError::Api { code: 14, .. })), "not allowed yet");
        fake.answer(r#"{"session":{"name":"Ann","key":"sk1","subscriber":0}}"#);
        let a = block(c.sign_in_scrobbler(form)).unwrap();
        assert_eq!((a.kind, a.on, a.key.as_str(), a.secret.as_str(), a.session.as_str(), a.user.as_str()), (ScrobblerKind::LastFm, true, "k", "s", "sk1", "Ann"));
        let sent = fields(&fake.sent.lock()[2]);
        assert_eq!((field(&sent, "method"), field(&sent, "token"), field(&sent, "password")), (Some("auth.getSession"), Some("tk"), None));
    }

    /// Libre.fm signs in with the MD5 of the user and the password's MD5, on its own key and address.
    #[test]
    fn libre_fm_signs_in_without_the_password() {
        let (c, fake) = client(NetProfile::default());
        fake.answer(r#"{"session":{"name":"ann","key":"sk2","subscriber":0}}"#);
        let form = ScrobblerForm { kind: ScrobblerKind::LibreFm, user: " Ann ".into(), password: "pw".into(), ..Default::default() };
        let a = block(c.sign_in_scrobbler(form)).unwrap();
        assert_eq!((a.kind, a.key.as_str(), a.session.as_str(), a.user.as_str()), (ScrobblerKind::LibreFm, LIBREFM_KEY, "sk2", "ann"));
        let sent = fake.sent.lock()[0].clone();
        let f = fields(&sent);
        let token = md5_hex(&format!("ann{}", md5_hex("pw")));
        assert_eq!(sent.url, LIBREFM);
        assert_eq!((field(&f, "username"), field(&f, "authToken"), field(&f, "password")), (Some("Ann"), Some(token.as_str()), None));
    }

    /// Each kind's form is ready once every field it asks but the address is filled; a form sent as JSON
    /// keeps only those fields.
    #[test]
    fn a_form_asks_each_kinds_fields() {
        let full = json!({ "token": "t", "apiKey": "k", "secret": "s", "user": "u", "password": "p", "address": "" });
        for kind in ScrobblerKind::ALL {
            let form = ScrobblerForm::from_json(kind.id(), &full, "").unwrap();
            assert!(scrobbler_form_ready(&form), "{kind:?}");
            for f in scrobbler_fields(kind).into_iter().filter(|a| a.required).map(|a| a.field) {
                assert!(!scrobbler_form_ready(&scrobbler_form_with(form.clone(), f, " ".into())), "{kind:?} without {f:?}");
            }
        }
        let libre = ScrobblerForm::from_json("librefm", &full, "").unwrap();
        assert_eq!((libre.key.as_str(), libre.secret.as_str(), libre.user.as_str()), ("", "", "u"));
        let lastfm = ScrobblerForm::from_json("lastfm", &full, "tk").unwrap();
        assert_eq!((lastfm.key.as_str(), lastfm.user.as_str(), lastfm.approved.as_str()), ("k", "", "tk"));
        assert_eq!(ScrobblerForm::from_json("listenbrainz", &full, "").unwrap().key, "t");
        assert!(ScrobblerForm::from_json("other", &full, "").is_none());
    }

    #[test]
    fn a_listenbrainz_token_is_checked() {
        let (c, fake) = client(NetProfile::default());
        fake.answer(r#"{"code":200,"valid":true,"user_name":"lee"}"#);
        let form = ScrobblerForm { kind: ScrobblerKind::ListenBrainz, key: "t".into(), ..Default::default() };
        let a = block(c.sign_in_scrobbler(form.clone())).unwrap();
        assert_eq!((a.user.as_str(), a.session.as_str()), ("lee", ""));
        let sent = fake.sent.lock()[0].clone();
        assert_eq!((sent.url.as_str(), sent.headers["Authorization"].as_str()), ("https://api.listenbrainz.org/1/validate-token", "Token t"));

        fake.answer(r#"{"code":200,"valid":false}"#);
        assert!(block(c.sign_in_scrobbler(form)).is_err());
    }

    /// Each account on is told the play and what plays now; one off, nothing.
    #[test]
    fn plays_go_to_the_accounts_on() {
        let (c, fake) = client(NetProfile::default());
        profile(&c, vec![lastfm(), listenbrainz(), SavedScrobbler { id: "off".into(), on: false, ..lastfm() }], false);
        for _ in 0..4 {
            fake.answer("{}");
        }
        block(c.scrobble(played(Some("a"), 1_700_000_000_000, Some("b"))));
        let sent = fake.sent.lock().clone();
        assert_eq!(sent.len(), 4, "no server and no account off");

        let scrobble = fields(&sent[0]);
        let pairs = ["method", "artist[0]", "track[0]", "album[0]", "duration[0]", "trackNumber[0]", "timestamp[0]", "sk"].map(|n| field(&scrobble, n));
        assert_eq!(pairs, [Some("track.scrobble"), Some("A"), Some("Ta"), Some("L"), Some("200"), Some("3"), Some("1700000000"), Some("sk")]);
        let listen: Value = serde_json::from_str(sent[1].json.as_deref().unwrap()).unwrap();
        assert_eq!(sent[1].url, "https://maloja.example/apis/listenbrainz/1/submit-listens");
        assert_eq!(listen["listen_type"], "single");
        assert_eq!(listen["payload"][0]["listened_at"], 1_700_000_000);
        assert_eq!(listen["payload"][0]["track_metadata"]["track_name"], "Ta");
        assert_eq!(listen["payload"][0]["track_metadata"]["additional_info"]["duration_ms"], 200_000);

        assert_eq!(field(&fields(&sent[2]), "method"), Some("track.updateNowPlaying"));
        assert_eq!(field(&fields(&sent[2]), "track"), Some("Tb"));
        let now: Value = serde_json::from_str(sent[3].json.as_deref().unwrap()).unwrap();
        assert_eq!((now["listen_type"].as_str(), now["payload"][0].get("listened_at")), (Some("playing_now"), None));
        assert_eq!(waiting(&c), 0, "taken");
    }

    /// A play not taken waits and goes, in one batch with the next, once the service answers; a play it
    /// refuses as it stands is dropped.
    #[test]
    fn a_play_not_taken_waits() {
        let (c, fake) = client(NetProfile::default());
        profile(&c, vec![listenbrainz()], false);
        let now = crate::db::now_ms();
        fake.fail(FailureKind::Connect);
        block(c.scrobble(played(Some("a"), now - 60_000, None)));
        fake.answers.lock().push_back(Ok((503, Vec::new())));
        block(c.scrobble(played(Some("b"), now, None)));
        assert_eq!(waiting(&c), 2);

        fake.sent.lock().clear();
        fake.answer("{}");
        block(c.scrobble(played(None, 0, None)));
        let sent = fake.sent.lock().clone();
        let listens: Value = serde_json::from_str(sent[0].json.as_deref().unwrap()).unwrap();
        assert_eq!(listens["listen_type"], "import");
        let titles: Vec<&str> = listens["payload"].as_array().unwrap().iter().map(|p| p["track_metadata"]["track_name"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Ta", "Tb"], "oldest first");
        assert_eq!(waiting(&c), 0);

        fake.answers.lock().push_back(Ok((400, Vec::new())));
        block(c.scrobble(played(Some("c"), now, None)));
        assert_eq!(waiting(&c), 0, "refused, dropped");
    }

    #[test]
    fn lastfm_answers() {
        let api = |code| fate(&Err(NetError::Api { code, reason: String::new() }));
        let http = |status| fate(&Err(NetError::Http { status }));
        assert_eq!([api(9), api(11), api(29), api(6)], [Fate::Kept, Fate::Kept, Fate::Kept, Fate::Refused]);
        assert_eq!([http(400), http(401), http(429), http(502)], [Fate::Refused, Fate::Kept, Fate::Kept, Fate::Kept]);
    }

    /// Plays waiting too long, or for an account turned off, are dropped; a song without an artist (a
    /// radio stream) waits for none.
    #[test]
    fn waiting_plays_are_trimmed() {
        let (c, fake) = client(NetProfile::default());
        profile(&c, vec![lastfm(), listenbrainz()], false);
        let now = crate::db::now_ms();
        for kind in [FailureKind::Connect, FailureKind::Connect] {
            fake.fail(kind);
        }
        block(c.scrobble(played(Some("a"), now - OLDEST_MS - 1, None)));
        block(c.scrobble(played(Some("b"), now, None)));
        block(c.scrobble(ScrobbleSend { counted: Some(Song { artist: String::new(), ..song("radio") }), counted_at: now, playing: None }));
        profile(&c, vec![lastfm()], false);
        block(c.scrobble(played(None, 0, None)));
        let left: Vec<(String, String)> = c.core.db.lock().prepare("SELECT account, title FROM scrobbles_waiting").unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        assert_eq!(left, [("fm".to_string(), "Tb".to_string())]);
    }

    /// The server is told when the profile's switch for it is on and the rules let it count plays.
    #[test]
    fn the_server_is_told_when_its_switch_is_on() {
        for on in [false, true] {
            let (c, fake) = client(NetProfile { url: "http://nd".into(), ..Default::default() });
            c.core.set_rules(crate::browse::ProfileRules { scrobbles: true, ..Default::default() });
            profile(&c, Vec::new(), on);
            fake.answer(crate::client::tests::OK);
            fake.answer(crate::client::tests::OK);
            block(c.scrobble(played(Some("a"), 5_000, Some("b"))));
            let asked = fake.asked();
            let expected = if on { vec!["&id=a&submission=true&time=5000", "&id=b&submission=false"] } else { vec![] };
            assert_eq!(asked.len(), expected.len(), "{asked:?}");
            for (url, end) in asked.iter().zip(expected) {
                assert!(url.contains("/rest/scrobble") && url.ends_with(end), "{url}");
            }
        }
    }
}
