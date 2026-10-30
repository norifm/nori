//! UI timings, gradient stops and small display rules shared by every client. [`stage`] is read once;
//! the rules are asked on events, never per frame. Layout and gestures are each platform's own.

use nori_look::sleeve;

use crate::Core;

/// UI constants, read once.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Stage {
    /// Share of the sleeve height that fades at the bottom (`nori_look::cover::FADE_SHARE`). The page
    /// colours are sampled from a shallower band (`cover::SAMPLE_SHARE`), which no client draws.
    pub melt: f32,
    pub rub_out: Vec<sleeve::Stop>,
    /// The same on its side, where the right edge goes soft over this share of the width.
    pub across_melt: f32,
    pub across_rub_out: Vec<sleeve::Stop>,
    pub across_soft: Vec<sleeve::Stop>,
    pub soft: Vec<sleeve::Stop>,
    pub soft_from: f32,
    pub soft_to: f32,
    pub status_shade: f32,
    pub status_shade_to: f32,
    pub hero_stops: Vec<f32>,
    pub floor_stops: Vec<f32>,
    pub lyrics_mask: Vec<sleeve::Stop>,

    /// Buffering time before Play shows a spinner (most skips start within it).
    pub spinner_after_ms: i64,
    /// A move from rest: the player opened by a tap, a panel changed.
    pub move_ms: i32,
    /// A move's curve, a cubic Bézier's two control points (x1, y1, x2, y2): most of the way early, a long
    /// soft landing. A move let go by a finger takes the same curve, shortened to keep the finger's speed.
    pub move_ease: Vec<f32>,
    /// Something appearing where nothing moved: content arriving, a tab's page, a colour changing.
    pub fade_ms: i32,
    /// Something going away: quicker than it came.
    pub fade_out_ms: i32,
    /// Stiffness of the spring that settles what a finger let go (damping ratio 1: no bounce).
    pub settle_stiffness: f32,
    /// A change of page: both pages shift a little and cross-fade.
    pub page_ms: i32,
    /// A change of page's curve, a cubic Bézier's two control points (x1, y1, x2, y2).
    pub page_ease: Vec<f32>,
    /// How far each page shifts in a change of page, dp.
    pub page_shift: f32,
    /// How long the previous cover stays while the new one loads, before the placeholder.
    pub sleeve_hold_ms: i64,
    /// How long the previous page colours stay while the new ones are computed.
    pub colour_wait_ms: i64,
    /// Page colour cross-fade.
    pub colour_fade_ms: i32,
    /// How long scrolled lyrics stay before following playback again.
    pub lyrics_reading_ms: i64,
    /// Word animation: minimum rise, settle, hold time before glowing, and glow fade (`nori_look::lyrics`).
    pub lyrics_rise_min_ms: i64,
    pub lyrics_settle_ms: i64,
    pub lyrics_held_ms: i64,
    pub lyrics_glow_fade_ms: i64,
    /// Brightness of unsung words in the active line.
    pub lyrics_unsung: f32,
    /// Data arriving this soon after a page opens appears without animation.
    pub quick_load_ms: i64,
    /// Limiter meter poll interval on the equalizer screen.
    pub meter_ms: i64,
    /// How far burn-in protection moves the screen either way: what runs to the screen's edges reaches this
    /// far past them, so no edge shows as it moves.
    pub burn_in_reach_dp: f32,
}

#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn stage() -> Stage {
    Stage {
        melt: nori_look::cover::FADE_SHARE,
        rub_out: sleeve::RUB_OUT.to_vec(),
        across_melt: sleeve::ACROSS_MELT,
        across_rub_out: sleeve::ACROSS_RUB_OUT.to_vec(),
        across_soft: sleeve::ACROSS_SOFT.to_vec(),
        soft: sleeve::SOFT.to_vec(),
        soft_from: sleeve::SOFT_FROM,
        soft_to: sleeve::SOFT_TO,
        status_shade: sleeve::STATUS_SHADE,
        status_shade_to: sleeve::STATUS_SHADE_TO,
        hero_stops: sleeve::HERO_STOPS.to_vec(),
        floor_stops: sleeve::FLOOR_STOPS.to_vec(),
        lyrics_mask: sleeve::LYRICS_MASK.to_vec(),
        spinner_after_ms: 300,
        move_ms: 340,
        move_ease: vec![0.25, 1.0, 0.5, 1.0],
        fade_ms: 180,
        fade_out_ms: 120,
        settle_stiffness: 450.0,
        page_ms: 200,
        page_ease: vec![0.4, 0.0, 0.2, 1.0],
        page_shift: 48.0,
        sleeve_hold_ms: 180,
        colour_wait_ms: 180,
        colour_fade_ms: 420,
        lyrics_reading_ms: nori_look::lyrics::READING_MS,
        lyrics_rise_min_ms: nori_look::lyrics::RISE_MIN_MS,
        lyrics_settle_ms: nori_look::lyrics::SETTLE_MS,
        lyrics_held_ms: nori_look::lyrics::HELD_MS,
        lyrics_glow_fade_ms: nori_look::lyrics::GLOW_FADE_MS,
        lyrics_unsung: nori_look::lyrics::UNSUNG,
        quick_load_ms: 300,
        meter_ms: 120,
        burn_in_reach_dp: nori_look::burn_in::REACH_DP,
    }
}

