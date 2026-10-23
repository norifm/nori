//! The seek bar on an e-ink screen ([`EinkSeek`]): the song in whole blocks and the time played in whole
//! minutes. Each redraw costs a panel refresh, so the bar changes a few times a song, and what it shows
//! is right at the precision it shows rather than a fine bar left behind.

/// A song is cut into blocks at least this long, so the bar changes at most twice a minute...
const BLOCK_MS: i64 = 30_000;
/// ...but never into fewer than this many, nor more than fit a phone's width.
const MIN_BLOCKS: i64 = 4;
const MAX_BLOCKS: i64 = 24;
const MINUTE_MS: i64 = 60_000;
/// The wake lands this far past a change, so the position read then is past it rather than just short.
const LATE_MS: i64 = 100;

/// What an e-ink seek bar shows with the song at one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EinkSeek {
    /// How many blocks the bar is cut into; 0 for a song whose length is unknown.
    pub blocks: u32,
    /// Blocks wholly played.
    pub filled: u32,
    /// Whole minutes played.
    pub minutes: u32,
    /// Song time until the blocks or minutes next change, plus [`LATE_MS`].
    pub next_ms: i64,
}

impl EinkSeek {
    /// The bar with the song at `position_ms` of `duration_ms` (0: length unknown).
    pub fn at(position_ms: i64, duration_ms: i64) -> Self {
        let at = if duration_ms > 0 { position_ms.clamp(0, duration_ms) } else { position_ms.max(0) };
        let minutes = at / MINUTE_MS;
        let next_minute = (minutes + 1) * MINUTE_MS;
        let minutes = minutes.min(u32::MAX as i64) as u32;
        if duration_ms <= 0 {
            return EinkSeek { blocks: 0, filled: 0, minutes, next_ms: next_minute - at + LATE_MS };
        }
        let blocks = (duration_ms / BLOCK_MS).clamp(MIN_BLOCKS, MAX_BLOCKS);
        let filled = at * blocks / duration_ms;
        // The first millisecond at which one more block is wholly played.
        let next_block = ((filled + 1) * duration_ms + blocks - 1) / blocks;
        let next = if filled < blocks { next_block.min(next_minute) } else { next_minute };
        EinkSeek { blocks: blocks as u32, filled: filled as u32, minutes, next_ms: next - at + LATE_MS }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays a song of `duration_ms` from the start, waking only when told; returns the wakes and checks
    /// that each one changed what is shown and that what is shown never runs ahead of the song.
    fn wakes(duration_ms: i64) -> usize {
        let mut at = 0;
        let mut shown = EinkSeek::at(at, duration_ms);
        let mut wakes = 0;
        while at + shown.next_ms < duration_ms.max(1) {
            at += shown.next_ms;
            let now = EinkSeek::at(at, duration_ms);
            assert_ne!((now.filled, now.minutes), (shown.filled, shown.minutes), "a wake at {at} that changed nothing");
            assert_eq!(now.minutes as i64, at / MINUTE_MS);
            assert!(now.filled as i64 * duration_ms <= at * now.blocks as i64, "ahead of the song at {at}");
            shown = now;
            wakes += 1;
        }
        wakes
    }

    #[test]
    fn redraws_a_few_times_a_song() {
        // Four minutes: 8 blocks of 30 s, the minutes changing with every other block.
        assert_eq!(wakes(240_000), 7);
        // Eighty seconds: the fewest blocks, 20 s each, the minute falling on one of them.
        assert_eq!(EinkSeek::at(0, 80_000).blocks, 4);
        assert_eq!(wakes(80_000), 3);
        // An hour: the most blocks, and then the minutes set the pace.
        assert_eq!(EinkSeek::at(0, 3_600_000).blocks, 24);
        assert_eq!(wakes(3_600_000), 59 + 12);
    }

    #[test]
    fn shows_where_the_song_is() {
        let s = EinkSeek::at(135_000, 240_000);
        assert_eq!((s.blocks, s.filled, s.minutes, s.next_ms), (8, 4, 2, 15_000 + LATE_MS));
        let end = EinkSeek::at(250_000, 240_000);
        assert_eq!((end.filled, end.minutes), (8, 4));
    }

    #[test]
    fn unknown_length_counts_minutes() {
        let s = EinkSeek::at(90_000, 0);
        assert_eq!((s.blocks, s.filled, s.minutes, s.next_ms), (0, 0, 1, 30_000 + LATE_MS));
    }
}
