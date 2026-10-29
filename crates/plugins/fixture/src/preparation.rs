//! Song preparation as nori's tests check it: a song titled "fixture ..." is wanted, and its data is
//! the frames it was fed (u64) and the frame its first samples were at (u64).

use nori_plugin_sdk::preparation::Song;
use nori_plugin_sdk::Preparation;

use crate::Test;

impl Preparation for Test {
    fn begin(&mut self, song: Song, _rate: u32, channels: u8) -> bool {
        self.prepared = song.title.starts_with("fixture").then_some((0, None, channels.max(1) as u64));
        self.prepared.is_some()
    }

    fn feed(&mut self, samples: Vec<f32>, at_frame: u64) {
        if let Some((frames, first, channels)) = self.prepared.as_mut() {
            *frames += samples.len() as u64 / *channels;
            first.get_or_insert(at_frame);
        }
    }

    fn finish(&mut self) -> Option<Vec<u8>> {
        let (frames, first, _) = self.prepared.take()?;
        Some([frames.to_le_bytes(), first.unwrap_or(0).to_le_bytes()].concat())
    }
}
