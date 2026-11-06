//! The engine's buffer while sound pages are in sight (`nori_core::tuning`).

use nori_core::tuning::{Buffer, SoundPage, TuningStep};

use crate::session::Parts;

impl Parts {
    /// Sound page `page` came in sight (`shown`) or left it.
    pub fn sound_page(&self, page: SoundPage, shown: bool) {
        let step = if shown { self.tuning.shown(page) } else { self.tuning.left() };
        self.tuned(step);
    }

    /// A change to what is heard ([`Parts::apply`]); on a sound page in sight, later ones are heard at
    /// once.
    pub(crate) fn sound_edited(&self) {
        self.tuned(self.tuning.edited());
    }

    fn tuned(&self, step: TuningStep) {
        if let Some(buffer) = step.set {
            self.engine.set_shallow(buffer == Buffer::Shallow);
        }
    }
}
