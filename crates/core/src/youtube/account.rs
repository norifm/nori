//! A YouTube Music profile's account: a browser session's cookies, kept in the profile's rows and
//! replaced there the moment YouTube rotates them, rotated before they go stale whenever the profile is
//! used, and given up only when YouTube refuses them (docs/research/youtube-music.md, "Signing in").
//!
//! The cookies are kept in the profile's `kv` rows, not in the stored settings: the platform holds and
//! writes the settings whole, and would put back the cookies of before a rotation.

use innertube_music as yt;
use serde::{Deserialize, Serialize};

use super::{net_error, Wire};
use crate::browse::youtube_rules;
use crate::client::{Client, NetResult};

/// The profile row the session is kept in.
const KEY: &str = "youtube_session";

/// Where a YouTube Music profile's account stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum YouTubeAccount {
    SignedOut,
    SignedIn,
    /// YouTube refused the session: the account's library and likes wait until it is signed in again.
    Ended,
}

/// How a sign-in went.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum YouTubeSignIn {
    /// Signed in to the account YouTube calls `name`.
    SignedIn { name: String },
    /// The text holds no signed-in session's cookies (no `SAPISID`).
    NoSession,
    /// YouTube does not take the cookies: the session has ended, or they are another browser's.
    Refused,
    /// YouTube could not be asked; nothing changed.
    Unreachable { reason: String },
}

/// The page a YouTube Music sign-in window opens (Google's sign-in, going on to music.youtube.com).
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn youtube_sign_in_page() -> String {
    yt::auth::SIGN_IN_PAGE.to_string()
}

/// Whether a sign-in window showing `address`, holding `cookies` for it (a `Cookie` header's value), is
/// done: its cookies are what [`Client::youtube_sign_in`] takes.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn youtube_sign_in_done(address: String, cookies: String) -> bool {
    yt::auth::signed_in_at(&address, &cookies)
}

/// The session as kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Kept {
    /// The `Cookie` header's value.
    cookies: String,
    /// When the cookies were last rotated (Unix seconds).
    rotated_at: u64,
    ended: bool,
}

/// The session: None until read from the profile's rows, then what they hold.
#[derive(Default)]
pub(crate) struct Session(parking_lot::Mutex<Option<Option<Kept>>>);

pub(super) fn unix_now() -> u64 {
    (crate::db::now_ms() / 1000).max(0) as u64
}

impl Client {
    /// The kept session, read from the profile's rows on first use; signed in, its cookies are the
    /// requests'.
    fn kept(&self) -> Option<Kept> {
        let mut s = self.innertube.session.0.lock();
        if s.is_none() {
            let kept: Option<Kept> = crate::db::kv_get(&self.core.db.lock(), KEY).ok().flatten().and_then(|j| serde_json::from_str(&j).ok());
            if let Some(k) = kept.as_ref().filter(|k| !k.ended) {
                self.innertube.api.set_cookies(Some(yt::Cookies::parse(&k.cookies)));
            }
            *s = Some(kept);
        }
        s.clone().flatten()
    }

    /// Keeps `kept` in memory and in the profile's rows at once; None forgets the session.
    fn keep(&self, kept: Option<Kept>) {
        // The database is let go before the session is locked: `kept` reads the database holding the session.
        let stored = {
            let db = self.core.db.lock();
            match &kept {
                Some(k) => crate::db::kv_put(&db, KEY, &serde_json::to_string(k).unwrap_or_default()),
                None => crate::db::kv_delete(&db, KEY),
            }
        };
        if let Err(e) = stored {
            crate::alog::info(&format!("YouTube session: could not keep it: {e}"));
        }
        *self.innertube.session.0.lock() = Some(kept);
    }

