//! The processors between the transition engine and the track, and the input they were given while
//! the track may still be asked to play what they made of it. With the processors' state kept every
//! [`MARK_FRAMES`], their output can be made again from any frame the track holds: a sound change
//! starts where the track can still change, continuing the old sound's state there.

use std::collections::VecDeque;

use crate::dsp::Equalizer;
use crate::pcm::Encoding;
use crate::silence::SilenceSkipper;
use crate::speed::SpeedPitch;
use crate::stage::{Saved, SongFacts, Stages};
use std::sync::Arc;

/// Input frames between kept states: the most that is run again to reach a splice.
pub(crate) const MARK_FRAMES: u64 = 8192;

/// The chain in media3's order: equalizer, silence skipping, speed.
#[derive(Default)]
pub struct Processors {
    pub eq: Option<Equalizer>,
    pub silence: Option<SilenceSkipper>,
    pub speed: Option<SpeedPitch>,
}

/// `clone_from` keeps every buffer's memory.
impl Clone for Processors {
    fn clone(&self) -> Self {
        Processors { eq: self.eq.clone(), silence: self.silence.clone(), speed: self.speed.clone() }
    }

    fn clone_from(&mut self, o: &Self) {
        self.eq.clone_from(&o.eq);
        self.silence.clone_from(&o.silence);
        self.speed.clone_from(&o.speed);
    }
}

/// Input as offered: its first frame, song frames per frame, timeline position (µs) of that frame,
/// the gain the chain applies to it (1 when the samples are at their level), and what the stages read
/// of its song.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub frame: u64,
    pub pace: f64,
    pub pts: i64,
    pub gain: f32,
    pub song: Arc<SongFacts>,
}

impl Piece {
    /// Timeline position (µs) and pace of input frame `frame` of this piece.
    pub fn at(&self, frame: u64, rate: u32) -> (i64, f64) {
        (self.pts + ((frame - self.frame) as f64 * self.pace * 1e6 / rate as f64) as i64, self.pace)
    }
}

/// The processors' and stages' state before input frame `frame`, when they had made `out` frames of
/// output from `media` song frames of input.
pub struct Mark {
    pub frame: u64,
    pub out: u64,
    pub media: f64,
    pub chain: Processors,
    pub(crate) stages: Saved,
}

/// Input kept since the flush, from the oldest mark the track may still need. A ring in which each
/// piece lies in one run: a piece that would cross its end starts at its beginning instead.
#[derive(Default)]
pub struct Kept {
    bytes: Vec<u8>,
    /// Where frame `first` is.
    head: usize,
    first: u64,
    /// The frame after the last kept.
    end: u64,
    frame_bytes: usize,
    /// Each piece and where its first frame is.
    pieces: VecDeque<(Piece, usize)>,
    /// The first frame kept from the ring's beginning while older ones lie after it.
    wrap: Option<u64>,
    marks: VecDeque<Mark>,
    /// Marks let go, reused so keeping one does not allocate.
    spare: Vec<(Processors, Saved)>,
}

impl Kept {
    /// Forgets everything; frames count from `first`, `frame_bytes` each. Makes room for `frames` of
    /// input and their marks at once, rather than growing on the buffer path.
    pub fn restart(&mut self, first: u64, frame_bytes: usize, frames: u64) {
        self.head = 0;
        self.first = first;
        self.end = first;
        self.frame_bytes = frame_bytes.max(1);
        self.pieces.clear();
        self.wrap = None;
        while let Some(m) = self.marks.pop_front() {
            self.spare.push((m.chain, m.stages));
        }
        let room = frames as usize * self.frame_bytes;
        if self.bytes.len() < room {
            self.bytes.resize(room, 0);
        }
        let marks = (frames / MARK_FRAMES) as usize + 2;
        self.marks.reserve(marks);
        self.spare.reserve(marks);
    }

    /// The frame after the last kept.
    pub fn end(&self) -> u64 {
        self.end
    }

    /// Keeps `input` at the end.
    pub fn keep(&mut self, input: &[u8], pace: f64, pts: i64, gain: f32, song: &Arc<SongFacts>) {
        let frame = self.end;
        let at = self.room(input.len());
        self.bytes[at..at + input.len()].copy_from_slice(input);
        self.pieces.push_back((Piece { frame, pace, pts, gain, song: song.clone() }, at));
        self.end += (input.len() / self.frame_bytes) as u64;
    }

