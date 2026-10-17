//! Burn-in protection for a screen kept on: what is drawn drifts a little once a minute along two
//! triangle waves whose periods share no factor (as Android's always-on display moves its clock), so
//! every position gets the same time and the path repeats only after about a month.

const MINUTE_MS: i64 = 60_000;
/// Periods in minutes; both prime, so the path repeats after 83 × 521 minutes (30 days).
const PERIOD_X: i64 = 83;
const PERIOD_Y: i64 = 521;
/// Minutes the drift takes to grow from nothing to its full reach, so starting it never jumps.
const RAMP: i64 = 4;
/// How far the drift reaches either way. A text or icon stroke is far thinner, so it wears in as a
/// faint smear instead of its shape.
pub const REACH_DP: f32 = 6.0;

/// Where the screen is drawn: offsets within ±`reach_dp`, the reach growing over the first minutes.
/// The client enlarges what it draws by `reach_dp` on each side so no edge comes into view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drift {
    pub x_dp: f32,
    pub y_dp: f32,
    pub reach_dp: f32,
}

/// The drift at `now_ms` for protection that started at `since_ms` (both wall clock).
pub fn drift(now_ms: i64, since_ms: i64) -> Drift {
    let minute = now_ms.div_euclid(MINUTE_MS);
    let ramp = (minute - since_ms.div_euclid(MINUTE_MS)).clamp(0, RAMP) as f32 / RAMP as f32;
    let reach = REACH_DP * ramp;
    Drift { x_dp: reach * wave(minute, PERIOD_X), y_dp: reach * wave(minute, PERIOD_Y), reach_dp: reach }
}

/// Milliseconds until the drift next moves: the next whole minute.
pub fn next_move_ms(now_ms: i64) -> i64 {
    MINUTE_MS - now_ms.rem_euclid(MINUTE_MS)
}

/// A triangle wave from -1 up to 1 and back over `period` minutes.
fn wave(minute: i64, period: i64) -> f32 {
    let t = minute.rem_euclid(period) as f32 / period as f32;
    let up = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
    up * 2.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: i64 = 1_760_000_000_000;

    fn at(minutes: i64) -> Drift {
        drift(START + minutes * MINUTE_MS, START)
    }

    #[test]
    fn starts_still_and_grows_to_full_reach() {
        assert_eq!(at(0), Drift { x_dp: 0.0, y_dp: 0.0, reach_dp: 0.0 });
        let reach: Vec<f32> = (0..=RAMP + 2).map(|m| at(m).reach_dp).collect();
        assert!(reach.windows(2).all(|w| w[1] >= w[0]), "{reach:?}");
        assert_eq!(reach[RAMP as usize], REACH_DP);
        assert_eq!(reach[RAMP as usize + 2], REACH_DP);
    }

    #[test]
    fn stays_within_reach_and_moves_little_each_minute() {
        let path: Vec<Drift> = (0..2 * PERIOD_Y).map(at).collect();
        for d in &path {
            assert!(d.x_dp.abs() <= d.reach_dp && d.y_dp.abs() <= d.reach_dp, "{d:?}");
        }
        // Past the ramp a step is a fraction of a dp: never a visible jump.
        for w in path[RAMP as usize..].windows(2) {
            assert!((w[1].x_dp - w[0].x_dp).abs() < 0.5 && (w[1].y_dp - w[0].y_dp).abs() < 0.5, "{w:?}");
        }
    }

    #[test]
    fn spends_equal_time_across_the_reach() {
        // One full horizontal period past the ramp, counted in four equal bands of the reach.
        let mut bands = [0; 4];
        for m in RAMP..RAMP + PERIOD_X {
            let x = at(m).x_dp / REACH_DP;
            bands[(((x + 1.0) / 2.0 * 4.0) as usize).min(3)] += 1;
        }
        assert!(bands.iter().all(|&n| (19..=23).contains(&n)), "{bands:?}");
        let xs: Vec<f32> = (RAMP..RAMP + PERIOD_X).map(|m| at(m).x_dp).collect();
        assert!(xs.iter().cloned().fold(f32::MAX, f32::min) < -0.95 * REACH_DP);
        assert!(xs.iter().cloned().fold(f32::MIN, f32::max) > 0.95 * REACH_DP);
    }

    #[test]
    fn moves_on_the_next_whole_minute() {
        assert_eq!([next_move_ms(0), next_move_ms(1), next_move_ms(59_999), next_move_ms(60_000)], [60_000, 59_999, 1, 60_000]);
        let now = START + 12_345;
        assert_ne!(drift(now + next_move_ms(now) + 10 * MINUTE_MS, START), drift(now + 10 * MINUTE_MS, START));
    }
}
