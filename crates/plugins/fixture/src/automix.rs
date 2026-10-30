//! AutoMix as nori's tests check it: a song titled "fixture ..." is analysed (its data the samples it
//! was fed, u64), refined to 123 BPM in key 7 once that data is in, and mixed out of with a 2 s
//! crossfade ending a second before nori's plan starts.

use nori_plugin_sdk::automix::{Analysis, Song, Transition, TransitionKind};
use nori_plugin_sdk::Automix;

use crate::Test;

impl Automix for Test {
    fn begin_analysis(&mut self, song: Song, _rate: u32) -> bool {
        self.analysed = song.title.starts_with("fixture").then_some(0);
        self.analysed.is_some()
    }

    fn feed_analysis(&mut self, samples: Vec<f32>) {
        if let Some(n) = self.analysed.as_mut() {
            *n += samples.len() as u64;
        }
    }

    fn finish_analysis(&mut self) -> Option<Vec<u8>> {
        self.analysed.take().map(|n| n.to_le_bytes().to_vec())
    }

    fn refine(&mut self, song: Song, analysis: Analysis, data: Option<Vec<u8>>) -> Option<Analysis> {
        (song.title.starts_with("fixture") && data.is_some_and(|d| d.len() == 8)).then_some(Analysis { bpm: 123.0, key: 7, ..analysis })
    }

    fn plan(&mut self, outgoing: Song, _incoming: Song, _out: Option<Analysis>, _in_: Option<Analysis>, nori: Transition) -> Option<Transition> {
        outgoing.title.starts_with("fixture").then(|| Transition { kind: TransitionKind::Crossfade, out_start_ms: nori.out_start_ms.saturating_sub(3_000), duration_ms: 2_000, in_start_ms: 0, tempo_ratio: 1.0, bass_swap: false, filter: false })
    }
}