    /// Where `n` more bytes go: after the last piece, or at the ring's beginning when they fit there
    /// before the oldest frame kept. A ring too small grows.
    fn room(&mut self, n: usize) -> usize {
        let Some((last, at)) = self.pieces.back() else {
            self.head = 0;
            self.wrap = None;
            if self.bytes.len() < n {
                self.bytes.resize(n, 0);
            }
            return 0;
        };
        let tail = at + (self.end - last.frame) as usize * self.frame_bytes;
        match self.wrap {
            None if tail + n <= self.bytes.len() => tail,
            None if n <= self.head => {
                self.wrap = Some(self.end);
                0
            }
            Some(_) if tail + n <= self.head => tail,
            _ => self.grow(n),
        }
    }

    /// Moves what is kept into a larger ring, in order from its beginning, with room for `n` more
    /// bytes; returns where they go.
    fn grow(&mut self, n: usize) -> usize {
        let fb = self.frame_bytes;
        let (first, first_at) = self.pieces.front().map(|(p, at)| (p.frame, *at)).expect("pieces kept");
        let lead = (self.first - first) as usize * fb;
        let held = lead + (self.end - self.first) as usize * fb;
        let mut bytes = vec![0; (self.bytes.len() * 2).max(held + n)];
        let mut to = 0;
        for k in 0..self.pieces.len() {
            let (p, at) = &self.pieces[k];
            let end = self.pieces.get(k + 1).map_or(self.end, |(n, _)| n.frame);
            let skip = if k == 0 { lead } else { 0 };
            let len = (end - p.frame) as usize * fb;
            bytes[to + skip..to + len].copy_from_slice(&self.bytes[at + skip..at + len]);
            self.pieces[k].1 = to;
            to += len;
        }
        debug_assert_eq!(first_at + lead, self.head);
        self.bytes = bytes;
        self.head = lead;
        self.wrap = None;
        to
    }

    /// Keeps `chain`'s and `stages`' state before `frame` if the last mark is [`MARK_FRAMES`] back (or
    /// there is none); returns whether it did.
    pub(crate) fn mark(&mut self, frame: u64, out: u64, media: f64, chain: &Processors, stages: &mut Stages) -> bool {
        if self.marks.back().is_some_and(|m| frame < m.frame + MARK_FRAMES) {
            return false;
        }
        self.push_mark(frame, out, media, chain, stages);
        true
    }

    /// Keeps `chain`'s and `stages`' state before `frame` as it changed there, in place of any mark from
    /// `frame` on.
    pub(crate) fn mark_changed(&mut self, frame: u64, out: u64, media: f64, chain: &Processors, stages: &mut Stages) {
        while self.marks.back().is_some_and(|m| m.frame >= frame) {
            let m = self.marks.pop_back().expect("checked");
            self.spare.push((m.chain, m.stages));
        }
        self.push_mark(frame, out, media, chain, stages);
    }

    fn push_mark(&mut self, frame: u64, out: u64, media: f64, chain: &Processors, stages: &mut Stages) {
        let (mut kept, mut saved) = self.spare.pop().unwrap_or_default();
        kept.clone_from(chain);
        stages.save(&mut saved);
        self.marks.push_back(Mark { frame, out, media, chain: kept, stages: saved });
    }

    /// Lets go of what comes before the last mark at or before output frame `out`.
    pub fn trim(&mut self, out: u64) {
        while self.marks.len() >= 2 && self.marks[1].out <= out {
            let m = self.marks.pop_front().expect("two marks");
            self.spare.push((m.chain, m.stages));
        }
        let Some(from) = self.marks.front().map(|m| m.frame) else { return };
        self.first = self.first.max(from);
        while self.pieces.len() >= 2 && self.pieces[1].0.frame <= from {
            self.pieces.pop_front();
        }
        if let Some((p, at)) = self.pieces.front() {
            self.head = at + (self.first - p.frame) as usize * self.frame_bytes;
            if self.wrap.is_some_and(|w| p.frame >= w) {
                self.wrap = None;
            }
        }
    }

    pub fn has_marks(&self) -> bool {
        !self.marks.is_empty()
    }

    /// The last mark that `f` holds for.
    pub fn mark_where(&self, f: impl Fn(&Mark) -> bool) -> Option<usize> {
        self.marks.iter().rposition(f)
    }

    pub fn mark_at(&self, k: usize) -> &Mark {
        &self.marks[k]
    }

    /// Lets go of the marks after input frame `frame`.
    pub fn forget_after(&mut self, frame: u64) {
        while self.marks.back().is_some_and(|m| m.frame > frame) {
            let m = self.marks.pop_back().expect("checked");
            self.spare.push((m.chain, m.stages));
        }
    }

    /// Lets go of the input from frame `frame` on.
    pub fn forget_from(&mut self, frame: u64) {
        self.forget_after(frame);
        let frame = frame.clamp(self.first, self.end);
        self.end = frame;
        while self.pieces.back().is_some_and(|(p, _)| p.frame >= frame) {
            self.pieces.pop_back();
        }
        if self.wrap.is_some_and(|w| w >= frame) {
            self.wrap = None;
        }
    }

