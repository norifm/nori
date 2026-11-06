//! Adding a profile and signing its account in, as every client walks them: which page shows, what is
//! on its way, and what follows. The clients draw the pages and word them.

use super::{AccountState, AfterAdding, FirstStep, ProviderInfo, SignInResult, SignInStep};
use crate::client::Client;
use crate::settings::ServerKind;

/// Where adding (or editing) a profile is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AddFlow {
    pub page: AddPage,
    pub send: AddSend,
}

/// The page adding a profile shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AddPage {
    /// The first screen: what a new profile plays from.
    #[default]
    Choosing,
    /// Plugin `plugin`, which a repo offers, its page opened from the first screen: once installed and
    /// on, its kind is chosen (`Chose`).
    Offered { plugin: String },
    /// `kind` asks nothing and is being added at once: the first screen stays unless that fails.
    Adding { kind: ServerKind },
    /// `kind`'s page, as its first step says (its form, or its sign-in and guest buttons); after adding
    /// at once failed, `AddNow` with only why. A saved profile edited is a `Form`.
    Provider { kind: ServerKind, step: FirstStep },
}

/// The profile sent to be checked and saved.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AddSend {
    #[default]
    Idle,
    /// On its way; `added` says how a new profile was asked for (None: a saved one, saved over itself).
    Sending { added: Option<AfterAdding> },
    Failed { why: AddFailure },
}

/// Why a profile was not added.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AddFailure {
    /// The form misses what its provider needs.
    Missing,
    /// The server or the provider refused it; `said` is the client's own words for why.
    Refused { said: String },
}

/// What happened on the pages of adding a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AddEvent {
    /// The first screen opened, afresh.
    Start,
    /// A kind chosen on the first screen.
    Chose { info: ProviderInfo },
    /// A saved profile of `kind` opened to be edited.
    Edit { kind: ServerKind },
    /// The page of `plugin`, offered by a repo, opened from the first screen to install it.
    OfferOpened { plugin: String },
    /// Back: a kind's page, or an offered plugin's, goes back to the first screen.
    Back,
    /// The form sent (`ready`: its provider takes it), for a new profile asked for as `added`.
    Send { ready: bool, added: Option<AfterAdding> },
    /// The profile sent was refused, in the client's words.
    Refused { said: String },
    /// A profile saved or added would not open, in the client's words: the first screen, with why.
    Unopened { said: String },
    /// The profile sent was checked and saved; the pages start afresh.
    Accepted,
}

/// What a step asks of the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum AddDo {
    Nothing,
    /// Send the form now, as its own button does ([`AfterAdding::Remind`]): its kind asks nothing.
    Connect,
    /// Send the form's profile, as a new one asked for as `added`, or a saved one.
    Send { added: Option<AfterAdding> },
    /// The profile sent was saved: the pages close, and the session opened goes on with `added`
    /// ([`Client::sign_in_due`]).
    Open { added: Option<AfterAdding> },
}

impl AddFlow {
    /// The profile being added or edited is on its way.
    pub fn busy(&self) -> bool {
        matches!(self.send, AddSend::Sending { .. })
    }

    /// The kind whose page shows, or is being added.
    pub fn kind(&self) -> Option<&ServerKind> {
        match &self.page {
            AddPage::Adding { kind } | AddPage::Provider { kind, .. } => Some(kind),
            AddPage::Choosing | AddPage::Offered { .. } => None,
        }
    }

    /// Whether the first screen shows (also while a kind that asks nothing is being added).
    pub fn choosing(&self) -> bool {
        matches!(self.page, AddPage::Choosing | AddPage::Adding { .. })
    }

    /// The step of the kind's page, once one shows.
    pub fn step(&self) -> Option<FirstStep> {
        match self.page {
            AddPage::Provider { step, .. } => Some(step),
            _ => None,
        }
    }

    /// The failure shown, if the last send failed.
    pub fn failure(&self) -> Option<&AddFailure> {
        match &self.send {
            AddSend::Failed { why } => Some(why),
            _ => None,
        }
    }

