//! The client of one profile over the platform's [`Transport`]: what its calls share (the core, the
//! profile's addresses, per-client state) and its writes. Reads and their store are in cache_policy.rs,
//! the source's own protocol in providers/.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::transport::{self, Transport};
use crate::Core;

pub use crate::providers::login_check;
pub use nori_net::requests::{NetProfile, NetResult, Starrable, Write};
pub(crate) use nori_net::requests::{blank, pairs};

/// The client for one server profile, over its core.
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct Client {
    pub(crate) core: Arc<Core>,
    pub(crate) transport: Arc<dyn Transport>,
    pub(crate) profile: RwLock<NetProfile>,
    /// Requests go to the profile's second address (stream quality is capped then).
    pub(crate) second: AtomicBool,
    /// State shared between lyrics lookups (failing services, YouTube matches).
    pub(crate) lyrics: nori_lyrics::services::LyricsMemory,
    pub(crate) prefetch: parking_lot::Mutex<crate::race::Prefetch>,
    /// Motion cover token and cached videos (motion.rs).
    pub(crate) motion: parking_lot::Mutex<crate::motion::Motion>,
    /// A replay of the queued writes is running; a second one would send them twice. Queuing a write and
    /// a replay finding nothing left happen under it, so a write queued as a replay ends is never left.
    pub(crate) replaying: parking_lot::Mutex<bool>,
    /// The car's folders last listed, for their later pages and the list a picked row plays (car.rs).
    pub(crate) car: parking_lot::Mutex<crate::car::Shown>,
    /// An AutoEQ index fetch is running; another asked meanwhile is dropped (profiles.rs).
    pub(crate) autoeq_fetching: AtomicBool,
    /// Told the profile's second address, so covers are keyed alike through both.
    covers: Arc<crate::covers::CoverNet>,
    /// Where the queue this device last handed the server stood, and the server's queue last offered
    /// here: neither is offered (`queue_offer`).
    pub(crate) queue_pushed: parking_lot::Mutex<Option<crate::library::QueuePlace>>,
    pub(crate) queue_seen: parking_lot::Mutex<Option<crate::library::QueuePlace>>,
}

/// The client of the server profile in use, as the platform switches profiles: where what outlives one
/// profile (a player, AutoMix's measuring) finds the client it works through now.
#[derive(Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct CurrentClient(RwLock<Option<Arc<Client>>>);

#[cfg_attr(feature = "ffi", uniffi::export)]
impl CurrentClient {
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn new() -> Arc<CurrentClient> {
        Arc::default()
    }

    /// `client` is the profile's in use from now on.
    pub fn set(&self, client: Arc<Client>) {
        *self.0.write() = Some(client);
    }
}

impl CurrentClient {
    /// The client in use; None before the first profile opens.
    pub fn get(&self) -> Option<Arc<Client>> {
        self.0.read().clone()
    }
}

impl Client {
    pub fn core(&self) -> &Arc<Core> {
        &self.core
    }

    /// The queue of this client's core.
    pub fn session(&self) -> &Arc<nori_queue::Session> {
        &self.core.session
    }

    /// The settings of this client's core.
    pub(crate) fn settings(&self) -> &crate::settings_store::Settings {
        &self.core.session.settings
    }

