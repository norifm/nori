//! A saved profile's account: signing it in and out, and who it acts as, through a client of the profile's
//! own. The open session keeps one ([`crate::session::Handle::account`]); the account sheet opened on
//! another profile makes one for it (`desktop::Root::account`), so editing it opens nothing else.

use std::sync::Arc;

use nori_core::client::Client;
use nori_core::providers::{Identity, SignInResult};
use nori_core::settings::SavedServer;
use nori_core::settings_store::Settings;
use nori_core::transport::{block_on, NetError};

/// One profile's account, cheap to clone.
#[derive(Clone)]
pub struct Account(Arc<Parts>);

struct Parts {
    client: Arc<Client>,
    settings: Arc<Settings>,
    profile: String,
    /// Told when the profile's picture changed.
    pictured: Box<dyn Fn() + Send + Sync>,
    /// Plugins serve one profile at a time: the open session's client and profile, handed them back once
    /// this account, made apart from it, is done with them.
    give_back: Option<(Arc<Client>, SavedServer)>,
}

impl Drop for Parts {
    fn drop(&mut self) {
        if let Some((open, profile)) = self.give_back.take() {
            let _ = open.take_plugins_back(profile, self.client.core().kind());
        }
    }
}

impl Account {
    /// `profile`'s account through `client`; `pictured` hears its picture change. Made apart from the open
    /// session, `give_back` is that session's client and profile, handed the plugins back once this account
    /// is dropped.
    pub(crate) fn new(client: Arc<Client>, settings: Arc<Settings>, profile: String, pictured: Box<dyn Fn() + Send + Sync>, give_back: Option<(Arc<Client>, SavedServer)>) -> Account {
        Account(Arc::new(Parts { client, settings, profile, pictured, give_back }))
    }

    pub fn client(&self) -> &Arc<Client> {
        &self.0.client
    }

    /// Signs the account in with `answer` (cookies from its page, pasted or from the sign-in window).
    pub fn sign_in(&self, answer: String) -> SignInResult {
        let r = block_on(self.0.client.sign_in(answer));
        self.signed_in(&r);
        r
    }

    /// How a sign-in went, whichever way it went (a page, pasting, a code): signed in, the profile keeps
    /// the account's name, which a session opened on it reads as signed in, and its picture.
    pub fn signed_in(&self, r: &SignInResult) {
        if let SignInResult::SignedIn { name } = r {
            self.keep(|s| std::mem::replace(&mut s.user, name.clone()) != *name);
            self.keep_picture();
        }
    }

    /// Signs the account out here; the profile forgets its name and picture.
    pub fn sign_out(&self) {
        self.0.client.sign_out();
        self.keep(|s| !std::mem::take(&mut s.user).is_empty());
        if self.keep(|s| s.picture.take().is_some()) {
            (self.0.pictured)();
        }
    }

    /// Those the signed-in account may act as, its own first, the one it acts as chosen; empty when it is
    /// only itself.
    pub fn identities(&self) -> Result<Vec<Identity>, NetError> {
        block_on(self.0.client.identities())
    }

    /// Acts as [`Account::identities`]' entry `index` from now on; the profile keeps its picture.
    pub fn choose_identity(&self, index: u32) -> Result<(), NetError> {
        block_on(self.0.client.choose_identity(index))?;
        self.keep_picture();
        Ok(())
    }

    /// Keeps the picture of the account as it acts now on the profile, off this thread; `pictured` hears
    /// when it changed.
    pub(crate) fn keep_picture(&self) {
        let me = self.clone();
        crate::spawn("nori-picture", move || {
            let picture = block_on(me.0.client.account_picture());
            if me.keep(|s| std::mem::replace(&mut s.picture, picture.clone()) != picture) {
                (me.0.pictured)();
            }
        });
    }

    /// Changes the saved profile with `change`, kept when it says it changed something.
    fn keep(&self, change: impl FnOnce(&mut SavedServer) -> bool) -> bool {
        let settings = &self.0.settings;
        let Some(mut prefs) = settings.current() else { return false };
        let Some(s) = prefs.servers.iter_mut().find(|s| s.id == self.0.profile) else { return false };
        let changed = change(s);
        if changed {
            settings.put(prefs);
        }
        changed
    }
}