    /// Takes `event`; nothing changes while a profile is on its way but its answer.
    pub fn on(&mut self, event: AddEvent) -> AddDo {
        if self.busy() && !matches!(event, AddEvent::Refused { .. } | AddEvent::Unopened { .. } | AddEvent::Accepted) {
            return AddDo::Nothing;
        }
        match event {
            AddEvent::Start => *self = AddFlow::default(),
            AddEvent::Chose { info } => return self.chose(&info),
            AddEvent::Edit { kind } => *self = AddFlow { page: AddPage::Provider { kind, step: FirstStep::Form }, send: AddSend::Idle },
            AddEvent::OfferOpened { plugin } => {
                if self.page == AddPage::Choosing {
                    *self = AddFlow { page: AddPage::Offered { plugin }, send: AddSend::Idle };
                }
            }
            AddEvent::Back => {
                if matches!(self.page, AddPage::Provider { .. } | AddPage::Offered { .. }) {
                    *self = AddFlow::default();
                }
            }
            AddEvent::Send { ready, added } => {
                if !ready {
                    self.failed(AddFailure::Missing);
                    return AddDo::Nothing;
                }
                self.send = AddSend::Sending { added };
                return AddDo::Send { added };
            }
            AddEvent::Refused { said } => {
                if self.busy() {
                    self.failed(AddFailure::Refused { said });
                }
            }
            AddEvent::Unopened { said } => *self = AddFlow { page: AddPage::Choosing, send: AddSend::Failed { why: AddFailure::Refused { said } } },
            AddEvent::Accepted => {
                if let AddSend::Sending { added } = self.send {
                    *self = AddFlow::default();
                    return AddDo::Open { added };
                }
            }
        }
        AddDo::Nothing
    }

    /// A kind chosen: its page, or, asking nothing, added at once.
    fn chose(&mut self, info: &ProviderInfo) -> AddDo {
        let (kind, step) = (info.kind.clone(), info.first_step());
        if step == FirstStep::AddNow {
            *self = AddFlow { page: AddPage::Adding { kind }, send: AddSend::Idle };
            return AddDo::Connect;
        }
        *self = AddFlow { page: AddPage::Provider { kind, step }, send: AddSend::Idle };
        AddDo::Nothing
    }

    /// Adding at once failed: its kind's page shows, with why.
    fn failed(&mut self, why: AddFailure) {
        if let AddPage::Adding { kind } = &self.page {
            self.page = AddPage::Provider { kind: kind.clone(), step: FirstStep::AddNow };
        }
        self.send = AddSend::Failed { why };
    }
}

/// [`AddFlow::on`], owned: the flow after `event`, and what to do.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct AddStep {
    pub flow: AddFlow,
    pub then: AddDo,
}

#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn add_flow_on(flow: AddFlow, event: AddEvent) -> AddStep {
    let mut flow = flow;
    let then = flow.on(event);
    AddStep { flow, then }
}

/// What is shown for signing the profile's account in, one at a time, and the last sign-in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SignInFlow {
    pub shown: SignInShown,
    pub attempt: SignInAttempt,
}

/// What is shown for signing in: nothing, the offer, or the provider's sign-in page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SignInShown {
    #[default]
    Closed,
    /// Signing in offered, which can be skipped: a profile just added, the account's row, or a session
    /// the provider `ended`.
    Offered { ended: bool },
    /// The provider's page, at `start` where its plugin asked for it there; None: the provider's own.
    Page { start: Option<String> },
}

/// The last sign-in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SignInAttempt {
    /// None, or its failure left for a fresh try.
    #[default]
    Idle,
    Busy,
    Failed { result: SignInResult },
}

/// What happened about signing the account in.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SignInEvent {
    /// Signing in offered: a profile just added, the account's row or a seal tapped while signed out, a
    /// session the provider `ended`. The page stays if it is open.
    Offer { ended: bool },
    /// The provider's page opens, at `start` where its plugin asked; its last failure is left.
    Open { start: Option<String> },
    /// The page closed, or the offer turned down.
    Close,
    /// The page's failure left, for a fresh try.
    Retry,
    /// An answer sent (the page's cookies, or what was pasted), or the page opened in a window of its
    /// own, whose cookies are the answer.
    Sent,
    /// The answer could not be sent (the window would not open): nothing is on its way.
    Unsent,
    /// How the answer went: signed in, nothing more shows; a failure stays on the page.
    Answered { result: SignInResult },
    /// Signed out: nothing shown.
    SignedOut,
}

impl SignInFlow {
    pub fn busy(&self) -> bool {
        self.attempt == SignInAttempt::Busy
    }

