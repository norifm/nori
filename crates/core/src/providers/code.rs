//! Signing in with a code ([`SignIn::Code`](super::SignIn::Code)): the provider makes a code, the user
//! allows it in a browser of their own where they are signed in already, and the code is checked as
//! often as the provider says until it is allowed or expires. The checks run only while the client
//! shows the code: a [`CodeSignIn`] stopped or dropped leaves no timer behind.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::future::select;
use nori_tasks::{Notify, Tasks};
use nori_time::Duration;

use super::flow::SignInEvent;
use super::SignInResult;
use crate::client::Client;

/// Checks at least this far apart, whatever the provider says.
const LEAST_EVERY: Duration = Duration::from_secs(1);

/// A code shown to sign in with.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SignInCode {
    /// What the user sees on the page they allow it on.
    pub code: String,
    /// That page, opened in the system's browser.
    pub url: String,
}

/// A code as its provider made it ([`super::Provider::start_code`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedCode {
    pub shown: SignInCode,
    /// Between checks.
    pub every: Duration,
    /// How long it lasts; None when the provider says so itself ([`CodeCheck::Expired`]).
    pub lasts: Option<Duration>,
    /// The provider's own (a poll token), handed back with every check.
    pub state: String,
}

/// How a code stands ([`super::Provider::check_code`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeCheck {
    Waiting,
    Expired,
    /// Allowed, or refused: how signing in with it went.
    Done(SignInResult),
}

/// Told what a code sign-in comes to, on its own thread: [`SignInEvent::CodeShown`], then
/// [`SignInEvent::CodeExpired`] or [`SignInEvent::Answered`], each for the client's [`super::flow::SignInFlow`].
#[cfg_attr(feature = "ffi", uniffi::export(with_foreign))]
pub trait CodeHeard: Send + Sync {
    fn heard(&self, event: SignInEvent);
}

/// One code asked for and checked until it is allowed, expires, or this is stopped.
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct CodeSignIn {
    stop: Arc<Stop>,
}

#[derive(Default)]
struct Stop {
    stopped: AtomicBool,
    told: Notify,
}