    /// Song frames of the input from frame `frame` on.
    pub fn media_from(&self, frame: u64) -> f64 {
        let mut media = 0.0;
        for (k, (p, _)) in self.pieces.iter().enumerate() {
            let end = self.pieces.get(k + 1).map_or(self.end, |(n, _)| n.frame);
            if end > frame {
                media += (end - p.frame.max(frame)) as f64 * p.pace;
            }
        }
        media
    }

    /// The input frame at timeline position `pts` (as a song's frames are stamped: whole µs, rounded
    /// down) within an unstretched piece, or the end of the input.
    pub fn frame_at(&self, pts: i64, rate: u32) -> Option<u64> {
        let k = self.pieces.partition_point(|(p, _)| p.pts <= pts).checked_sub(1)?;
        let p = &self.pieces[k].0;
        let end = self.pieces.get(k + 1).map_or(self.end, |(n, _)| n.frame);
        let d = ((pts - p.pts) as f64 * rate as f64 / 1_000_000.0).round() as u64;
        (p.pace == 1.0 && p.frame + d <= end).then_some(p.frame + d)
    }

    /// The piece holding input frame `frame`, and the frame after it.
    pub fn piece(&self, frame: u64) -> Option<(Piece, u64)> {
        let k = self.pieces.partition_point(|(p, _)| p.frame <= frame).checked_sub(1)?;
        let end = self.pieces.get(k + 1).map_or(self.end, |(p, _)| p.frame);
        (frame < end).then(|| (self.pieces[k].0.clone(), end))
    }

    /// Input frames `from..to`, of one piece.
    pub fn frames(&self, from: u64, to: u64) -> &[u8] {
        let (p, at) = &self.pieces[self.pieces.partition_point(|(p, _)| p.frame <= from) - 1];
        let at = at + (from - p.frame) as usize * self.frame_bytes;
        &self.bytes[at..at + (to - from) as usize * self.frame_bytes]
    }

    /// Scales the input from frame `from` whose timeline position is in `pts` by `ratio`: its gain, or
    /// the samples where they carry it.
    pub fn rescale(&mut self, from: u64, pts: std::ops::Range<i64>, ratio: f32, encoding: Encoding, rate: u32) {
        let mut k = 0;
        while k < self.pieces.len() {
            let (p, at) = self.pieces[k].clone();
            let end = self.pieces.get(k + 1).map_or(self.end, |(n, _)| n.frame);
            k += 1;
            if end <= from || !pts.contains(&p.pts) {
                continue;
            }
            let byte = |f: u64| at + (f - p.frame) as usize * self.frame_bytes;
            if p.gain == 1.0 {
                let range = byte(p.frame.max(from).max(self.first))..byte(end);
                crate::pcm::scale(&mut self.bytes[range], encoding, ratio);
            } else if p.frame < from {
                // What comes before `from` is run again at the gain it was run at.
                let at = byte(from);
                self.pieces.insert(k, (Piece { frame: from, pts: p.at(from, rate).0, gain: p.gain * ratio, ..p }, at));
                k += 1;
            } else {
                self.pieces[k - 1].0.gain *= ratio;
            }
        }
    }

