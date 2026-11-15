//! The Smart Fluid Pipe's placement and filter, and what a pipe takes the Encased Fluid Pipe
//! from. No game types.
//!
//! Ported from Create `content/fluids/pipes/SmartFluidPipeBlock` (`getStateForPlacement`,
//! `getPipeAxis`), vanilla `FaceAttachedHorizontalDirectionalBlock.getStateForPlacement` and
//! `Direction.orderedByNearest` (`getNearestLookingDirections`),
//! `SmartFluidPipeBlockEntity.SmartPipeBehaviour` and `FilteringBehaviour.test(FluidStack)`
//! through `FilterItemStack.test`, `EncasedPipeBlock`.
//!
//! Known differences from Create: a placement hook can't read the neighbours, so
//! [`adjust`] is applied once the block is placed.

use crate::{Axis, Dir};

/// `AttachFace`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Floor,
    Wall,
    Ceiling,
}

impl Face {
    pub fn name(self) -> &'static str {
        match self {
            Face::Floor => "floor",
            Face::Wall => "wall",
            Face::Ceiling => "ceiling",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Face::Floor, Face::Wall, Face::Ceiling].into_iter().find(|f| f.name() == name)
    }
}

/// `Direction.orderedByNearest`: all six directions, the one the entity looks along first.
/// `yaw` and `pitch` are in degrees.
pub fn nearest_looking(yaw: f32, pitch: f32) -> [Dir; 6] {
    let f = pitch.to_radians();
    let f1 = (-yaw).to_radians();
    let (f2, f3, f4, f5) = (f.sin(), f.cos(), f1.sin(), f1.cos());
    let (flag, flag1, flag2) = (f4 > 0.0, f2 < 0.0, f5 > 0.0);
    let f6 = if flag { f4 } else { -f4 };
    let f7 = if flag1 { -f2 } else { f2 };
    let f8 = if flag2 { f5 } else { -f5 };
    let (f9, f10) = (f6 * f3, f8 * f3);
    let direction = if flag { Dir::East } else { Dir::West };
    let direction1 = if flag1 { Dir::Up } else { Dir::Down };
    let direction2 = if flag2 { Dir::South } else { Dir::North };
    let make = |a: Dir, b: Dir, c: Dir| [a, b, c, c.opposite(), b.opposite(), a.opposite()];
    if f6 > f8 {
        if f7 > f9 {
            make(direction1, direction, direction2)
        } else if f10 > f7 {
            make(direction, direction2, direction1)
        } else {
            make(direction, direction1, direction2)
        }
    } else if f7 > f10 {
        make(direction1, direction2, direction)
    } else if f6 > f7 {
        make(direction2, direction, direction1)
    } else {
        make(direction2, direction1, direction)
    }
}

/// `Direction.fromYRot`: the horizontal direction a player turned `yaw` degrees looks.
pub fn horizontal_direction(yaw: f32) -> Dir {
    match (((yaw / 90.0 + 0.5).floor() as i32) & 3) as u8 {
        0 => Dir::South,
        1 => Dir::West,
        2 => Dir::North,
        _ => Dir::East,
    }
}

/// `FaceAttachedHorizontalDirectionalBlock.getStateForPlacement`: the first of the directions the
/// player looks along that the block can survive on (always the first: `canSurvive` is true).
pub fn placement(order: &[Dir; 6], horizontal: Dir) -> (Face, Dir) {
    let look = order[0];
    if look.axis() == Axis::Y {
        (if look == Dir::Up { Face::Ceiling } else { Face::Floor }, horizontal)
    } else {
        (Face::Wall, look.opposite())
    }
}

/// `SmartFluidPipeBlock.getStateForPlacement`'s search: the axis every neighbour it would
/// connect to (`prefersConnectionTo`) lies on, or `None` when there are none or they differ.
pub fn preferred_axis(prefers: impl Fn(Dir) -> bool) -> Option<Axis> {
    let mut found: Option<Axis> = None;
    for side in Dir::ALL {
        if !prefers(side) {
            continue;
        }
        if found.is_some_and(|axis| axis != side.axis()) {
            return None;
        }
        found = Some(side.axis());
    }
    found
}