impl CodeSignIn {
    /// A code sign-in whose waits are `tasks`' timers; its work ends at [`CodeSignIn::stop`].
    pub fn with_tasks(client: Arc<Client>, heard: Arc<dyn CodeHeard>, tasks: Arc<dyn Tasks>) -> Arc<CodeSignIn> {
        let stop = Arc::new(Stop::default());
        let stopped = stop.clone();
        let work = run(client, heard, tasks.clone());
        tasks.spawn(Box::pin(async move {
            let until = stopped.told.until(|| stopped.stopped.load(Ordering::Acquire));
            select(std::pin::pin!(work), std::pin::pin!(until)).await;
        }));
        Arc::new(CodeSignIn { stop })
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl CodeSignIn {
    /// Nothing more is checked or told: the client left the code.
    pub fn stop(&self) {
        self.stop.stopped.store(true, Ordering::Release);
        self.stop.told.notify();
    }
}

impl Drop for CodeSignIn {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Signs the profile's account in with a code ([`super::SignIn::Code`]) on a thread of its own,
    /// which ends with it; `heard` is told the code, then how it went.
    pub fn sign_in_with_code(self: Arc<Self>, heard: Arc<dyn CodeHeard>) -> Arc<CodeSignIn> {
        #[cfg(not(target_arch = "wasm32"))]
        let tasks: Arc<dyn Tasks> = Arc::new(nori_tasks::Native::new("nori-sign-in").expect("a thread for the sign-in"));
        #[cfg(target_arch = "wasm32")]
        let tasks: Arc<dyn Tasks> = Arc::new(nori_tasks::Local);
        CodeSignIn::with_tasks(self, heard, tasks)
    }
}

/// Asks for a code, says it, and checks it until it comes to something.
async fn run(client: Arc<Client>, heard: Arc<dyn CodeHeard>, tasks: Arc<dyn Tasks>) {
    let provider = client.core.provider();
    let code = match provider.start_code(&client).await {
        Ok(code) => code,
        Err(e) => return heard.heard(SignInEvent::Answered { result: SignInResult::Unreachable { reason: e.to_string() } }),
    };
    heard.heard(SignInEvent::CodeShown { code: code.shown.clone() });
    let every = code.every.max(LEAST_EVERY);
    let ends = code.lasts.map(|l| tasks.now_us() + l.as_micros() as i64);
    loop {
        let left = ends.map(|at| Duration::from_micros((at - tasks.now_us()).max(0) as u64));
        tasks.sleep(left.map_or(every, |l| l.min(every))).await;
        if ends.is_some_and(|at| tasks.now_us() >= at) {
            return heard.heard(SignInEvent::CodeExpired);
        }
        match provider.check_code(&client, &code).await {
            CodeCheck::Waiting => {}
            CodeCheck::Expired => return heard.heard(SignInEvent::CodeExpired),
            CodeCheck::Done(result) => return heard.heard(SignInEvent::Answered { result }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache_policy::Read;
    use crate::client::{NetResult, Write};
    use crate::providers::{Answer, Provider, ProviderInfo, SignIn};
    use crate::settings::{SavedServer, ServerKind};
    use crate::stream::{StreamQuality, StreamTarget};
    use crate::transport::Transport;
    use crate::{browse::ProfileRules, Core};
    use parking_lot::Mutex;
    use std::sync::mpsc;

    /// A provider whose code is allowed at its `allowed`th check (never: 0), or says it expired at its
    /// `expires`th; its checks and the state they were handed kept.
    #[derive(Default)]
    struct Coded {
        allowed: u32,
        expires: u32,
        lasts: Option<Duration>,
        unreachable: bool,
        checks: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl Provider for Coded {
        fn info(&self) -> ProviderInfo {
            ProviderInfo { kind: ServerKind::default(), fields: Vec::new(), sign_in: vec![SignIn::Code] }
        }
        fn ready(&self, _: &SavedServer) -> bool {
            true
        }
        async fn check(&self, _: Arc<dyn Transport>, profile: SavedServer) -> NetResult<SavedServer> {
            Ok(profile)
        }
        fn rules(&self, _: &SavedServer, _: bool) -> ProfileRules {
            crate::browse::profile_rules(false)
        }
        fn configure(&self, _: &Core, _: &SavedServer) -> nori_model::Result<()> {
            Ok(())
        }
        async fn read(&self, _: &Client, _: Read) -> NetResult<Option<Answer>> {
            Ok(None)
        }
        async fn write(&self, _: &Client, _: Write) -> NetResult<()> {
            Ok(())
        }
        fn stream(&self, _: &Client, id: String, _: StreamQuality) -> StreamTarget {
            StreamTarget { url: id.clone(), key: id }
        }
        fn cover_address(&self, _: &Core, cover: &str, _: u32) -> String {
            cover.into()
        }
        async fn start_code(&self, _: &Client) -> NetResult<StartedCode> {
            if self.unreachable {
                return Err(crate::transport::NetError::Http { status: 503 });
            }
            let shown = SignInCode { code: "AB12".into(), url: "https://coded.example/activate/AB12".into() };
            Ok(StartedCode { shown, every: Duration::from_secs(3), lasts: self.lasts, state: "poll-token".into() })
        }
        async fn check_code(&self, _: &Client, code: &StartedCode) -> CodeCheck {
            let mut checks = self.checks.lock();
            checks.push(code.state.clone());
            match checks.len() as u32 {
                n if n == self.allowed => CodeCheck::Done(SignInResult::SignedIn { name: "Cody".into() }),
                n if n == self.expires => CodeCheck::Expired,
                _ => CodeCheck::Waiting,
            }
        }
    }

    struct Heard(Mutex<mpsc::Sender<SignInEvent>>);

    impl CodeHeard for Heard {
        fn heard(&self, event: SignInEvent) {
            let _ = self.0.lock().send(event);
        }
    }

    fn client(provider: Arc<Coded>) -> Arc<Client> {
        let core = Core::new(String::new(), "coded".into(), Default::default()).unwrap();
        core.install(ServerKind::default(), provider);
        Client::new(core, Arc::new(crate::client::tests::Fake::default()), Default::default())
    }

    /// A code sign-in over `provider` on a simulated clock: what it hears, and the executor.
    fn started(provider: Arc<Coded>) -> (Arc<CodeSignIn>, mpsc::Receiver<SignInEvent>, Arc<nori_tasks::Native>) {
        let client = client(provider);
        let tasks = Arc::new(nori_tasks::Native::simulated("code", 0).unwrap());
        let (tx, rx) = mpsc::channel();
        let code = CodeSignIn::with_tasks(client, Arc::new(Heard(Mutex::new(tx))), tasks.clone());
        tasks.settle();
        (code, rx, tasks)
    }

    fn shown() -> SignInEvent {
        SignInEvent::CodeShown { code: SignInCode { code: "AB12".into(), url: "https://coded.example/activate/AB12".into() } }
    }

    /// Moves the clock to its next timer `n` times.
    fn advance(tasks: &nori_tasks::Native, n: usize) {
        for _ in 0..n {
            assert!(tasks.advance(), "a check is due");
            tasks.settle();
        }
    }

    #[test]
    fn a_provider_offering_a_code_first_opens_it() {
        let c = client(Arc::new(Coded::default()));
        assert_eq!(c.sign_in_due(Some(crate::providers::AfterAdding::SignIn)), Some(SignInEvent::Code), "added with its sign-in button");
        assert_eq!(c.sign_in_open(), Some(SignInEvent::Code), "the offer's sign-in");
    }

    #[test]
    fn a_code_is_shown_then_checked_every_few_seconds_until_allowed() {
        let provider = Arc::new(Coded { allowed: 3, ..Default::default() });
        let (_code, heard, tasks) = started(provider.clone());
        assert_eq!(heard.try_recv(), Ok(shown()));
        assert_eq!(tasks.waits(), [Duration::from_secs(3)], "the provider's pace");
        advance(&tasks, 2);
        assert_eq!(heard.try_recv(), Err(mpsc::TryRecvError::Empty), "still waiting");
        advance(&tasks, 1);
        assert_eq!(heard.try_recv(), Ok(SignInEvent::Answered { result: SignInResult::SignedIn { name: "Cody".into() } }));
        assert_eq!(*provider.checks.lock(), ["poll-token"; 3], "each check is handed the provider's state");
        assert!(tasks.waits().is_empty(), "nothing checks after");
    }

    #[test]
    fn stopped_it_checks_and_says_nothing_more() {
        let provider = Arc::new(Coded::default());
        let (code, heard, tasks) = started(provider.clone());
        advance(&tasks, 1);
        code.stop();
        tasks.settle();
        assert!(tasks.waits().is_empty(), "no timer is left");
        assert!(!tasks.advance());
        assert_eq!(provider.checks.lock().len(), 1);
        assert_eq!(heard.try_iter().collect::<Vec<_>>(), [shown()]);
    }

    #[test]
    fn dropped_it_stops_too() {
        let (code, _heard, tasks) = started(Arc::new(Coded::default()));
        drop(code);
        tasks.settle();
        assert!(tasks.waits().is_empty());
    }

    #[test]
    fn a_code_expires_when_its_provider_says_or_its_time_is_up() {
        // Said at the second check; or, lasting 7 s, checked at 3 s and 6 s and gone at 7 s.
        for (provider, timers) in [(Coded { expires: 2, ..Default::default() }, 2), (Coded { lasts: Some(Duration::from_secs(7)), ..Default::default() }, 3)] {
            let provider = Arc::new(provider);
            let (_code, heard, tasks) = started(provider.clone());
            advance(&tasks, timers);
            assert_eq!(heard.try_iter().collect::<Vec<_>>(), [shown(), SignInEvent::CodeExpired]);
            assert_eq!(provider.checks.lock().len(), 2);
            assert!(tasks.waits().is_empty());
        }
    }

    #[test]
    fn a_code_that_cant_be_had_is_a_sign_in_that_couldnt_ask() {
        let (_code, heard, tasks) = started(Arc::new(Coded { unreachable: true, ..Default::default() }));
        assert!(matches!(heard.try_recv(), Ok(SignInEvent::Answered { result: SignInResult::Unreachable { .. } })));
        assert!(tasks.waits().is_empty());
    }
}