/// What the play button shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum TransportGlyph {
    Play,
    Pause,
    /// Buffering long enough to notice.
    Spinner,
}

/// The play button's glyph: Spinner once buffering outlasted `spinner_after_ms` (`waited`), Pause while
/// playing or buffering, else Play.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn transport_glyph(playing: bool, buffering: bool, waited: bool) -> TransportGlyph {
    if waited {
        TransportGlyph::Spinner
    } else if playing || buffering {
        TransportGlyph::Pause
    } else {
        TransportGlyph::Play
    }
}

/// The times either side of the seek bar, in whole seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SeekTimes {
    pub at_s: i64,
    pub left_s: i64,
}

/// Seek bar times for the drag position while `dragging`, else the pending seek `held_ms` (-1: none),
/// else the playback position.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn seek_times(dragging: bool, drag: f32, held_ms: i64, position_ms: i64, duration_ms: i64) -> SeekTimes {
    let d = duration_ms.max(1) as f32;
    let shown = if dragging {
        (drag * d) as i64
    } else if held_ms >= 0 {
        held_ms
    } else {
        position_ms
    };
    SeekTimes { at_s: shown / 1000, left_s: (duration_ms - shown).max(0) / 1000 }
}

/// The queue panel's rows.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct QueueRows {
    /// List indexes in play order.
    pub order: Vec<u32>,
    /// Dragging is offered only when play order is list order (not shuffled).
    pub reorderable: bool,
    /// List indexes a swipe cannot remove: the current song as shown and as queued (they differ during a mix).
    pub kept: Vec<u32>,
    /// Position in `order` of the shown current song, -1 for none; rows before it have played.
    pub now: i32,
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// Rows for the page's `len`-song queue with `shown` current (-1: none). Uses the queue's play order
    /// when it has the same length, else list order.
    pub fn queue_rows(&self, len: u32, shuffle: bool, shown: i32) -> QueueRows {
        let (order, current) = self.session.playlist(|p| {
            let here = p.len() == len as usize;
            (here.then(|| p.play_order().map(|i| i as u32).collect()), p.current().filter(|_| here))
        });
        rows(order, len, shuffle, shown, current)
    }
}

/// Rows for a mirrored queue of `len` songs, listed in play order already (`remote::Mirror::rows`), with
/// row `shown` playing.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn mirrored_queue_rows(len: u32, shuffle: bool, shown: i32) -> QueueRows {
    rows(None, len, shuffle, shown, None)
}

fn rows(order: Option<Vec<u32>>, len: u32, shuffle: bool, shown: i32, current: Option<usize>) -> QueueRows {
    let order = order.filter(|o| o.len() == len as usize).unwrap_or_else(|| (0..len).collect());
    let mut kept: Vec<u32> = u32::try_from(shown).ok().into_iter().chain(current.map(|c| c as u32)).filter(|&i| i < len).collect();
    kept.dedup();
    let now = u32::try_from(shown).ok().and_then(|s| order.iter().position(|&i| i == s)).map_or(-1, |p| p as i32);
    QueueRows { order, reorderable: !shuffle, kept, now }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_and_seek() {
        assert_eq!(transport_glyph(false, true, false), TransportGlyph::Pause);
        assert_eq!(transport_glyph(false, true, true), TransportGlyph::Spinner);
        assert_eq!(transport_glyph(true, false, false), TransportGlyph::Pause);
        assert_eq!(transport_glyph(false, false, false), TransportGlyph::Play);

        // Seek times prefer drag then held then position.
        assert_eq!(seek_times(true, 0.5, 9_000, 1_000, 200_000), SeekTimes { at_s: 100, left_s: 100 });
        assert_eq!(seek_times(false, 0.5, 9_000, 1_000, 200_000), SeekTimes { at_s: 9, left_s: 191 });
        assert_eq!(seek_times(false, 0.5, -1, 61_500, 200_000), SeekTimes { at_s: 61, left_s: 138 });
        assert_eq!(seek_times(false, 0.0, -1, 5_000, 0), SeekTimes { at_s: 5, left_s: 0 });
    }

    #[test]
    fn queue_rows() {
        assert_eq!(rows(Some(vec![2, 0, 1]), 3, true, 2, Some(2)), QueueRows { order: vec![2, 0, 1], reorderable: false, kept: vec![2], now: 0 });
        assert_eq!(rows(None, 3, false, -1, None), QueueRows { order: vec![0, 1, 2], reorderable: true, kept: vec![], now: -1 });

        // Now is position in play order.
        assert_eq!(rows(None, 5, false, 2, Some(2)).now, 2);
        // Shuffled: list index 1 plays fourth.
        assert_eq!(rows(Some(vec![3, 4, 0, 1, 2]), 5, true, 1, Some(1)).now, 3);
        assert_eq!(rows(None, 3, false, -1, None).now, -1);
        assert_eq!(rows(None, 3, false, 7, None).now, -1);

        // Current rows are kept from swipes.
        // During a mix the page and queue disagree: both kept.
        assert_eq!(rows(None, 4, false, 1, Some(2)).kept, [1, 2]);
        assert_eq!(rows(None, 4, false, 3, None).kept, [3]);
        assert_eq!(rows(None, 2, false, 5, Some(7)).kept, Vec::<u32>::new(), "out of range");
    }

}
