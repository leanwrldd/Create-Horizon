//! What the Smart Observer and the Threshold Switch share (`DirectedDirectionalBlock`): a block
//! that looks at the block it is attached to, from a floor, a ceiling or a wall, and gives no
//! signal back into it. No game types.
//!
//! Ported from Create `content/redstone/DirectedDirectionalBlock`, `SmartObserverBlock` and
//! `ThresholdSwitchBlock` (`getStateForPlacement`) and Minecraft's `Direction.orderedByNearest`
//! as `BlockPlaceContext.getNearestLookingDirections` uses it.
//!
//! Known differences from Create: none in these rules.

use crate::{Axis, Dir};

/// `AttachFace`, as the `target` property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Floor,
    Wall,
    Ceiling,
}

impl Target {
    pub const fn name(self) -> &'static str {
        match self {
            Target::Floor => "floor",
            Target::Wall => "wall",
            Target::Ceiling => "ceiling",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "floor" => Some(Target::Floor),
            "wall" => Some(Target::Wall),
            "ceiling" => Some(Target::Ceiling),
            _ => None,
        }
    }
}

/// `DirectedDirectionalBlock.getTargetDirection`: the side of the block it looks at.
pub fn target_direction(target: Target, facing: Dir) -> Dir {
    match target {
        Target::Ceiling => Dir::Up,
        Target::Floor => Dir::Down,
        Target::Wall => facing,
    }
}

/// `Direction.orderedByNearest(entity)` (`BlockPlaceContext.getNearestLookingDirections`): the
/// six directions, the one the look points along most first and its opposite last.
pub fn ordered_by_nearest(yaw: f32, pitch: f32) -> [Dir; 6] {
    let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
    // The look vector: `Entity.calculateViewVector`.
    let look = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
    let mut axes = [
        (look[0].abs(), if look[0] > 0.0 { Dir::East } else { Dir::West }),
        (look[1].abs(), if look[1] > 0.0 { Dir::Up } else { Dir::Down }),
        (look[2].abs(), if look[2] > 0.0 { Dir::South } else { Dir::North }),
    ];
    // Largest first; ties keep the order x, y, z as the original's comparisons do.
    axes.sort_by(|a, b| b.0.total_cmp(&a.0));
    [
        axes[0].1,
        axes[1].1,
        axes[2].1,
        axes[2].1.opposite(),
        axes[1].1.opposite(),
        axes[0].1.opposite(),
    ]
}

/// The player's horizontal facing (`BlockPlaceContext.getHorizontalDirection`).
pub fn horizontal(yaw: f32) -> Dir {
    crate::looking(yaw, 0.0)
}

/// `getStateForPlacement` of the Smart Observer and the Threshold Switch: the `target` and
/// `facing` for a block put down by a player whose look orders the directions `nearest`, facing
/// horizontally `horizontal`. `preferred` is the first direction of `nearest` that has something
/// to look at; without one the block looks the way the player does when sneaking and back at
/// them otherwise. A direction up or down becomes a floor or a ceiling.
pub fn placement(preferred: Option<Dir>, nearest: Dir, horizontal: Dir, sneaking: bool) -> (Target, Dir) {
    let direction = preferred.unwrap_or(if sneaking { nearest } else { nearest.opposite() });
    if direction.axis() == Axis::Y {
        let target = if direction == Dir::Up { Target::Ceiling } else { Target::Floor };
        (target, horizontal)
    } else {
        (Target::Wall, direction)
    }
}

/// `DirectedDirectionalBlock.getRotatedBlockState`: the block after a Wrench click on the face
/// `clicked`. A top or bottom face turns its facing; a side face turns the direction the block
/// looks in about that face's axis.
pub fn rotated(target: Target, facing: Dir, clicked: Dir) -> (Target, Dir) {
    if clicked.axis() == Axis::Y {
        // `IWrenchable.getRotatedBlockState`: `HORIZONTAL_FACING` turned clockwise about y.
        return (target, crate::rules::drill::clockwise(facing, Axis::Y));
    }
    let looking = target_direction(target, facing);
    let mut next = crate::rules::drill::clockwise(looking, clicked.axis());
    if clicked.sign() < 0 {
        next = next.opposite();
    }
    if next.axis() == Axis::Y {
        (if next == Dir::Up { Target::Ceiling } else { Target::Floor }, facing)
    } else {
        (Target::Wall, next)
    }
}

