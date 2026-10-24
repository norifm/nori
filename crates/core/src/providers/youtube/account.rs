//! A YouTube Music profile's account: a browser session's cookies, kept in the profile's rows and
//! replaced there the moment YouTube rotates them, rotated before they go stale whenever the profile is
//! used, and given up only when YouTube refuses them.
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
    /// Ended, and asked not to be reminded of at start.
    #[serde(default)]
    quiet: bool,
}

/// The session: None until read from the profile's rows, then what they hold; and whether an ended
/// one was asked about again since the profile opened.
#[derive(Default)]
pub(super) struct Session {
    kept: parking_lot::Mutex<Option<Option<Kept>>>,
    rechecked: std::sync::atomic::AtomicBool,
    /// Held across a rotation: calls that find the session due together rotate it once.
    rotating: futures_util::lock::Mutex<()>,
}

pub(super) fn unix_now() -> u64 {
    (crate::db::now_ms() / 1000).max(0) as u64
}

fn stored(db: &rusqlite::Connection) -> Option<Kept> {
    crate::db::kv_get(db, KEY).ok().flatten().and_then(|j| serde_json::from_str(&j).ok())
}

/// Whether YouTube ended the session kept in `core`'s rows.
pub(super) fn ended(core: &crate::Core) -> bool {
    stored(&core.db.lock()).is_some_and(|k| k.ended)
}

impl State {
    /// The kept session, read from the profile's rows on first use with the kept visitor id; signed in,
    /// its cookies are the requests'.
    fn kept(&self, c: &Client) -> Option<Kept> {
        let mut s = self.session.kept.lock();
        if s.is_none() {
            let db = c.core.db.lock();
            self.api.set_visitor(crate::db::kv_get(&db, VISITOR).ok().flatten());
            let kept = stored(&db);
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
        if kept.as_ref().is_some_and(|k| k.ended) {
            c.core.set_rules(rules(false));
        }
        *self.session.kept.lock() = Some(kept);
    }

    /// Before a call: a signed-in session last rotated [`yt::auth::ROTATE_EVERY`] ago or more is rotated
    /// first, unless another process using the profile has rotated it since; a failure that is not a
    /// refusal leaves it for the next call. An ended session is asked about once a profile opening, and
    /// taken up again if YouTube still knows its cookies.
    pub(super) async fn fresh_session(&self, c: &Client, now: u64) {
        let Some(kept) = self.kept(c) else { return };
        if kept.ended {
            return self.recheck(c, kept).await;
        }
        if !yt::auth::rotation_due(kept.rotated_at, now) {
            return;
        }
        let _rotating = self.session.rotating.lock().await;
        let Some(mut kept) = self.kept(c).filter(|k| !k.ended && yt::auth::rotation_due(k.rotated_at, now)) else { return };
        let latest = stored(&c.core.db.lock());
        if let Some(newer) = latest.filter(|l| *l != kept) {
            self.api.set_cookies((!newer.ended).then(|| yt::Cookies::parse(&newer.cookies)));
            *self.session.kept.lock() = Some(Some(newer.clone()));
            if newer.ended {
                c.core.set_rules(rules(false));
            }
            if newer.ended || !yt::auth::rotation_due(newer.rotated_at, now) {
                return;
            }
            kept = newer;
        }
        match self.api.rotate(&Wire(&*c.transport)).await {
            Ok(_) => {
                self.api.take_changed();
                let cookies = self.api.cookies().map_or(kept.cookies, |c| c.header());
                self.keep(c, Some(Kept { cookies, rotated_at: now, ended: false, quiet: false }));
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

    /// An ended session's cookies asked about once: still the account's, it is signed in again.
    async fn recheck(&self, c: &Client, kept: Kept) {
        if self.session.rechecked.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        // Asked apart: the profile's other calls go on as a visitor's meanwhile.
        let probe = yt::InnerTube::default();
        probe.set_cookies(Some(yt::Cookies::parse(&kept.cookies)));
        match probe.account_name(&Wire(&*c.transport)).await {
            Ok(_) => {
                let cookies = probe.cookies().map_or(kept.cookies, |c| c.header());
                self.api.set_cookies(Some(yt::Cookies::parse(&cookies)));
                self.keep(c, Some(Kept { cookies, rotated_at: unix_now(), ended: false, quiet: false }));
                c.core.set_rules(rules(true));
            }
            Err(e) => crate::alog::info(&format!("YouTube session: still ended: {}", net_error(e))),
        }
    }

    /// After a call: a visitor id and cookies YouTube gave are kept at once; a session it refused is
    /// marked ended, with the cookies it last had.
    pub(super) fn after<T>(&self, c: &Client, r: yt::Result<T, crate::transport::NetError>) -> NetResult<T> {
        if let Some(visitor) = self.api.take_new_visitor() {
            if let Err(e) = crate::db::kv_put(&c.core.db.lock(), VISITOR, &visitor) {
                crate::alog::info(&format!("YouTube visitor id: could not keep it: {e}"));
            }
        }
        if let Some(kept) = self.kept(c).filter(|k| !k.ended) {
            if matches!(r, Err(yt::Error::SignedOut)) {
                let cookies = self.api.cookies().map_or(kept.cookies, |c| c.header());
                self.api.set_cookies(None);
                self.keep(c, Some(Kept { cookies, ended: true, ..kept }));
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
                self.keep(c, Some(Kept { cookies, rotated_at: unix_now(), ended: false, quiet: false }));
                c.core.set_rules(rules(true));
                SignInResult::SignedIn { name }
            }
            Err(e) => {
                api.set_cookies(before);
                match e {
                    yt::Error::SignedOut | yt::Error::Refused | yt::Error::Status(401 | 403) => SignInResult::Refused,
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

    /// An ended session not yet asked to be quiet: the client reminds to sign in again at start.
    pub(super) fn reminds(&self, c: &Client) -> bool {
        self.kept(c).is_some_and(|k| k.ended && !k.quiet)
    }

    /// Stops the reminder until the account is signed in, and its session ends, again.
    pub(super) fn stop_reminding(&self, c: &Client) {
        if let Some(k) = self.kept(c).filter(|k| k.ended) {
            self.keep(c, Some(Kept { quiet: true, ..k }));
        }
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
        serde_json::to_string(&Kept { cookies: cookies.into(), rotated_at: unix_now() - ago, ended: false, quiet: false }).unwrap()
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
