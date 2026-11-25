//! The Pair a device sheet: which of a device's two roles this computer takes (showing its code, or
//! taking another device's), and what the core's pairing (`PairingView`) shows for it. The core pairs;
//! this only words and draws it.

use nori_core::sync::PairingView;
use slint::{ComponentHandle, Image, ModelRc, SharedString, VecModel};

use crate::session::Msg;
use crate::{words, AppWindow, PairSheet};

/// What the sheet draws, as app.slint numbers its stages.
const WAITING: i32 = 0;
const CODE: i32 = 1;
const TAKING: i32 = 2;
const CONFIRM: i32 = 3;
const DONE: i32 = 4;
const STOPPED: i32 = 5;

/// What sync tells the desktop, as the window's messages.
pub struct Told {
    pub root: std::sync::Arc<nori_host::desktop::Root>,
    pub tx: crate::session::Tx,
}

impl nori_core::sync::SyncClient for Told {
    fn here(&self) -> nori_core::sync::Here {
        self.root.plugins().kinds.sync_here()
    }

    fn profiles(&self, synced: nori_core::sync::ProfilesSynced) {
        nori_host::sync::profiles_synced(self.root.data(), &synced);
        self.tx.send(Msg::Synced(synced));
    }

    fn install(&self, plugins: Vec<nori_core::sync::NeededPlugin>) {
        self.tx.send(Msg::SyncInstall(plugins));
    }

    fn event(&self, event: nori_core::sync::SyncEvent) {
        self.tx.send(Msg::SyncEvent(event));
    }

    fn changed(&self) {
        self.tx.send(Msg::SyncChanged);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    /// The new device: it shows its code.
    #[default]
    Show,
    /// The device with the profiles: it takes the other's code.
    Take,
}

/// What the sheet shows of a pairing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shown {
    pub stage: i32,
    pub line: String,
    pub error: String,
    pub link: String,
    pub code_words: String,
    pub question: String,
    pub leaves: String,
    pub asks_keep: bool,
    pub again: String,
}

#[derive(Default)]
pub struct Sheet {
    pub role: Role,
    /// The devices waiting nearby, listed while taking a code.
    pub nearby: Vec<nori_core::sync::NearbyDevice>,
    /// The new device's name from the confirmation, for the line once it is paired.
    asked: Option<String>,
    /// The link the QR code was drawn for, and the code.
    qr: (String, Image),
    stage: i32,
}

impl Sheet {
    pub fn new(role: Role) -> Sheet {
        Sheet { role, ..Sheet::default() }
    }

    /// What `view` shows in this role.
    pub fn shown(&mut self, view: &PairingView) -> Shown {
        let waiting = |line: &str| Shown { stage: WAITING, line: line.into(), ..Shown::default() };
        match (self.role, view) {
            (_, PairingView::Confirm { confirmation: c }) => {
                self.asked = Some(c.name.clone());
                Shown {
                    stage: CONFIRM,
                    question: words::give_sign_ins(&c.name, c.profiles),
                    leaves: c.leaves.map(|n| words::leaves_group(&c.name, n)).unwrap_or_default(),
                    code_words: c.words.as_ref().map(|w| w.join(" ")).unwrap_or_default(),
                    asks_keep: c.asks_keep_copy,
                    ..Shown::default()
                }
            }
            (_, PairingView::Sent) => Shown { stage: DONE, line: words::pair_sent(self.asked.as_deref().unwrap_or_default()), ..Shown::default() },
            (_, PairingView::Paired) => Shown { stage: DONE, line: words::PAIR_JOINED.into(), ..Shown::default() },
            (Role::Show, PairingView::Code { link, words }) => Shown { stage: CODE, link: link.clone(), code_words: words.join(" "), ..Shown::default() },
            (Role::Show, PairingView::Idle | PairingView::Starting) => waiting(words::PAIR_REACHING_SERVER),
            (Role::Show, PairingView::Refused { reason }) => Shown { stage: STOPPED, line: words::pairing_refused(*reason, true), again: words::NEW_CODE.into(), ..Shown::default() },
            (Role::Take, PairingView::Starting) => waiting(words::PAIR_REACHING_DEVICE),
            (Role::Take, PairingView::Refused { reason }) => Shown { stage: TAKING, error: words::pairing_refused(*reason, false), ..Shown::default() },
            (Role::Take, PairingView::Idle | PairingView::Code { .. }) => Shown { stage: TAKING, ..Shown::default() },
        }
    }

