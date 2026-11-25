//! Pairing and profile sync on the terminal: what the core's sync (`Syncing`) tells the event loop,
//! what Your devices, the profiles and the Pair a device sheet show of it, and the sheet's stage for each
//! of the core's pairing views. The core pairs and syncs; this only words and draws it.

use std::sync::mpsc::Sender;
use std::sync::Arc;

use nori_core::sync::{MergeOffer, NearbyDevice, PairSide as Role, PairingView, Seen, SyncState};

use crate::backend::Msg;

/// What sync tells the terminal, as the event loop's messages.
pub struct Told {
    pub root: Arc<nori_host::desktop::Root>,
    pub tx: Sender<Msg>,
}

impl nori_core::sync::SyncClient for Told {
    fn here(&self) -> nori_core::sync::Here {
        self.root.plugins().kinds.sync_here()
    }

    fn profiles(&self, synced: nori_core::sync::ProfilesSynced) {
        nori_host::sync::profiles_synced(self.root.data(), &synced);
        let _ = self.tx.send(Msg::Synced(synced));
    }

    fn event(&self, event: nori_core::sync::SyncEvent) {
        let _ = self.tx.send(Msg::SyncEvent(event));
    }

    fn changed(&self) {
        let _ = self.tx.send(Msg::SyncChanged);
    }
}

/// Where sync stands, as the screens show it, read again whenever sync says something changed.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncShown {
    /// In a group: profiles sync and Reset pairing is offered.
    pub paired: bool,
    pub state: SyncState,
    pub pairing: PairingView,
    /// The group's other devices.
    pub devices: Vec<Seen>,
    pub keeps_copy: bool,
    /// Profiles that may be one.
    pub offers: Vec<MergeOffer>,
    /// The saved profiles that sync: removing one removes it from every paired device.
    pub synced: Vec<String>,
    /// The plugins synced profiles need, by name, each with whether it was installed from a file on the
    /// other device (no repo to install it from).
    pub needed: Vec<(String, bool)>,
    /// Install is under way.
    pub installing: bool,
    /// A backup restored here had a pairing, which it doesn't bring.
    pub not_restored: bool,
    /// The devices waiting nearby with their pairing screen open.
    pub nearby: Vec<NearbyDevice>,
    /// This computer's name in the group.
    pub name: String,
    /// Unix ms, and the local time's offset from UTC in seconds, for when devices were seen.
    pub now_ms: i64,
    pub offset_s: i64,
}

impl Default for SyncShown {
    fn default() -> SyncShown {
        SyncShown {
            paired: false,
            state: SyncState::Idle,
            pairing: PairingView::Idle,
            devices: Vec::new(),
            keeps_copy: false,
            offers: Vec::new(),
            synced: Vec::new(),
            needed: Vec::new(),
            installing: false,
            not_restored: false,
            nearby: Vec::new(),
            name: String::new(),
            now_ms: 0,
            offset_s: 0,
        }
    }
}

/// What the sheet shows of a pairing.
#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    /// Reaching the server.
    Waiting,
    /// This computer's code: its link (the QR code's text), its words, and a refusal it goes on waiting
    /// after.
    Code { link: String, words: String, note: Option<String> },
    /// The code shown here stopped working: why, with New code.
    Stopped(String),
    /// Taking the other device's code, under why the last one was refused.
    Taking(Option<String>),
    /// The question on the device with the profiles.
    Confirm { question: String, words: Option<String>, leaves: Option<String>, asks_keep: bool },
    /// Over: the sheet closes.
    Done,
}

