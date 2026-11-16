//! Which sides a Fluid Pipe opens: its six `down` .. `east` states and how they follow its
//! neighbours. No game types.
//!
//! Ported from Create `content/fluids/pipes/FluidPipeBlock` (`updateBlockState`, `isOpenAt`,
//! `shouldDrawCasing`), `FluidPropagator.getStraightPipeAxis` and `GlassFluidPipeBlock` /
//! `AxisPipeBlock`.
//!
//! Known differences from Create: no waterlogging, and a pipe is never rotated or mirrored by a
//! structure; the glue refreshes the sides from its neighbours every few ticks instead of on each
//! neighbour's update. The arms, rims and casing a pipe draws are `fluid_pipe_attachments`; brackets
//! are `bracket`.

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

/// `FluidPipeBlock.updateBlockState`'s first lines: a pipe with a bracket on it returns its state
/// as it is, so its sides stop changing while the bracket stays. Otherwise [`update_sides`].
pub fn refreshed(
    bracketed: bool,
    previous: &Sides,
    preferred: Dir,
    ignore: Option<Dir>,
    can_connect: impl Fn(Dir) -> bool,
) -> Sides {
    if bracketed {
        return *previous;
    }
    update_sides(previous, preferred, ignore, can_connect)
}

/// `FluidPipeBlock.canConnectTo`, the `isPipe(neighbour)` branch: a pipe joins a neighbouring
/// pipe, unless the neighbour carries a bracket and `towards` (the way from the pipe to the
/// neighbour) is not along the neighbour's straight run (`getStraightPipeAxis`).
pub fn joins_pipe(neighbour_bracketed: bool, neighbour_straight: Option<Axis>, towards: Dir) -> bool {
    !neighbour_bracketed || neighbour_straight == Some(towards.axis())
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

/// `DirectionalAxisKineticBlock.getRotationAxis`, which the Fluid Valve uses: the shaft's axis
/// from the block's `facing` and `axis_along_first`.
pub fn valve_rotation_axis(facing: Dir, along_first: bool) -> Axis {
    match facing.axis() {
        Axis::X => if along_first { Axis::Y } else { Axis::Z },
        Axis::Y => if along_first { Axis::X } else { Axis::Z },
        Axis::Z => if along_first { Axis::X } else { Axis::Y },
    }
}

/// `FluidValveBlock.getPipeAxis`: the axis the water runs along, the one of the two that are
/// not `facing`'s which isn't the shaft's.
pub fn valve_pipe_axis(facing: Dir, along_first: bool) -> Axis {
    // Create's local `alongFirst` is the property inverted; it skips one axis when it is false.
    let mut skip = along_first;
    for axis in Axis::ALL {
        if axis == facing.axis() {
            continue;
        }
        if !skip {
            return axis;
        }
        skip = false;
    }
    unreachable!("a facing leaves two axes")
}

/// `SmartFluidPipeBlock.getPipeAxis`: Y on a wall, else the axis of its horizontal `facing`.
pub fn smart_pipe_axis(face: &str, facing: Dir) -> Axis {
    if face == "wall" { Axis::Y } else { facing.axis() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bracketed_pipe_keeps_its_sides_and_only_joins_along_its_axis() {
        // A straight pipe along y with a bracket: a pipe appearing to its east changes nothing.
        let straight = sides(&[Dir::Up, Dir::Down]);
        assert_eq!(refreshed(true, &straight, Dir::North, None, |d| d == Dir::East), straight);
        assert_ne!(refreshed(false, &straight, Dir::North, None, |d| d == Dir::East), straight);
        // The bracketed neighbour is joined along its axis, not across it.
        assert!(joins_pipe(true, Some(Axis::Y), Dir::Up));
        assert!(joins_pipe(true, Some(Axis::Y), Dir::Down));
        assert!(!joins_pipe(true, Some(Axis::Y), Dir::East));
        assert!(!joins_pipe(true, None, Dir::East));
        assert!(joins_pipe(false, None, Dir::East));
    }

    #[test]
    fn a_valve_runs_along_the_axis_its_shaft_does_not() {
        for facing in Dir::ALL {
            for along_first in [false, true] {
                let shaft = valve_rotation_axis(facing, along_first);
                let pipe = valve_pipe_axis(facing, along_first);
                assert!(shaft != facing.axis() && pipe != facing.axis() && shaft != pipe);
            }
        }
        // `AXIS_ALONG_FIRST_COORDINATE` with the shaft on X, facing Y: the pipe is Z.
        assert_eq!(valve_rotation_axis(Dir::Up, true), Axis::X);
        assert_eq!(valve_pipe_axis(Dir::Up, true), Axis::Z);
        assert_eq!(valve_pipe_axis(Dir::Up, false), Axis::X);
    }

    #[test]
    fn a_smart_pipe_on_a_wall_is_vertical() {
        assert_eq!(smart_pipe_axis("wall", Dir::East), Axis::Y);
        assert_eq!(smart_pipe_axis("floor", Dir::East), Axis::X);
        assert_eq!(smart_pipe_axis("ceiling", Dir::North), Axis::Z);
    }

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
