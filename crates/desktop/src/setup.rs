//! The first run's questions as the desktop shows them (the core's `setup`): the page shown, the answers
//! given, and the covers the listening preview plays.

use nori_core::setup::{setup_answers, setup_covers, SetupAnswer, SetupStep};
use slint::{Image, ModelRc, SharedPixelBuffer, VecModel};

use crate::{words, SetupCard};

/// The side the preview's covers are decoded at, in pixels.
const COVER_PX: u32 = 240;

/// A first run under way: its questions, the one shown, the answer given to each so far, and the covers its
/// listening preview plays (the core's `setup_covers`), once decoded.
pub struct Setup {
    steps: Vec<SetupStep>,
    at: usize,
    answers: Vec<Option<SetupAnswer>>,
    /// 1 when the page shown came on, -1 when it came back.
    way: i32,
    covers: Vec<Image>,
}

impl Setup {
    pub fn new(steps: Vec<SetupStep>) -> Setup {
        let answers = vec![None; steps.len()];
        Setup { steps, at: 0, answers, way: 1, covers: Vec::new() }
    }

    /// The answer at `i` of the question shown, kept until another is picked; None for one it doesn't offer.
    pub fn answer(&mut self, i: usize) -> Option<SetupAnswer> {
        let answer = *setup_answers(self.steps[self.at]).get(i)?;
        self.answers[self.at] = Some(answer);
        Some(answer)
    }

    /// The next question; false past the last, where the run is over.
    pub fn next(&mut self) -> bool {
        self.way = 1;
        self.at += 1;
        self.at < self.steps.len()
    }

    pub fn back(&mut self) {
        self.way = -1;
        self.at = self.at.saturating_sub(1);
    }

    pub fn step(&self) -> SetupStep {
        self.steps[self.at]
    }

    /// The preview's covers, as [`decode_covers`] gives them.
    pub fn covers_decoded(&mut self, covers: Vec<Vec<u8>>) {
        self.covers = covers.iter().map(|px| Image::from_rgba8(SharedPixelBuffer::clone_from_slice(px, COVER_PX, COVER_PX))).collect();
    }

    /// Puts the page shown on the window.
    pub fn show(&self, ui: &crate::AppWindow) {
        let step = self.step();
        let answers = setup_answers(step);
        let chosen = self.answers[self.at];
        ui.set_setup_at(self.at as i32);
        ui.set_setup_count(self.steps.len() as i32);
        ui.set_setup_way(self.way);
        ui.set_setup_look(step == SetupStep::Look);
        ui.set_setup_pair(step == SetupStep::Pair);
        ui.set_setup_title(words::setup_step(step).into());
        let cards: Vec<SetupCard> = answers.iter().map(|&a| words::setup_answer(a)).map(|(name, line)| SetupCard { name: name.into(), line: line.into() }).collect();
        ui.set_setup_answers(ModelRc::new(VecModel::from(cards)));
        ui.set_setup_chosen(chosen.and_then(|c| answers.iter().position(|&a| a == c)).map_or(-1, |i| i as i32));
        let (step_share, fade) = spacing(chosen);
        ui.set_setup_spacing(step_share);
        ui.set_setup_fade(fade);
        ui.set_setup_beat(chosen == Some(SetupAnswer::AutoMixDj));
        // Plates until the covers are decoded.
        let covers = if step != SetupStep::Listening {
            Vec::new()
        } else if self.covers.is_empty() {
            vec![Image::default(); setup_covers().len()]
        } else {
            self.covers.clone()
        };
        ui.set_setup_covers(ModelRc::new(VecModel::from(covers)));
    }
}

/// How the preview plays `style`'s songs: where each cover starts after the one before, as a share of a cover's
/// side, and how much of it fades in over that one. Without mixing apart with a cut between them (also before
/// any pick), AutoMix overlapping and blended, its DJ drawn close and blended longer.
fn spacing(style: Option<SetupAnswer>) -> (f32, f32) {
    match style {
        Some(SetupAnswer::AutoMix) => (0.72, 0.28),
        Some(SetupAnswer::AutoMixDj) => (0.5, 0.5),
        _ => (1.12, 0.0),
    }
}

/// The preview's covers as RGBA rows, [`COVER_PX`] square; slow, so off the UI thread.
pub fn decode_covers() -> Vec<Vec<u8>> {
    let mut decoder = nori_covers::decode::Decoder::new();
    let side = COVER_PX as usize;
    setup_covers().iter().filter_map(|c| decoder.decode(c, side, side, nori_covers::Alpha::Straight).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn going_back_shows_the_answer_given() {
        let mut s = Setup::new(vec![SetupStep::Listening, SetupStep::Look]);
        assert_eq!(s.answer(2), Some(SetupAnswer::AutoMixDj));
        assert_eq!(s.answer(1), Some(SetupAnswer::AutoMix), "another pick replaces it");
        assert!(s.next());
        assert_eq!((s.step(), s.answer(0)), (SetupStep::Look, None), "the last page offers no answer");
        s.back();
        assert_eq!((s.step(), s.answers[0]), (SetupStep::Listening, Some(SetupAnswer::AutoMix)));
        assert!(s.next());
        assert!(!s.next(), "past the last page the run is over");
    }

    #[test]
    fn the_preview_covers_decode() {
        assert_eq!(decode_covers().iter().map(Vec::len).collect::<Vec<_>>(), vec![(COVER_PX * COVER_PX * 4) as usize; setup_covers().len()]);
    }
}
