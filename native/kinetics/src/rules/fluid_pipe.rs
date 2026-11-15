//! Which sides a Fluid Pipe opens: its six `down` .. `east` states and how they follow its
//! neighbours. No game types.
//!
//! Ported from Create `content/fluids/pipes/FluidPipeBlock` (`updateBlockState`, `isOpenAt`,
//! `shouldDrawCasing`), `FluidPropagator.getStraightPipeAxis` and `GlassFluidPipeBlock` /
//! `AxisPipeBlock`.
//!
//! Known differences from Create: a pipe has no bracket and no casing, no waterlogging, and
//! it is never rotated or mirrored by a structure; the glue refreshes the sides from its
//! neighbours every few ticks instead of on each neighbour's update.

use crate::{Axis, Dir};

/// The six sides, in [`Dir::ALL`] order (`Iterate.directions`: down, up, north, south, west, east).
pub type Sides = [bool; 6];

pub fn open(sides: &Sides, face: Dir) -> bool {
    sides[face as usize]
}

fn count(sides: &Sides) -> usize {
    sides.iter().filter(|open| **open).count()
}

/// `FluidPipeBlock.updateBlockState`: every side but `ignore` connects where `can_connect` says
/// so; a pipe with one connection gets the opposite end too, one with none keeps its straight
/// run (or takes `preferred` and its opposite).
pub fn update_sides(
    previous: &Sides,
    preferred: Dir,
    ignore: Option<Dir>,
    can_connect: impl Fn(Dir) -> bool,
) -> Sides {
    let mut state = *previous;
    for d in Dir::ALL {
        if Some(d) != ignore {
            state[d as usize] = can_connect(d);
        }
    }
    // See if it has enough connections.
    let mut connected: Option<Dir> = None;
    for d in Dir::ALL {
        if open(&state, d) {
            if connected.is_some() {
                return state;
            }
            connected = Some(d);
        }
    }
    // Add the opposite end if there is only one connection.
    if let Some(d) = connected {
        state[d.opposite() as usize] = true;
        return state;
    }
    // If it can't connect to anything and was a straight run before, do nothing.
    if count(previous) == 2 {
        return *previous;
    }
    state[preferred as usize] = true;
    state[preferred.opposite() as usize] = true;
    state
}

/// `FluidPropagator.getStraightPipeAxis`: the axis of a pipe with exactly two open sides that
/// face each other.
pub fn straight_axis(sides: &Sides) -> Option<Axis> {
    let mut found = None;
    let mut connections = 0;
    for axis in Axis::ALL {
        let negative = Dir::ALL.into_iter().find(|d| d.axis() == axis && d.sign() < 0)?;
        let positive = negative.opposite();
        let (a, b) = (open(sides, negative), open(sides, positive));
        connections += usize::from(a) + usize::from(b);
        if a && b {
            if found.is_some() {
                return None;
            }
            found = Some(axis);
        }
    }
    if connections == 2 { found } else { None }
}

/// `FluidPipeBlock.getAxisState`: a straight pipe along `axis`.
pub fn axis_sides(axis: Axis) -> Sides {
    Dir::ALL.map(|d| d.axis() == axis)
}

/// `FluidPipeBlock.onWrenched`: the axis a wrench makes a glass pipe along, for a pipe clicked
/// on face `clicked`: a straight pipe's own, else (Create picks the open side nearest the click
/// point, which a hook doesn't get) the first open side that isn't along the face clicked.
/// `None` when the click is along the axis (the wrench does nothing).
pub fn glass_axis(sides: &Sides, clicked: Dir) -> Option<Axis> {
    let axis = straight_axis(sides).or_else(|| {
        Dir::ALL
            .into_iter()
            .find(|d| open(sides, *d) && d.axis() != clicked.axis())
            .map(Dir::axis)
    })?;
    if clicked.axis() == axis { None } else { Some(axis) }
}