    /// Takes `event`. Returns false for an answer sent while one is on its way: the client does not send it.
    pub fn on(&mut self, event: SignInEvent) -> bool {
        match event {
            SignInEvent::Offer { ended } => {
                if !matches!(self.shown, SignInShown::Page { .. }) {
                    self.shown = SignInShown::Offered { ended };
                }
            }
            SignInEvent::Open { start } => {
                self.shown = SignInShown::Page { start };
                self.left();
            }
            SignInEvent::Close => {
                self.shown = SignInShown::Closed;
                self.left();
            }
            SignInEvent::Retry => self.left(),
            SignInEvent::Sent => {
                if self.busy() {
                    return false;
                }
                self.attempt = SignInAttempt::Busy;
            }
            SignInEvent::Unsent => {
                if self.busy() {
                    self.attempt = SignInAttempt::Idle;
                }
            }
            SignInEvent::Answered { result } => {
                if let SignInResult::SignedIn { .. } = result {
                    *self = SignInFlow::default();
                } else {
                    self.attempt = SignInAttempt::Failed { result };
                }
            }
            SignInEvent::SignedOut => *self = SignInFlow::default(),
        }
        true
    }

    /// A failure left; an answer on its way goes on.
    fn left(&mut self) {
        if let SignInAttempt::Failed { .. } = self.attempt {
            self.attempt = SignInAttempt::Idle;
        }
    }
}

/// [`SignInFlow::on`], owned: the flow after `event`, and whether to send the answer it was.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SignInStepped {
    pub flow: SignInFlow,
    pub send: bool,
}