/// What `SmartFluidPipeBlock.getStateForPlacement` does with the preferred axis after the
/// vanilla placement: lines the pipe up with its neighbours.
pub fn adjust(face: Face, facing: Dir, preferred: Option<Axis>, order: &[Dir; 6]) -> (Face, Dir) {
    let (mut face, mut facing) = (face, facing);
    match preferred {
        Some(Axis::Y) => {
            face = Face::Wall;
            facing = facing.opposite();
        }
        Some(axis) => {
            if face == Face::Wall {
                face = Face::Floor;
            }
            for direction in order {
                if direction.axis() == axis {
                    facing = direction.opposite();
                }
            }
        }
        None => {}
    }
    (face, facing)
}

/// What a Smart Fluid Pipe's filter makes of the fluid trying to pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    /// No filter item (`isActive` false): everything passes.
    Off,
    /// An item that holds no fluid (`resolveFluid` leaves the filter's fluid empty): nothing passes.
    NoFluid,
    /// An item that holds this fluid.
    Fluid(String),
}

/// `SmartPipeBehaviour.canPullFluidFrom` for a pipe with an open pull: `fluid.isEmpty() ||
/// filter.test(fluid)`.
pub fn passes(filter: &Filter, fluid: Option<&str>) -> bool {
    let Some(fluid) = fluid else { return true };
    match filter {
        Filter::Off => true,
        Filter::NoFluid => false,
        Filter::Fluid(held) => held == fluid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looking_south_level_gives_the_vanilla_order() {
        assert_eq!(
            nearest_looking(0.0, 0.0),
            [Dir::South, Dir::Down, Dir::West, Dir::East, Dir::Up, Dir::North]
        );
        // Straight down.
        assert_eq!(nearest_looking(0.0, 90.0)[0], Dir::Down);
        assert_eq!(nearest_looking(0.0, -90.0)[0], Dir::Up);
        // Yaw 90 is west, -90 east, 180 north.
        assert_eq!(nearest_looking(90.0, 0.0)[0], Dir::West);
        assert_eq!(nearest_looking(-90.0, 0.0)[0], Dir::East);
        assert_eq!(nearest_looking(180.0, 0.0)[0], Dir::North);
    }

    #[test]
    fn horizontal_directions_follow_the_yaw() {
        assert_eq!(horizontal_direction(0.0), Dir::South);
        assert_eq!(horizontal_direction(90.0), Dir::West);
        assert_eq!(horizontal_direction(180.0), Dir::North);
        assert_eq!(horizontal_direction(-90.0), Dir::East);
    }

    #[test]
    fn a_pipe_placed_looking_down_stands_on_the_floor_facing_the_player() {
        let order = nearest_looking(0.0, 80.0);
        assert_eq!(placement(&order, Dir::South), (Face::Floor, Dir::South));
        let order = nearest_looking(0.0, -80.0);
        assert_eq!(placement(&order, Dir::South), (Face::Ceiling, Dir::South));
        // Looking at a wall: on the wall, facing back at the player.
        let order = nearest_looking(0.0, 0.0);
        assert_eq!(placement(&order, Dir::South), (Face::Wall, Dir::North));
    }

    #[test]
    fn neighbours_on_one_axis_line_the_pipe_up() {
        let prefers = |d: Dir| d.axis() == Axis::X;
        assert_eq!(preferred_axis(prefers), Some(Axis::X));
        assert_eq!(preferred_axis(|d| d == Dir::East || d == Dir::North), None);
        assert_eq!(preferred_axis(|_| false), None);
        // A vertical neighbour makes it a wall pipe turned round.
        assert_eq!(
            adjust(Face::Floor, Dir::South, Some(Axis::Y), &nearest_looking(0.0, 60.0)),
            (Face::Wall, Dir::North)
        );
        // A horizontal one lies on the floor along the axis, facing as the player looks along it.
        let order = nearest_looking(-90.0, 0.0);
        assert_eq!(adjust(Face::Wall, Dir::West, Some(Axis::X), &order), (Face::Floor, Dir::East));
        assert_eq!(adjust(Face::Floor, Dir::West, None, &order), (Face::Floor, Dir::West));
    }

    #[test]
    fn the_filter_lets_through_its_own_fluid_only() {
        assert!(passes(&Filter::Off, Some("minecraft:water")));
        assert!(passes(&Filter::Fluid("minecraft:lava".into()), None));
        assert!(!passes(&Filter::Fluid("minecraft:lava".into()), Some("minecraft:water")));
        assert!(passes(&Filter::Fluid("minecraft:water".into()), Some("minecraft:water")));
        assert!(!passes(&Filter::NoFluid, Some("minecraft:water")));
    }
}