    /// Before a call: a signed-in session last rotated [`yt::auth::ROTATE_EVERY`] ago or more is rotated
    /// first. A failure that is not a refusal leaves it for the next call.
    pub(super) async fn youtube_fresh_session(&self, now: u64) {
        let Some(kept) = self.kept().filter(|k| !k.ended && yt::auth::rotation_due(k.rotated_at, now)) else { return };
        match self.innertube.api.rotate(&Wire(&*self.transport)).await {
            Ok(_) => {
                let cookies = self.innertube.api.take_changed().map_or(kept.cookies, |c| c.header());
                self.keep(Some(Kept { cookies, rotated_at: now, ended: false }));
            }
            Err(e) => {
                let ended = matches!(e, yt::Error::SignedOut);
                crate::alog::info(&format!("YouTube session: not rotated: {}", net_error(e)));
                if ended {
                    self.keep(Some(Kept { ended: true, ..kept }));
                }
            }
        }
    }

    /// After a call: cookies YouTube changed are kept at once; a session it refused is marked ended.
    pub(super) fn youtube_after<T>(&self, r: yt::Result<T, crate::transport::NetError>) -> NetResult<T> {
        if let Some(kept) = self.kept().filter(|k| !k.ended) {
            if matches!(r, Err(yt::Error::SignedOut)) {
                self.keep(Some(Kept { ended: true, ..kept }));
            } else if let Some(c) = self.innertube.api.take_changed() {
                self.keep(Some(Kept { cookies: c.header(), ..kept }));
            }
        }
        r.map_err(net_error)
    }

    /// Whether the session's cookies go with the requests.
    pub(super) fn youtube_signed_in(&self) -> bool {
        self.kept().is_some_and(|k| !k.ended)
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Signs this YouTube Music profile in with a browser session's cookies (a `Cookie` header or a
    /// cookies.txt), checked by asking YouTube for the account's name. Anything but success leaves the
    /// account as it was.
    pub async fn youtube_sign_in(&self, cookies: String) -> YouTubeSignIn {
        let jar = yt::Cookies::parse(&cookies);
        if !jar.signed_in() {
            return YouTubeSignIn::NoSession;
        }
        self.kept();
        let api = &self.innertube.api;
        let before = api.cookies();
        api.set_cookies(Some(jar));
        match api.account_name(&Wire(&*self.transport)).await {
            Ok(name) => {
                let cookies = api.cookies().map(|c| c.header()).unwrap_or_default();
                api.take_changed();
                // Fresh from the browser: it was just rotated.
                self.keep(Some(Kept { cookies, rotated_at: unix_now(), ended: false }));
                *self.core.rules.write() = youtube_rules(true);
                YouTubeSignIn::SignedIn { name }
            }
            Err(e) => {
                api.set_cookies(before);
                match e {
                    yt::Error::SignedOut | yt::Error::Status(401 | 403) => YouTubeSignIn::Refused,
                    e => YouTubeSignIn::Unreachable { reason: net_error(e).to_string() },
                }
            }
        }
    }

    /// Signs out: the session's cookies are forgotten here (the browser's session is not ended).
    pub fn youtube_sign_out(&self) {
        self.innertube.api.set_cookies(None);
        self.keep(None);
        *self.core.rules.write() = youtube_rules(false);
    }

    /// Where this YouTube Music profile's account stands.
    pub fn youtube_account(&self) -> YouTubeAccount {
        match self.kept() {
            None => YouTubeAccount::SignedOut,
            Some(k) if k.ended => YouTubeAccount::Ended,
            Some(_) => YouTubeAccount::SignedIn,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The kept session as a test stores it: last rotated `ago` seconds before now.
    pub(crate) fn session(cookies: &str, ago: u64) -> String {
        serde_json::to_string(&Kept { cookies: cookies.into(), rotated_at: unix_now() - ago, ended: false }).unwrap()
    }

    pub(crate) fn stored(c: &Client) -> Option<Kept> {
        crate::db::kv_get(&c.core.db.lock(), KEY).unwrap().map(|j| serde_json::from_str(&j).unwrap())
    }

    pub(crate) fn store(c: &Client, json: &str) {
        crate::db::kv_put(&c.core.db.lock(), KEY, json).unwrap();
    }

    impl Kept {
        pub(crate) fn cookies(&self) -> &str {
            &self.cookies
        }

        pub(crate) fn ended(&self) -> bool {
            self.ended
        }

        pub(crate) fn rotated_ago(&self) -> u64 {
            unix_now().saturating_sub(self.rotated_at)
        }
    }
}