#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn sign_in_flow_on(flow: SignInFlow, event: SignInEvent) -> SignInStepped {
    let mut flow = flow;
    let send = flow.on(event);
    SignInStepped { flow, send }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// What to show about the account now: `added` says how a profile just opened was added; otherwise
    /// (a saved profile opened, or its account changed under it) signing in again is offered when the
    /// provider ended its session, until told to stop reminding. A profile added as a guest stops the
    /// reminder here.
    pub fn sign_in_due(&self, added: Option<AfterAdding>) -> Option<SignInEvent> {
        let step = match added {
            Some(after) => self.core.provider().info().after_adding(after),
            None => self.reminds_to_sign_in().then_some(SignInStep::Offer),
        }?;
        match step {
            SignInStep::Offer => Some(SignInEvent::Offer { ended: added.is_none() && self.account() == AccountState::Ended }),
            SignInStep::Open => Some(SignInEvent::Open { start: None }),
            SignInStep::StopReminding => {
                self.stop_reminding();
                None
            }
        }
    }

    /// Signing in offered when asked: the account's row, a seal tapped while signed out.
    pub fn sign_in_offer(&self) -> SignInEvent {
        SignInEvent::Offer { ended: self.account() == AccountState::Ended }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{FieldSpec, FormField, SignIn};

    fn info(kind: &str, fields: &[FormField], page: bool) -> ProviderInfo {
        ProviderInfo {
            kind: ServerKind::Plugin { id: kind.into() },
            fields: fields.iter().map(|&what| FieldSpec { what, advanced: false }).collect(),
            sign_in: if page { vec![SignIn::WebPage { start_url: "https://x".into() }] } else { Vec::new() },
        }
    }

    fn kind(id: &str) -> ServerKind {
        ServerKind::Plugin { id: id.into() }
    }

    #[test]
    fn a_kind_with_a_form_shows_it_and_back_chooses_again() {
        let mut f = AddFlow::default();
        assert_eq!(f.on(AddEvent::Chose { info: info("a", &[FormField::Address], false) }), AddDo::Nothing);
        assert_eq!(f.page, AddPage::Provider { kind: kind("a"), step: FirstStep::Form });
        assert_eq!(f.on(AddEvent::Send { ready: false, added: Some(AfterAdding::Remind) }), AddDo::Nothing);
        assert_eq!(f.failure(), Some(&AddFailure::Missing));
        f.on(AddEvent::Back);
        assert_eq!(f, AddFlow::default(), "the failure goes with the page");
    }

    #[test]
    fn a_kind_asking_nothing_is_added_at_once_and_its_page_shows_only_a_failure() {
        let mut f = AddFlow::default();
        assert_eq!(f.on(AddEvent::Chose { info: info("a", &[], false) }), AddDo::Connect);
        assert_eq!(f.on(AddEvent::Send { ready: true, added: Some(AfterAdding::Remind) }), AddDo::Send { added: Some(AfterAdding::Remind) });
        assert!(f.choosing() && f.busy());
        assert_eq!(f.on(AddEvent::Chose { info: info("b", &[FormField::Address], false) }), AddDo::Nothing, "nothing else while it is on its way");
        f.on(AddEvent::Refused { said: "no".into() });
        assert_eq!(f.page, AddPage::Provider { kind: kind("a"), step: FirstStep::AddNow });
        assert_eq!(f.failure(), Some(&AddFailure::Refused { said: "no".into() }));
        f.on(AddEvent::Unopened { said: "gone".into() });
        assert!(f.choosing() && f.failure().is_some(), "a profile that would not open: the first screen, with why");
    }

    #[test]
    fn a_send_goes_once_and_carries_how_it_was_asked_for_only_to_its_answer() {
        let mut f = AddFlow::default();
        f.on(AddEvent::Chose { info: info("a", &[], true) });
        assert_eq!(f.step(), Some(FirstStep::Account));
        assert_eq!(f.on(AddEvent::Send { ready: true, added: Some(AfterAdding::SignIn) }), AddDo::Send { added: Some(AfterAdding::SignIn) });
        assert_eq!(f.on(AddEvent::Send { ready: true, added: Some(AfterAdding::Guest) }), AddDo::Nothing, "pressed twice");
        f.on(AddEvent::Refused { said: "down".into() });
        assert_eq!(f.on(AddEvent::Send { ready: true, added: None }), AddDo::Send { added: None }, "tried again");
        assert_eq!(f.on(AddEvent::Accepted), AddDo::Open { added: None }, "the failed send's way of asking is gone");
        assert_eq!(f, AddFlow::default());
    }

    #[test]
    fn an_offered_plugins_page_opens_only_from_the_first_screen() {
        let mut f = AddFlow::default();
        f.on(AddEvent::OfferOpened { plugin: "p".into() });
        assert_eq!(f.page, AddPage::Offered { plugin: "p".into() });
        f.on(AddEvent::Back);
        assert_eq!(f, AddFlow::default(), "its page left");
        f.on(AddEvent::OfferOpened { plugin: "p".into() });
        f.on(AddEvent::Chose { info: info("p", &[FormField::Address], false) });
        assert_eq!(f.page, AddPage::Provider { kind: kind("p"), step: FirstStep::Form }, "installed, it is chosen");
        f.on(AddEvent::OfferOpened { plugin: "q".into() });
        assert_eq!(f.kind(), Some(&kind("p")));
    }

    #[test]
    fn a_saved_profile_edited_shows_its_form() {
        let mut f = AddFlow::default();
        f.on(AddEvent::Edit { kind: kind("a") });
        assert_eq!((f.choosing(), f.step()), (false, Some(FirstStep::Form)));
    }

    #[test]
    fn one_sign_in_dialog_at_a_time() {
        let mut s = SignInFlow::default();
        s.on(SignInEvent::Open { start: Some("https://plugin".into()) });
        s.on(SignInEvent::Offer { ended: true });
        assert_eq!(s.shown, SignInShown::Page { start: Some("https://plugin".into()) }, "the offer waits for the page");
        s.on(SignInEvent::Close);
        s.on(SignInEvent::Offer { ended: true });
        assert_eq!(s.shown, SignInShown::Offered { ended: true });
        s.on(SignInEvent::Close);
        assert_eq!(s, SignInFlow::default());
    }

    #[test]
    fn a_sign_in_goes_once_and_its_failure_stays_on_the_page() {
        let mut s = SignInFlow::default();
        s.on(SignInEvent::Open { start: None });
        assert!(s.on(SignInEvent::Sent));
        assert!(!s.on(SignInEvent::Sent), "one on its way");
        s.on(SignInEvent::Answered { result: SignInResult::Refused });
        assert_eq!(s.attempt, SignInAttempt::Failed { result: SignInResult::Refused });
        assert_eq!(s.shown, SignInShown::Page { start: None });
        s.on(SignInEvent::Retry);
        assert_eq!(s.attempt, SignInAttempt::Idle);
        s.on(SignInEvent::Sent);
        s.on(SignInEvent::Open { start: None });
        assert!(s.busy(), "an answer on its way goes on");
        s.on(SignInEvent::Answered { result: SignInResult::SignedIn { name: "me".into() } });
        assert_eq!(s, SignInFlow::default(), "signed in, the page has done its work");
        s.on(SignInEvent::Offer { ended: false });
        s.on(SignInEvent::Sent);
        s.on(SignInEvent::Unsent);
        assert_eq!(s.attempt, SignInAttempt::Idle, "a window that would not open sent nothing");
        s.on(SignInEvent::Sent);
        s.on(SignInEvent::Answered { result: SignInResult::SignedIn { name: "me".into() } });
        assert_eq!(s.shown, SignInShown::Closed, "signed in from the offer, it closes too");
    }
}