    /// The input from frame `from` of the song `was` tells of is now told of by `now` (its prepared
    /// data came); what comes before `from` is run again as it was.
    pub fn retell(&mut self, from: u64, was: &Arc<SongFacts>, now: &Arc<SongFacts>, rate: u32) {
        let mut k = 0;
        while k < self.pieces.len() {
            let end = self.pieces.get(k + 1).map_or(self.end, |(n, _)| n.frame);
            let (p, at) = &self.pieces[k];
            k += 1;
            if end <= from || !Arc::ptr_eq(&p.song, was) {
                continue;
            }
            if p.frame < from {
                let at = at + (from - p.frame) as usize * self.frame_bytes;
                let rest = Piece { frame: from, pts: p.at(from, rate).0, song: now.clone(), ..p.clone() };
                self.pieces.insert(k, (rest, at));
                k += 1;
            } else {
                self.pieces[k - 1].0.song = now.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(n: u64, from: u8) -> Vec<u8> {
        (0..n * 4).map(|i| from.wrapping_add(i as u8)).collect()
    }

    #[test]
    fn trimmed_to_mark_before_played() {
        let mut k = Kept::default();
        k.restart(0, 4, 0);
        let (chain, mut stages) = (Processors::default(), Stages::default());
        for i in 0..4u64 {
            k.mark(i * MARK_FRAMES, i * MARK_FRAMES, 0.0, &chain, &mut stages);
            k.keep(&frames(MARK_FRAMES, i as u8), 1.0, i as i64 * 1000, 1.0, &crate::stage::no_facts());
        }
        k.trim(2 * MARK_FRAMES + 5);
        assert_eq!(k.mark_at(0).frame, 2 * MARK_FRAMES, "nothing kept before the mark still needed");
        assert_eq!(k.frames(2 * MARK_FRAMES, 2 * MARK_FRAMES + 1), &frames(1, 2)[..], "the input kept is the input given");
        assert_eq!(k.end(), 4 * MARK_FRAMES);
        assert_eq!(k.piece(3 * MARK_FRAMES + 1), Some((Piece { frame: 3 * MARK_FRAMES, pace: 1.0, pts: 3000, gain: 1.0, song: crate::stage::no_facts() }, 4 * MARK_FRAMES)));
    }

    /// Frames `from..from + n`, each its own number.
    fn numbered(from: u64, n: u64) -> Vec<u8> {
        (from..from + n).flat_map(|f| (f as u32).to_le_bytes()).collect()
    }

    /// Every frame kept is the one given, piece by piece.
    fn assert_whole(k: &Kept) {
        let mut f = k.first;
        while let Some((_, end)) = k.piece(f) {
            assert_eq!(k.frames(f, end), &numbered(f, end - f)[..], "frames {f}..{end}");
            f = end;
        }
        assert_eq!(f, k.end());
    }

    #[test]
    fn playing_keeps_its_input_in_the_room_made_for_it() {
        let mut k = Kept::default();
        let room = 12 * MARK_FRAMES;
        k.restart(0, 4, room);
        let (chain, mut stages) = (Processors::default(), Stages::default());
        for (i, n) in [4096, 1152, 4608, 576, 8192].into_iter().cycle().take(400).enumerate() {
            let at = k.end();
            k.mark(at, at, 0.0, &chain, &mut stages);
            k.keep(&numbered(at, n), 1.0, i as i64, 1.0, &crate::stage::no_facts());
            // The track holds eight marks' worth of output.
            k.trim(k.end().saturating_sub(8 * MARK_FRAMES));
            assert_whole(&k);
            assert!(k.bytes.capacity() <= room as usize * 4, "{} bytes held for {room} frames", k.bytes.capacity());
        }
    }

    #[test]
    fn input_forgotten_and_scaled_across_the_wrap() {
        let mut k = Kept::default();
        k.restart(0, 4, 4 * MARK_FRAMES);
        let (chain, mut stages) = (Processors::default(), Stages::default());
        let mut keep = |k: &mut Kept, n: u64| {
            let at = k.end();
            k.mark(at, at, 0.0, &chain, &mut stages);
            k.keep(&numbered(at, n), 1.0, at as i64, 1.0, &crate::stage::no_facts());
        };
        for _ in 0..3 {
            keep(&mut k, MARK_FRAMES);
        }
        k.trim(2 * MARK_FRAMES);
        // A piece that fits only from the start of the ring, then the input from inside it forgotten.
        keep(&mut k, MARK_FRAMES + 100);
        assert_eq!(k.pieces.back().map(|(_, at)| *at), Some(0), "the piece starts the ring again");
        keep(&mut k, 50);
        assert_whole(&k);
        k.forget_from(3 * MARK_FRAMES + 40);
        assert_eq!(k.end(), 3 * MARK_FRAMES + 40);
        assert_whole(&k);
        keep(&mut k, 300);
        assert_whole(&k);
        // More than the ring holds: it grows, keeping what it had.
        keep(&mut k, 3 * MARK_FRAMES);
        assert!(k.bytes.len() > 16 * MARK_FRAMES as usize, "the ring grew");
        assert_whole(&k);
        // Scaling a piece's samples touches its frames from `from` and nothing else.
        let before: Vec<u8> = k.frames(3 * MARK_FRAMES, 3 * MARK_FRAMES + 40).to_vec();
        k.rescale(3 * MARK_FRAMES + 20, 3 * MARK_FRAMES as i64..3 * MARK_FRAMES as i64 + 1, 0.5, Encoding::Pcm16, 48_000);
        let mut want = before.clone();
        crate::pcm::scale(&mut want[80..], Encoding::Pcm16, 0.5);
        assert_eq!(k.frames(3 * MARK_FRAMES, 3 * MARK_FRAMES + 40), &want[..]);
        assert_eq!(k.frames(3 * MARK_FRAMES + 340, 3 * MARK_FRAMES + 341), &numbered(3 * MARK_FRAMES + 340, 1)[..]);
    }
}
