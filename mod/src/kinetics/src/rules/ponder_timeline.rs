//! Ponder's timeline: which keyframe marker the pointer is on (`PonderUI`'s timeline and its keyframes). No game
//! types.
//!
//! The bar is `width` wide with a 1 pixel border, so the fill and the markers are drawn over `width - 2` pixels
//! (`x + (width - 2) * tick / duration`); the pointer's position on it has to be read over the same `width - 2`.
//! Reading it over the whole `width` put a pointer exactly on a marker a little before it, and the click took the
//! keyframe before: the bug this module fixes.

/// The pixels a tick span covers on the bar: its inside, without the 1 pixel border on each side.
fn inner(width: f32) -> f32 {
    (width - 2.0).max(1.0)
}

/// Where a marker at `mark` ticks is drawn (its left edge), on a bar at `x` that is `width` wide for `duration` ticks.
pub fn mark_x(x: f32, width: f32, mark: f32, duration: f32) -> f32 {
    x + inner(width) * mark / duration
}

/// The tick under the pointer at `mouse_x`.
pub fn tick_at(x: f32, width: f32, mouse_x: f32, duration: f32) -> f32 {
    (mouse_x - x) * duration / inner(width)
}

/// Which marker a pointer at `mouse_x` is on: the last one at or before the pointer, or the end of the scene when the
/// pointer is nearer it than to the last marker. A marker counts from one pixel before its own, so that the pixels it
/// is drawn on (it is 2 wide) and the one beside it select it. `None` with fewer than two markers.
pub fn hovered_mark(x: f32, width: f32, mouse_x: f32, duration: f32, markers: &[f32]) -> Option<f32> {
    if markers.len() < 2 {
        return None;
    }
    let tick = tick_at(x, width, mouse_x, duration);
    let last = *markers.last()?;
    if duration - tick < (tick - last) / 2.0 {
        return Some(duration);
    }
    let slack = duration / inner(width);
    Some(markers[markers.partition_point(|&at| at <= tick + slack).saturating_sub(1)])
}

#[cfg(test)]
mod tests {
    use super::*;

    // A bar at x = 10, 202 wide: 200 pixels inside for 200 ticks, so a pixel is a tick.
    const X: f32 = 10.0;
    const WIDTH: f32 = 202.0;
    const DURATION: f32 = 200.0;
    const MARKERS: [f32; 3] = [0.0, 40.0, 100.0];

    #[test]
    fn a_marker_is_drawn_where_the_pointer_reads_it() {
        for mark in MARKERS {
            let at = mark_x(X, WIDTH, mark, DURATION);
            assert!((tick_at(X, WIDTH, at, DURATION) - mark).abs() < 1e-4, "{mark}");
        }
    }

    #[test]
    fn a_click_exactly_on_a_marker_picks_it_and_not_the_one_before() {
        for mark in MARKERS {
            let at = mark_x(X, WIDTH, mark, DURATION);
            assert_eq!(hovered_mark(X, WIDTH, at, DURATION, &MARKERS), Some(mark));
            // The second pixel of its 2 wide mark, too.
            assert_eq!(hovered_mark(X, WIDTH, at + 1.0, DURATION, &MARKERS), Some(mark));
        }
    }

    #[test]
    fn between_markers_the_earlier_one_is_picked() {
        let at = |tick: f32| X + tick;
        assert_eq!(hovered_mark(X, WIDTH, at(20.0), DURATION, &MARKERS), Some(0.0));
        assert_eq!(hovered_mark(X, WIDTH, at(70.0), DURATION, &MARKERS), Some(40.0));
        // One pixel before a marker still takes it, two do not.
        assert_eq!(hovered_mark(X, WIDTH, at(39.0), DURATION, &MARKERS), Some(40.0));
        assert_eq!(hovered_mark(X, WIDTH, at(38.0), DURATION, &MARKERS), Some(0.0));
    }

    #[test]
    fn near_the_end_the_end_is_picked() {
        let at = |tick: f32| X + tick;
        assert_eq!(hovered_mark(X, WIDTH, at(190.0), DURATION, &MARKERS), Some(DURATION));
        assert_eq!(hovered_mark(X, WIDTH, at(110.0), DURATION, &MARKERS), Some(100.0));
    }

    #[test]
    fn a_scene_with_fewer_than_two_markers_has_none_to_pick() {
        assert_eq!(hovered_mark(X, WIDTH, 50.0, DURATION, &[0.0]), None);
        assert_eq!(hovered_mark(X, WIDTH, 50.0, DURATION, &[]), None);
    }
}