    /// Whether the network is metered now, as the platform says.
    pub fn metered(&self) -> bool {
        self.transport.network() == transport::Network::Metered
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn new(core: Arc<Core>, transport: Arc<dyn Transport>, covers: Arc<crate::covers::CoverNet>) -> Arc<Self> {
        Arc::new(Client { core, transport, covers, profile: RwLock::new(NetProfile::default()), second: AtomicBool::new(false), lyrics: Default::default(), prefetch: Default::default(), motion: Default::default(), replaying: Default::default(), car: Default::default(), autoeq_fetching: AtomicBool::new(false), queue_pushed: Default::default(), queue_seen: Default::default() })
    }

    /// Sets the profile's addresses, folder and bitrate cap.
    pub fn set_profile(&self, profile: NetProfile) {
        if !blank(&profile.alt_url) {
            self.covers.alike(&profile.url, &profile.alt_url);
        }
        *self.profile.write() = profile;
    }

    /// Whether requests go to the second address.
    pub fn on_second_address(&self) -> bool {
        self.second.load(Ordering::Relaxed)
    }

    /// Evicts kept pages whose key starts with one of `prefixes` ([`crate::browse::home_refresh_drops`]):
    /// a manual refresh.
    pub fn drop_cached(&self, prefixes: Vec<String>) -> NetResult<()> {
        for p in prefixes {
            self.core.cache_evict(p)?;
        }
        Ok(())
    }

    /// Sends one change to the profile's provider; once it took it (or queued it), the pages it changes
    /// are stale ([`crate::cache_policy::stale`]).
    pub async fn write(&self, w: Write) -> NetResult<()> {
        let stale = crate::cache_policy::stale(&w);
        self.core.provider().write(self, w).await?;
        for s in stale {
            self.core.cache_stale(s)?;
        }
        Ok(())
    }
}

pub use nori_db::{db_file_name, db_forget_server, DB_FILE};

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    use parking_lot::Mutex;

    use super::*;
    use crate::transport::{Exchange, FailureKind, TransportError, TransportResponse};
    use crate::ServerConfig;

    pub const OK: &str = r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#;

    type Answer = Result<(u16, Vec<u8>), FailureKind>;

    /// A scripted transport: answers in order, records every request.
    #[derive(Default)]
    pub struct Fake {
        pub answers: Mutex<VecDeque<Answer>>,
        pub asked: Mutex<Vec<(String, u32)>>,
        pub sent: Mutex<Vec<Exchange>>,
        pub switched: Mutex<u32>,
        /// Each request pends once before answering, as a real one would.
        pub pends: Mutex<bool>,
        pub metered: Mutex<bool>,
    }

    impl Fake {
        pub fn answer(&self, body: &str) {
            self.answers.lock().push_back(Ok((200, body.as_bytes().to_vec())));
        }
        pub fn fail(&self, kind: FailureKind) {
            self.answers.lock().push_back(Err(kind));
        }
        pub fn asked(&self) -> Vec<String> {
            self.asked.lock().iter().map(|(u, _)| u.clone()).collect()
        }
    }

    #[async_trait::async_trait]
    impl Transport for Fake {
        async fn get(&self, url: String, timeout_ms: u32) -> Result<TransportResponse, TransportError> {
            self.asked.lock().push((url, timeout_ms));
            if *self.pends.lock() {
                let mut pended = false;
                std::future::poll_fn(|cx| {
                    if std::mem::replace(&mut pended, true) {
                        return Poll::Ready(());
                    }
                    cx.waker().wake_by_ref();
                    Poll::Pending
                })
                .await;
            }
            match self.answers.lock().pop_front().unwrap_or(Err(FailureKind::Connect)) {
                Ok((status, body)) => Ok(TransportResponse { status, body, set_cookies: Vec::new() }),
                Err(kind) => Err(TransportError::Failed { kind, detail: Some(format!("{kind:?}")) }),
            }
        }
        async fn send(&self, request: Exchange) -> Result<TransportResponse, TransportError> {
            self.sent.lock().push(request.clone());
            self.get(request.url, request.timeout_ms).await
        }
        fn address_changed(&self) {
            *self.switched.lock() += 1;
        }
        fn network(&self) -> transport::Network {
            if *self.metered.lock() { transport::Network::Metered } else { transport::Network::Unmetered }
        }
    }

    /// Polls `f` to completion; the fake transport never pends.
    pub fn block<F: Future>(f: F) -> F::Output {
        let mut f = pin!(f);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    pub fn client(profile: NetProfile) -> (Arc<Client>, Arc<Fake>) {
        let core = Core::new(String::new(), "t".into(), Arc::default()).unwrap();
        core.configure(ServerConfig { url: profile.url.clone(), user: "u".into(), password: "p".into(), ..Default::default() }).unwrap();
        let fake = Arc::new(Fake::default());
        let c = Client::new(core, fake.clone(), Default::default());
        c.set_profile(profile);
        (c, fake)
    }

    pub fn two_addresses() -> NetProfile {
        NetProfile { url: "http://lan:4533".into(), alt_url: "https://wan.example".into(), ..Default::default() }
    }
}