    /// Draws `view` on the sheet. Keeping an encrypted copy is chosen again each time a confirmation comes.
    pub fn fill(&mut self, ui: &AppWindow, view: &PairingView) {
        let shown = self.shown(view);
        let g = ui.global::<PairSheet>();
        if shown.stage == CONFIRM && self.stage != CONFIRM {
            g.set_keep(true);
        }
        self.stage = shown.stage;
        if self.qr.0 != shown.link {
            self.qr = (shown.link.clone(), if shown.link.is_empty() { Image::default() } else { crate::jam::qr(&shown.link) });
        }
        g.set_role(match self.role {
            Role::Show => 0,
            Role::Take => 1,
        });
        g.set_stage(shown.stage);
        g.set_qr(self.qr.1.clone());
        g.set_link(shown.link.into());
        g.set_code_words(shown.code_words.into());
        g.set_line(shown.line.into());
        g.set_error(shown.error.into());
        g.set_question(shown.question.into());
        g.set_leaves(shown.leaves.into());
        g.set_asks_keep(shown.asks_keep);
        g.set_again(shown.again.into());
        let nearby: Vec<SharedString> = self.nearby.iter().map(|n| n.name.as_str().into()).collect();
        g.set_nearby(ModelRc::new(VecModel::from(nearby)));
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
        let mut sheet = Sheet::new(Role::Take);
        let asked = sheet.shown(&confirmation(Some(vec!["acorn", "bison", "cedar"]), Some(1), true));
        assert_eq!(asked.stage, CONFIRM);
        assert_eq!(asked.question, "Give ‘Pixel 8’ your sign-ins for 3 profiles?");
        assert_eq!(asked.code_words, "acorn bison cedar", "a link's or a nearby device's code shows the words");
        assert_eq!(asked.leaves, "Pixel 8 will stop syncing with its 1 other device.");
        assert!(asked.asks_keep);
        let by_camera = sheet.shown(&confirmation(None, None, false));
        assert_eq!((by_camera.code_words.as_str(), by_camera.leaves.as_str(), by_camera.asks_keep), ("", "", false), "a scanned code needs no words, and a device in no group leaves none");
        assert_eq!(sheet.shown(&PairingView::Sent).line, "Pixel 8 is paired.", "the name the confirmation gave");
    }

    #[test]
    fn each_role_shows_its_side_of_the_pairing() {
        let mut show = Sheet::default();
        let code = PairingView::Code { link: "nori://pair#v=1".into(), words: vec!["acorn".into(), "bison".into(), "cedar".into()] };
        assert_eq!((show.shown(&PairingView::Starting).stage, show.shown(&code).stage), (WAITING, CODE));
        assert_eq!(show.shown(&code).code_words, "acorn bison cedar", "the new device shows the words while it waits");
        let ran_out = show.shown(&PairingView::Refused { reason: PairingRefused::Expired });
        assert_eq!((ran_out.stage, ran_out.line.as_str(), ran_out.again.as_str()), (STOPPED, "The code ran out.", "New code"));
        assert_eq!(show.shown(&PairingView::Paired).line, "This computer is paired.");
        let mut take = Sheet::new(Role::Take);
        assert_eq!(take.shown(&PairingView::Idle).stage, TAKING);
        let refused = take.shown(&PairingView::Refused { reason: PairingRefused::NotACode });
        assert_eq!((refused.stage, refused.error.as_str()), (TAKING, "That isn’t a nori pairing code."), "the link stays to fix, under its refusal");
        assert_eq!(take.shown(&PairingView::Refused { reason: PairingRefused::Expired }).error, "The code ran out, so make a new one on the other device.");
    }
}