/// What the sheet shows of `view` in `role`; `shown` is the code shown last, which stays while the new
/// device waits on after a refusal.
pub fn stage(role: Role, view: &PairingView, shown: Option<&(String, Vec<String>)>) -> Stage {
    use nori_core::sync::PairingRefused as R;
    match (role, view) {
        (_, PairingView::Confirm { confirmation: c }) => Stage::Confirm {
            question: crate::text::give_sign_ins(&c.name, c.profiles),
            words: c.words.as_ref().map(|w| w.join(" ")),
            leaves: c.leaves.map(|n| crate::text::leaves_group(&c.name, n)),
            asks_keep: c.asks_keep_copy,
        },
        (_, PairingView::Sent | PairingView::Paired) => Stage::Done,
        (Role::Show, PairingView::Code { link, words }) => Stage::Code { link: link.clone(), words: words.join(" "), note: None },
        (Role::Show, PairingView::Refused { reason: reason @ (R::Tampered | R::Spent) }) if shown.is_some() => {
            let (link, words) = shown.expect("checked");
            Stage::Code { link: link.clone(), words: words.join(" "), note: Some(crate::text::pairing_refused(*reason, true)) }
        }
        (Role::Show, PairingView::Refused { reason }) => Stage::Stopped(crate::text::pairing_refused(*reason, true)),
        (Role::Show, PairingView::Idle | PairingView::Starting) | (Role::Take, PairingView::Starting) => Stage::Waiting,
        (Role::Take, PairingView::Refused { reason }) => Stage::Taking(Some(crate::text::pairing_refused(*reason, false))),
        (Role::Take, PairingView::Idle | PairingView::Code { .. }) => Stage::Taking(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_core::sync::{Confirmation, PairingRefused};

    fn confirmation(words: Option<Vec<&str>>, leaves: Option<u32>, asks_keep_copy: bool) -> PairingView {
        let words = words.map(|w| w.into_iter().map(String::from).collect());
        PairingView::Confirm { confirmation: Confirmation { name: "Pixel 8".into(), profiles: 3, leaves, words, asks_keep_copy } }
    }

    #[test]
    fn the_confirmation_says_the_cores_data() {
        let asked = stage(Role::Take, &confirmation(Some(vec!["acorn", "bison", "cedar"]), Some(1), true), None);
        assert_eq!(
            asked,
            Stage::Confirm {
                question: "Give ‘Pixel 8’ your sign-ins for 3 profiles?".into(),
                words: Some("acorn bison cedar".into()),
                leaves: Some("Pixel 8 will stop syncing with its 1 other device.".into()),
                asks_keep: true
            }
        );
        let Stage::Confirm { words, leaves, asks_keep, .. } = stage(Role::Take, &confirmation(None, None, false), None) else { panic!("a question") };
        assert_eq!((words, leaves, asks_keep), (None, None, false), "a scanned code needs no words, and a device in no group leaves none");
    }

    #[test]
    fn each_role_shows_its_side_of_the_pairing() {
        let words = vec!["acorn".to_string(), "bison".into(), "cedar".into()];
        let code = PairingView::Code { link: "nori://pair#v=1".into(), words: words.clone() };
        assert_eq!(stage(Role::Show, &PairingView::Starting, None), Stage::Waiting);
        assert_eq!(stage(Role::Show, &code, None), Stage::Code { link: "nori://pair#v=1".into(), words: "acorn bison cedar".into(), note: None });
        let shown = ("nori://pair#v=1".to_string(), words);
        let tampered = stage(Role::Show, &PairingView::Refused { reason: PairingRefused::Tampered }, Some(&shown));
        assert_eq!(tampered, Stage::Code { link: shown.0.clone(), words: "acorn bison cedar".into(), note: Some("A key came that doesn't fit this code. Still waiting.".into()) }, "the code stays while it waits on");
        assert_eq!(stage(Role::Show, &PairingView::Refused { reason: PairingRefused::Expired }, Some(&shown)), Stage::Stopped("This code ran out.".into()));
        assert_eq!(stage(Role::Take, &PairingView::Idle, None), Stage::Taking(None));
        assert_eq!(stage(Role::Take, &PairingView::Refused { reason: PairingRefused::NotACode }, None), Stage::Taking(Some("This isn't a nori pairing code.".into())), "the link stays to fix, under its refusal");
        assert_eq!(stage(Role::Take, &PairingView::Refused { reason: PairingRefused::Expired }, None), Stage::Taking(Some("This code ran out. Make a new one on the other device.".into())));
        assert_eq!(stage(Role::Take, &PairingView::Sent, None), Stage::Done);
    }
}
