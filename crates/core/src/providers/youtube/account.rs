//! A YouTube Music profile's account: a browser session's cookies, kept in the profile's rows and
//! replaced there the moment YouTube rotates them, rotated before they go stale whenever the profile is
//! used, and given up only when YouTube refuses them (docs/research/youtube-music.md, "Signing in").
//!
//! The cookies are kept in the profile's `kv` rows, not in the stored settings: the platform holds and
//! writes the settings whole, and would put back the cookies of before a rotation.

use innertube_music as yt;
use serde::{Deserialize, Serialize};

use super::{net_error, rules, State, Wire};
use crate::client::{Client, NetResult};
use crate::providers::{AccountState, SignInResult};

/// The profile row the session is kept in.
const KEY: &str = "youtube_session";

/// The profile row YouTube's visitor id is kept in: the same visitor is served the same Home.
const VISITOR: &str = "youtube_visitor";

/// The session as kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Kept {
    /// The `Cookie` header's value.
    cookies: String,
    /// When the cookies were last rotated (Unix seconds).
    rotated_at: u64,
    ended: bool,
}

/// The session: None until read from the profile's rows, then what they hold.
#[derive(Default)]
pub(super) struct Session(parking_lot::Mutex<Option<Option<Kept>>>);

pub(super) fn unix_now() -> u64 {
    (crate::db::now_ms() / 1000).max(0) as u64
}

impl State {
    /// The kept session, read from the profile's rows on first use with the kept visitor id; signed in,
    /// its cookies are the requests'.
    fn kept(&self, c: &Client) -> Option<Kept> {
        let mut s = self.session.0.lock();
        if s.is_none() {
            let db = c.core.db.lock();
            self.api.set_visitor(crate::db::kv_get(&db, VISITOR).ok().flatten());
            let kept: Option<Kept> = crate::db::kv_get(&db, KEY).ok().flatten().and_then(|j| serde_json::from_str(&j).ok());
            drop(db);
            if let Some(k) = kept.as_ref().filter(|k| !k.ended) {
                self.api.set_cookies(Some(yt::Cookies::parse(&k.cookies)));
            }
            *s = Some(kept);
        }
        s.clone().flatten()
    }

    /// Keeps `kept` in memory and in the profile's rows at once; None forgets the session.
    fn keep(&self, c: &Client, kept: Option<Kept>) {
        // The database is let go before the session is locked: `kept` reads the database holding the session.
        let stored = {
            let db = c.core.db.lock();
            match &kept {
                Some(k) => crate::db::kv_put(&db, KEY, &serde_json::to_string(k).unwrap_or_default()),
                None => crate::db::kv_delete(&db, KEY),
            }
        };
        if let Err(e) = stored {
            crate::alog::info(&format!("YouTube session: could not keep it: {e}"));
        }
        *self.session.0.lock() = Some(kept);
    }

    /// Before a call: a signed-in session last rotated [`yt::auth::ROTATE_EVERY`] ago or more is rotated
    /// first. A failure that is not a refusal leaves it for the next call.
    pub(super) async fn fresh_session(&self, c: &Client, now: u64) {
        let Some(kept) = self.kept(c).filter(|k| !k.ended && yt::auth::rotation_due(k.rotated_at, now)) else { return };
        match self.api.rotate(&Wire(&*c.transport)).await {
            Ok(_) => {
                let cookies = self.api.take_changed().map_or(kept.cookies, |c| c.header());
                self.keep(c, Some(Kept { cookies, rotated_at: now, ended: false }));
            }
            Err(e) => {
                let ended = matches!(e, yt::Error::SignedOut);
                crate::alog::info(&format!("YouTube session: not rotated: {}", net_error(e)));
                if ended {
                    self.keep(c, Some(Kept { ended: true, ..kept }));
                }
            }
        }
    }

    /// After a call: a visitor id and cookies YouTube gave are kept at once; a session it refused is
    /// marked ended.
    pub(super) fn after<T>(&self, c: &Client, r: yt::Result<T, crate::transport::NetError>) -> NetResult<T> {
        if let Some(visitor) = self.api.take_new_visitor() {
            if let Err(e) = crate::db::kv_put(&c.core.db.lock(), VISITOR, &visitor) {
                crate::alog::info(&format!("YouTube visitor id: could not keep it: {e}"));
            }
        }
        if let Some(kept) = self.kept(c).filter(|k| !k.ended) {
            if matches!(r, Err(yt::Error::SignedOut)) {
                self.keep(c, Some(Kept { ended: true, ..kept }));
            } else if let Some(changed) = self.api.take_changed() {
                self.keep(c, Some(Kept { cookies: changed.header(), ..kept }));
            }
        }
        r.map_err(net_error)
    }

    /// Whether the session's cookies go with the requests.
    pub(super) fn signed_in(&self, c: &Client) -> bool {
        self.kept(c).is_some_and(|k| !k.ended)
    }

    /// Signs in with a browser session's cookies (a `Cookie` header or a cookies.txt), checked by asking
    /// YouTube for the account's name.
    pub(super) async fn sign_in(&self, c: &Client, cookies: String) -> SignInResult {
        let jar = yt::Cookies::parse(&cookies);
        if !jar.signed_in() {
            return SignInResult::NoSession;
        }
        self.kept(c);
        let api = &self.api;
        let before = api.cookies();
        api.set_cookies(Some(jar));
        match api.account_name(&Wire(&*c.transport)).await {
            Ok(name) => {
                let cookies = api.cookies().map(|c| c.header()).unwrap_or_default();
                api.take_changed();
                // Fresh from the browser: it was just rotated.
                self.keep(c, Some(Kept { cookies, rotated_at: unix_now(), ended: false }));
                c.core.set_rules(rules(true));
                SignInResult::SignedIn { name }
            }
            Err(e) => {
                api.set_cookies(before);
                match e {
                    yt::Error::SignedOut | yt::Error::Status(401 | 403) => SignInResult::Refused,
                    e => SignInResult::Unreachable { reason: net_error(e).to_string() },
                }
            }
        }
    }

    /// Forgets the session's cookies here.
    pub(super) fn sign_out(&self, c: &Client) {
        self.api.set_cookies(None);
        self.keep(c, None);
        c.core.set_rules(rules(false));
    }

    pub(super) fn account(&self, c: &Client) -> AccountState {
        match self.kept(c) {
            None => AccountState::SignedOut,
            Some(k) if k.ended => AccountState::Ended,
            Some(_) => AccountState::SignedIn,
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
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