/// `SmartObserverBlock.getSignal` and `ThresholdSwitchBlock.getSignal`: 15 on every side but the
/// one that touches the block looked at. `face` is the side of the block that touches the one
/// reading the signal.
pub fn signal_towards(face: Dir, target_dir: Dir, lit: bool) -> i32 {
    if lit && face != target_dir { 15 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_target_is_the_facing_on_a_wall_and_up_or_down_otherwise() {
        assert_eq!(target_direction(Target::Wall, Dir::East), Dir::East);
        assert_eq!(target_direction(Target::Ceiling, Dir::East), Dir::Up);
        assert_eq!(target_direction(Target::Floor, Dir::East), Dir::Down);
    }

    #[test]
    fn nearest_directions_start_along_the_look_and_end_against_it() {
        let south = ordered_by_nearest(0.0, 0.0);
        assert_eq!(south[0], Dir::South);
        assert_eq!(south[5], Dir::North);
        // Looking steeply down: down first, then the horizontal one.
        let down = ordered_by_nearest(0.0, 80.0);
        assert_eq!((down[0], down[1], down[5]), (Dir::Down, Dir::South, Dir::Up));
        // Looking west: west, then (a hair of) the rest.
        assert_eq!(ordered_by_nearest(90.0, 0.0)[0], Dir::West);
        assert_eq!(ordered_by_nearest(-90.0, 0.0)[0], Dir::East);
        let all = ordered_by_nearest(30.0, 20.0);
        for dir in Dir::ALL {
            assert_eq!(all.iter().filter(|d| **d == dir).count(), 1);
        }
    }

    #[test]
    fn it_faces_the_block_it_finds_and_else_back_at_the_player() {
        // Something on the north side.
        assert_eq!(placement(Some(Dir::North), Dir::South, Dir::South, false), (Target::Wall, Dir::North));
        // Nothing: looking south, the block looks back north; sneaking it looks the way they look.
        assert_eq!(placement(None, Dir::South, Dir::South, false), (Target::Wall, Dir::North));
        assert_eq!(placement(None, Dir::South, Dir::South, true), (Target::Wall, Dir::South));
        // Above: a ceiling target, facing the way the player faces.
        assert_eq!(placement(Some(Dir::Up), Dir::South, Dir::East, false), (Target::Ceiling, Dir::East));
        // The player looking down at the floor, nothing found: it looks back up, at a ceiling.
        assert_eq!(placement(None, Dir::Down, Dir::West, false), (Target::Ceiling, Dir::West));
        assert_eq!(placement(None, Dir::Down, Dir::West, true), (Target::Floor, Dir::West));
    }

    #[test]
    fn the_wrench_turns_the_block_about_the_clicked_face() {
        // On a top or bottom face it turns the facing.
        assert_eq!(rotated(Target::Wall, Dir::North, Dir::Up), (Target::Wall, Dir::East));
        assert_eq!(rotated(Target::Floor, Dir::North, Dir::Up), (Target::Floor, Dir::East));
        // On an east face (a positive x axis): looking north goes up; looking up goes south.
        assert_eq!(rotated(Target::Wall, Dir::North, Dir::East), (Target::Ceiling, Dir::North));
        assert_eq!(rotated(Target::Ceiling, Dir::North, Dir::East), (Target::Wall, Dir::South));
        // On the west face (a negative one) it turns the other way.
        assert_eq!(rotated(Target::Wall, Dir::North, Dir::West), (Target::Floor, Dir::North));
        // Along the click's own axis a wall block just looks the same way.
        assert_eq!(rotated(Target::Wall, Dir::East, Dir::East), (Target::Wall, Dir::East));
    }

    #[test]
    fn a_lit_block_gives_power_on_every_side_but_the_one_it_looks_at() {
        assert_eq!(signal_towards(Dir::North, Dir::East, true), 15);
        assert_eq!(signal_towards(Dir::East, Dir::East, true), 0);
        assert_eq!(signal_towards(Dir::North, Dir::East, false), 0);
    }
}