/// `FluidPipeBlock`'s shape: the 8 pixel core (`PipeBlock(4 / 16f)`) and an arm to each open
/// side, as `[x0, y0, z0, x1, y1, z1]` in blocks.
pub fn shape(sides: &Sides) -> Vec<[f32; 6]> {
    let (lo, hi) = (4.0 / 16.0, 12.0 / 16.0);
    let mut boxes = vec![[lo, lo, lo, hi, hi, hi]];
    for d in Dir::ALL {
        if !open(sides, d) {
            continue;
        }
        let mut b = [lo, lo, lo, hi, hi, hi];
        let axis = d.axis().index();
        if d.sign() > 0 {
            b[axis + 3] = 1.0;
        } else {
            b[axis] = 0.0;
        }
        boxes.push(b);
    }
    boxes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sides(open: &[Dir]) -> Sides {
        Dir::ALL.map(|d| open.contains(&d))
    }

    #[test]
    fn a_pipe_with_nothing_beside_it_runs_along_the_preferred_direction() {
        let state = update_sides(&[false; 6], Dir::North, None, |_| false);
        assert_eq!(state, sides(&[Dir::North, Dir::South]));
    }

    #[test]
    fn one_connection_gets_the_opposite_end() {
        let state = update_sides(&[false; 6], Dir::Up, None, |d| d == Dir::East);
        assert_eq!(state, sides(&[Dir::East, Dir::West]));
    }

    #[test]
    fn two_connections_make_a_corner() {
        let state = update_sides(&[false; 6], Dir::Up, None, |d| d == Dir::East || d == Dir::Down);
        assert_eq!(state, sides(&[Dir::East, Dir::Down]));
    }

    #[test]
    fn losing_every_neighbour_leaves_a_straight_pipe_as_it_was() {
        let before = sides(&[Dir::East, Dir::West]);
        assert_eq!(update_sides(&before, Dir::Up, None, |_| false), before);
        // Any two sides count (`prevStateSides == 2`): a corner stays too.
        let corner = sides(&[Dir::East, Dir::Down]);
        assert_eq!(update_sides(&corner, Dir::Up, None, |_| false), corner);
        // A tee that lost every neighbour becomes a run along the preferred direction.
        let tee = sides(&[Dir::East, Dir::West, Dir::Down]);
        assert_eq!(update_sides(&tee, Dir::Up, None, |_| false), sides(&[Dir::Up, Dir::Down]));
    }

    #[test]
    fn the_ignored_side_keeps_its_state() {
        let before = sides(&[Dir::East, Dir::West]);
        let state = update_sides(&before, Dir::Up, Some(Dir::West), |d| d == Dir::East);
        assert_eq!(state, before);
    }

    #[test]
    fn straight_pipes_are_found_by_their_open_sides() {
        assert_eq!(straight_axis(&sides(&[Dir::East, Dir::West])), Some(Axis::X));
        assert_eq!(straight_axis(&sides(&[Dir::East, Dir::Down])), None);
        assert_eq!(straight_axis(&sides(&[Dir::East, Dir::West, Dir::Up])), None);
        assert_eq!(straight_axis(&axis_sides(Axis::Y)), Some(Axis::Y));
        assert_eq!(straight_axis(&[false; 6]), None);
    }

    #[test]
    fn a_wrench_on_a_corner_picks_an_open_side_across_the_click() {
        let corner = sides(&[Dir::East, Dir::Down]);
        assert_eq!(glass_axis(&corner, Dir::North), Some(Axis::Y));
        assert_eq!(glass_axis(&corner, Dir::Up), Some(Axis::X));
        // A straight pipe along x can't be turned from its end, but can from the side.
        let straight = sides(&[Dir::East, Dir::West]);
        assert_eq!(glass_axis(&straight, Dir::East), None);
        assert_eq!(glass_axis(&straight, Dir::Up), Some(Axis::X));
    }

    #[test]
    fn the_shape_is_a_core_and_an_arm_to_each_open_side() {
        let shape = shape(&sides(&[Dir::East, Dir::West]));
        assert_eq!(shape.len(), 3);
        assert!(shape.contains(&[0.25, 0.25, 0.25, 1.0, 0.75, 0.75]));
        assert!(shape.contains(&[0.0, 0.25, 0.25, 0.75, 0.75, 0.75]));
    }
}
